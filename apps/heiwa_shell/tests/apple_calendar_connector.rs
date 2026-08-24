//! Mac-first L3 connector acceptance against a hermetic `osascript` fixture.
//!
//! The real binary owns staging, approval, state, receipts, and journal replay.
//! Only Calendar.app itself is replaced so CI never touches a user's calendar.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

fn fixture_osascript(root: &Path) -> PathBuf {
    let path = root.join("fixture-osascript");
    fs::write(
        &path,
        r#"#!/bin/sh
set -eu
mode="$5"
printf '%s\n' "$mode" >> "$HEIWA_APPLE_CALENDAR_FIXTURE_LOG"
case "$mode" in
  list)
    printf '%s\n' '[{"name":"Calendar","writable":true},{"name":"Birthdays","writable":false}]'
    ;;
  create)
    if [ -e "$HEIWA_APPLE_CALENDAR_FIXTURE_EVENT" ]; then
      created=false
    else
      : > "$HEIWA_APPLE_CALENDAR_FIXTURE_EVENT"
      created=true
    fi
    printf '%s\n' "{\"calendar\":\"Calendar\",\"external_id\":\"fixture-event-123\",\"marker\":\"heiwa://calendar/holds/fixture\",\"title\":\"call mom\",\"start\":\"2026-06-19T15:00:00-07:00\",\"end\":\"2026-06-19T15:30:00-07:00\",\"created\":$created}"
    ;;
  *)
    printf '%s\n' "unknown fixture mode: $mode" >&2
    exit 2
    ;;
esac
"#,
    )
    .expect("write osascript fixture");
    let mut permissions = fs::metadata(&path).expect("fixture metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).expect("make fixture executable");
    path
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn available_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("ephemeral address")
        .port()
}

/// Wait until the runtime can actually serve a request.
///
/// A successful `TcpStream::connect` is not readiness. It only proves the
/// listening socket is bound and the kernel accepted the connection into the
/// backlog; the server can still reset it before handling anything, which
/// surfaces as `ConnectionReset` at the *read* of the first real request
/// rather than at the connect. Under load that race is reliable enough to fail
/// CI, so readiness here means one complete request/response round trip.
fn wait_for_runtime(port: u16) {
    // A healthy runtime serves in well under a second, so a generous ceiling
    // costs nothing in the normal case and buys the headroom a loaded CI
    // runner needs to start a fresh process.
    const ATTEMPTS: u32 = 600;
    const INTERVAL: Duration = Duration::from_millis(50);

    for _ in 0..ATTEMPTS {
        if serves_a_request(port) {
            return;
        }
        thread::sleep(INTERVAL);
    }
    panic!(
        "temporary Heiwa runtime did not serve a request on port {port} within {:?}",
        INTERVAL * ATTEMPTS
    );
}

/// One full round trip against a read-only route, or `false`.
///
/// Every failure mode is a retry rather than a panic: while the runtime is
/// still starting, refusing, resetting, and answering partially are all
/// expected.
fn serves_a_request(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    // Bound the wait so a wedged server fails the loop instead of hanging it.
    if stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .is_err()
    {
        return false;
    }
    if write!(
        stream,
        "GET /api/v1/calendar/resources HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .is_err()
    {
        return false;
    }
    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return false;
    }
    // A complete response, not one the server abandoned part-way.
    response.starts_with("HTTP/1.1") && response.contains("\r\n\r\n")
}

fn post_json(port: u16, target: &str, body: &serde_json::Value) -> String {
    let body = body.to_string();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect runtime");
    write!(
        stream,
        "POST {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer apple-connector-test-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn get(port: u16, target: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect runtime");
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn response_json(response: &str) -> serde_json::Value {
    let body = response.split("\r\n\r\n").nth(1).expect("response body");
    serde_json::from_str(body).expect("response JSON")
}

struct Fixture {
    _root: tempfile::TempDir,
    home: PathBuf,
    evidence: PathBuf,
    bridge: PathBuf,
    log: PathBuf,
    event_state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temp fixture root");
        let home = root.path().join("home");
        let evidence = root.path().join("evidence");
        let log = root.path().join("bridge.log");
        let event_state = root.path().join("event-created");
        fs::create_dir_all(&home).expect("create temp home");
        let bridge = fixture_osascript(root.path());
        Self {
            _root: root,
            home,
            evidence,
            bridge,
            log,
            event_state,
        }
    }

    fn heiwa(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_heiwa"));
        command
            .env("HOME", &self.home)
            .env("HEIWA_EVIDENCE_DIR", &self.evidence)
            .env("HEIWA_APPLE_CALENDAR_OSASCRIPT", &self.bridge)
            .env("HEIWA_APPLE_CALENDAR_FIXTURE_LOG", &self.log)
            .env("HEIWA_APPLE_CALENDAR_FIXTURE_EVENT", &self.event_state)
            .env_remove("HEIWA_HOME")
            .env_remove("HEIWA_STATE_DIR");
        command
    }

    fn establish_local_identity(&self) {
        let root = self.home.join(".heiwa");
        fs::create_dir_all(&root).expect("create Heiwa root");
        fs::write(
            root.join("local-identity.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 1,
                "installation_id": "install-apple-connector-test",
                "display_name": "Ada",
                "created_at": "2026-08-21T00:00:00Z"
            }))
            .expect("identity JSON"),
        )
        .expect("write local identity");
    }

    fn connect_apple_calendar_cli(&self) {
        self.establish_local_identity();
        let output = self
            .heiwa()
            .args(["connect", "apple-calendar", "--authorize"])
            .output()
            .expect("connect Apple Calendar");
        assert!(
            output.status.success(),
            "connect stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn stranger_app_keeps_apple_calendar_private_until_this_profile_connects_it() {
    let fixture = Fixture::new();
    fixture.establish_local_identity();
    let port = available_port();
    let child = fixture
        .heiwa()
        .env("HEIWA_MACHINE_AUTH_TOKEN", "apple-connector-test-token")
        .args(["app", "start", "--port", &port.to_string(), "--no-open"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start temporary runtime");
    let _child = ChildGuard(child);
    wait_for_runtime(port);

    let before = response_json(&get(port, "/api/v1/calendar/resources"));
    assert_eq!(before["data"]["status"], "disconnected");
    assert_eq!(before["data"]["calendars"], serde_json::json!([]));
    assert!(
        !fixture.log.exists(),
        "a fresh profile must not probe or reveal Calendar.app resources"
    );

    let connected = post_json(
        port,
        "/api/v1/connectors/apple_calendar/connect",
        &serde_json::json!({}),
    );
    assert!(
        connected.starts_with("HTTP/1.1 200 OK\r\n"),
        "unexpected connect response: {connected}"
    );
    let connected = response_json(&connected);
    assert_eq!(connected["data"]["status"], "connected");
    assert_eq!(connected["data"]["resource_count"], 2);

    let after = response_json(&get(port, "/api/v1/calendar/resources"));
    assert_eq!(after["data"]["status"], "ready");
    assert_eq!(after["data"]["calendars"][0]["name"], "Calendar");
    assert!(
        fixture
            .home
            .join(".heiwa/state/connectors/apple_calendar.json")
            .is_file(),
        "the explicit profile enrollment must be durable"
    );
    let enrollment_path = fixture
        .home
        .join(".heiwa/state/connectors/apple_calendar.json");
    let enrollment: serde_json::Value =
        serde_json::from_slice(&fs::read(&enrollment_path).expect("read connector enrollment"))
            .expect("connector enrollment JSON");
    assert_eq!(
        enrollment["schema_version"],
        "heiwa_connector_enrollment_v1"
    );
    assert_eq!(
        enrollment["installation_id"],
        "install-apple-connector-test"
    );
    assert!(
        enrollment["device_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "connector enrollment must be node-bound"
    );
    assert_eq!(
        fs::metadata(&enrollment_path)
            .expect("enrollment metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn authenticated_app_approval_endpoint_executes_the_existing_connector_effect() {
    let fixture = Fixture::new();
    fixture.establish_local_identity();
    let port = available_port();
    let child = fixture
        .heiwa()
        .env("HEIWA_MACHINE_AUTH_TOKEN", "apple-connector-test-token")
        .args(["app", "start", "--port", &port.to_string(), "--no-open"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start temporary runtime");
    let _child = ChildGuard(child);
    wait_for_runtime(port);

    let connected = post_json(
        port,
        "/api/v1/connectors/apple_calendar/connect",
        &serde_json::json!({}),
    );
    assert!(connected.starts_with("HTTP/1.1 200 OK\r\n"));

    let staged = response_json(&post_json(
        port,
        "/api/v1/calendar/holds",
        &serde_json::json!({
            "title": "call mom",
            "date": "2026-06-19",
            "start": "15:00",
            "end": "15:30",
            "kind": "focus",
            "promotion": {
                "connector": "apple_calendar",
                "calendar": "Calendar"
            }
        }),
    ));
    let request_id = staged["data"]["approval_request"]["request_id"]
        .as_str()
        .expect("approval request id");
    assert!(!fixture.event_state.exists());

    let decided = post_json(
        port,
        &format!("/api/v1/approvals/{request_id}/approve"),
        &serde_json::json!({"note":"approved in Heiwa.app"}),
    );
    assert!(
        decided.starts_with("HTTP/1.1 200 OK\r\n"),
        "unexpected approval response: {decided}"
    );
    let decided = response_json(&decided);
    assert_eq!(decided["data"]["decision"]["outcome"], "approved");
    assert_eq!(
        decided["data"]["decision"]["applied_effects"][0]["kind"],
        "apple_calendar_create"
    );
    assert!(fixture.event_state.exists());

    let pending = response_json(&get(port, "/api/v1/approvals"));
    assert_eq!(
        pending["data"]["approvals"],
        serde_json::json!([]),
        "a decided request must leave the app's pending approval list"
    );
}

#[test]
fn listing_resources_before_enrollment_is_empty_and_does_not_probe_calendar_app() {
    let fixture = Fixture::new();
    let output = fixture
        .heiwa()
        .args(["calendar", "calendars", "--source", "apple", "--json"])
        .output()
        .expect("calendar resource listing runs");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("calendar resources JSON");
    assert_eq!(payload["source"], "apple_calendar");
    assert_eq!(payload["status"], "disconnected");
    assert_eq!(payload["calendars"], serde_json::json!([]));
    assert_eq!(payload["revoke"]["owner"], "macOS");
    assert!(
        !fixture.log.exists(),
        "resource listing must not probe Calendar.app before enrollment"
    );
    assert!(
        !fixture.home.join(".heiwa").exists(),
        "resource discovery must not create Heiwa state"
    );

    let sync = fixture
        .heiwa()
        .args([
            "calendar", "sync", "--source", "apple", "--limit", "5", "--json",
        ])
        .output()
        .expect("calendar sync reports disconnected source");
    assert!(sync.status.success());
    let sync: serde_json::Value = serde_json::from_slice(&sync.stdout).expect("calendar sync JSON");
    assert_eq!(sync["sources"][0]["status"], "disconnected");
    assert!(
        !fixture.log.exists(),
        "calendar sync must not probe Calendar.app before enrollment"
    );
}

#[test]
fn reconnect_refuses_to_overwrite_a_future_enrollment_schema() {
    let fixture = Fixture::new();
    fixture.establish_local_identity();
    let enrollment_path = fixture
        .home
        .join(".heiwa/state/connectors/apple_calendar.json");
    fs::create_dir_all(enrollment_path.parent().expect("connector directory"))
        .expect("create connector directory");
    let future = serde_json::json!({
        "schema_version": "heiwa_connector_enrollment_v2",
        "connector": "apple_calendar",
        "installation_id": "future-installation",
        "device_id": "future-device",
        "connected_at": "2026-08-21T00:00:00Z",
        "scopes": ["calendar.read"]
    });
    fs::write(
        &enrollment_path,
        serde_json::to_vec_pretty(&future).expect("future enrollment JSON"),
    )
    .expect("write future enrollment");

    let output = fixture
        .heiwa()
        .args(["connect", "apple-calendar", "--authorize"])
        .output()
        .expect("reconnect runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("upgrade Heiwa"));
    let unchanged: serde_json::Value = serde_json::from_slice(
        &fs::read(&enrollment_path).expect("future enrollment remains present"),
    )
    .expect("future enrollment remains JSON");
    assert_eq!(unchanged, future);
    assert!(
        !fixture.log.exists(),
        "future-schema refusal must happen before probing Calendar.app"
    );
}

#[test]
fn disconnect_after_staging_blocks_the_pending_external_write() {
    let fixture = Fixture::new();
    fixture.connect_apple_calendar_cli();
    let staged = fixture
        .heiwa()
        .args([
            "schedule",
            "call",
            "mom",
            "--at",
            "2026-06-19T15:00",
            "--promote",
            "apple",
            "--calendar",
            "Calendar",
            "--json",
        ])
        .output()
        .expect("schedule staging runs");
    assert!(staged.status.success());
    let staged: serde_json::Value = serde_json::from_slice(&staged.stdout).expect("staging JSON");
    let request_id = staged["approval_request"]["request_id"]
        .as_str()
        .expect("request id");

    let disconnected = fixture
        .heiwa()
        .args(["connect", "apple-calendar", "--disconnect"])
        .output()
        .expect("disconnect runs");
    assert!(disconnected.status.success());

    let approved = fixture
        .heiwa()
        .args(["approvals", "decide", request_id, "--approve"])
        .output()
        .expect("approval runs");

    assert!(!approved.status.success());
    assert!(String::from_utf8_lossy(&approved.stderr).contains("not connected"));
    assert!(!fixture.event_state.exists());
    assert!(
        !fixture
            .home
            .join(".heiwa/state/dispatch/approvals/decisions")
            .join(format!("{request_id}.json"))
            .exists(),
        "a failed external effect must remain pending and undecided"
    );
}

#[test]
fn approval_executes_apple_write_and_replays_connector_receipt() {
    let fixture = Fixture::new();
    fixture.connect_apple_calendar_cli();
    let staged = fixture
        .heiwa()
        .args([
            "schedule",
            "call",
            "mom",
            "--at",
            "2026-06-19T15:00",
            "--promote",
            "apple",
            "--calendar",
            "Calendar",
            "--json",
        ])
        .output()
        .expect("schedule staging runs");
    assert!(
        staged.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&staged.stderr)
    );
    let staged: serde_json::Value = serde_json::from_slice(&staged.stdout).expect("staging JSON");
    let request_id = staged["approval_request"]["request_id"]
        .as_str()
        .expect("request id");
    let hold_id = staged["hold"]["id"].as_str().expect("hold id");
    let work_id = staged["hold"]["work_id"].as_str().expect("work id");

    assert_eq!(staged["approval_request"]["work_id"], work_id);
    assert_eq!(staged["approval_request"]["risk_tier"], "T2");
    assert_eq!(
        staged["approval_request"]["intent"]["promotion"]["connector"],
        "apple_calendar"
    );
    assert_eq!(
        staged["approval_request"]["intent"]["promotion"]["calendar"],
        "Calendar"
    );
    assert_eq!(staged["hold"]["external_promotion"], "approval_required");
    assert_eq!(
        fs::read_to_string(&fixture.log).expect("bridge log after staging"),
        "list\nlist\n",
        "connection and staging may list resources but must not create an event"
    );
    assert!(!fixture.event_state.exists());

    let approved = fixture
        .heiwa()
        .args(["approvals", "decide", request_id, "--approve", "--json"])
        .output()
        .expect("approval runs");
    assert!(
        approved.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&approved.stderr)
    );
    let approved: serde_json::Value =
        serde_json::from_slice(&approved.stdout).expect("approval JSON");
    assert_eq!(
        approved["decision"]["applied_effects"][0]["kind"],
        "apple_calendar_create"
    );
    assert_eq!(
        approved["decision"]["applied_effects"][0]["external_event"]["external_id"],
        "fixture-event-123"
    );
    assert!(fixture.event_state.exists());

    let hold_path = fixture
        .home
        .join(".heiwa/state/calendar/holds")
        .join(format!("{hold_id}.json"));
    let hold: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&hold_path).expect("promoted hold is persisted"))
            .expect("promoted hold JSON");
    assert_eq!(hold["status"], "confirmed");
    assert_eq!(hold["work_id"], work_id);
    assert_eq!(hold["external_promotion"], "promoted");
    assert_eq!(hold["external_event"]["external_id"], "fixture-event-123");

    let receipt_path = fixture
        .home
        .join(".heiwa/state/calendar/receipts")
        .join(format!("rcpt-{hold_id}-apple-create.json"));
    let receipt: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&receipt_path).expect("connector receipt file"))
            .expect("connector receipt JSON");
    assert_eq!(receipt["schema_version"], "heiwa_connector_receipt_v1");
    assert_eq!(receipt["work_id"], work_id);
    assert_eq!(receipt["approval_id"], request_id);
    assert_eq!(receipt["connector"], "apple_calendar");
    assert_eq!(receipt["external_id"], "fixture-event-123");

    let replay = heiwa_evidence::read_stream(&fixture.evidence, "connector_receipts")
        .expect("connector receipt journal replays");
    assert_eq!(replay.skipped_lines, 0);
    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.events[0].record["receipt_id"], receipt["receipt_id"]);
    assert_eq!(replay.events[0].record["work_id"], work_id);

    let retried = fixture
        .heiwa()
        .args(["approvals", "decide", request_id, "--approve", "--json"])
        .output()
        .expect("approval retry runs");
    assert!(
        retried.status.success(),
        "retry stderr: {}",
        String::from_utf8_lossy(&retried.stderr)
    );
    let replay = heiwa_evidence::read_stream(&fixture.evidence, "connector_receipts")
        .expect("connector receipt journal replays after retry");
    assert_eq!(
        replay.events.len(),
        1,
        "stable receipt id must not append duplicate evidence"
    );
    let receipt_after_retry: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&receipt_path).expect("connector receipt survives retry"),
    )
    .expect("connector receipt JSON after retry");
    assert_eq!(
        receipt_after_retry["after"]["created"], true,
        "retry must preserve first-success creation truth"
    );
}

#[test]
fn authenticated_app_hold_endpoint_stages_named_apple_promotion() {
    let fixture = Fixture::new();
    fixture.establish_local_identity();
    let port = available_port();
    let child = fixture
        .heiwa()
        .env("HEIWA_MACHINE_AUTH_TOKEN", "apple-connector-test-token")
        .args(["app", "start", "--port", &port.to_string(), "--no-open"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start temporary runtime");
    let _child = ChildGuard(child);
    wait_for_runtime(port);

    let connected = post_json(
        port,
        "/api/v1/connectors/apple_calendar/connect",
        &serde_json::json!({}),
    );
    assert!(
        connected.starts_with("HTTP/1.1 200 OK\r\n"),
        "unexpected connect response: {connected}"
    );

    let response = post_json(
        port,
        "/api/v1/calendar/holds",
        &serde_json::json!({
            "title": "App-staged focus block",
            "date": "2026-06-19",
            "start": "16:00",
            "end": "16:30",
            "kind": "focus",
            "promotion": {
                "connector": "apple_calendar",
                "calendar": "Calendar"
            }
        }),
    );
    assert!(
        response.starts_with("HTTP/1.1 201 Created\r\n"),
        "unexpected response: {response}"
    );
    let body = response.split("\r\n\r\n").nth(1).expect("response body");
    let payload: serde_json::Value = serde_json::from_str(body).expect("response JSON");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["data"]["hold"]["promotion"]["calendar"], "Calendar");
    assert_eq!(payload["data"]["approval_request"]["risk_tier"], "T2");
    assert_eq!(
        payload["data"]["approval_request"]["work_id"],
        payload["data"]["hold"]["work_id"]
    );
    assert_eq!(
        fs::read_to_string(&fixture.log).expect("bridge log after app staging"),
        "list\nlist\n",
        "connection and app staging may list resources but must not create before approval"
    );
}
