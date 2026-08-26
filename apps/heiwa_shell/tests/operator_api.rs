//! Integration tests for the authenticated operator HTTP contract.

use heiwa_core::auth::{sign_local_request, LocalRequestParts, LocalRequestSignature};
use heiwa_evidence::OperatorJournal;
use heiwa_session::operator::{OperatorSessionService, StartTurnRequest};
use heiwa_work::{work_created_event, WorkId};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const TOKEN: &str = "test-machine-token";

struct TestRuntime {
    child: Child,
    port: u16,
    _home: tempfile::TempDir,
    evidence: tempfile::TempDir,
}

impl TestRuntime {
    fn start(configured_auth: bool) -> Self {
        Self::start_with_provider_path(configured_auth, true)
    }

    fn start_without_providers() -> Self {
        Self::start_with_provider_path(true, false)
    }

    fn start_with_ollama_override(override_endpoint: &str, stored_endpoint: &str) -> Self {
        let port = reserve_port();
        let home = tempfile::tempdir().unwrap();
        let evidence = tempfile::tempdir().unwrap();
        let accounts_dir = home.path().join(".heiwa");
        std::fs::create_dir_all(&accounts_dir).unwrap();
        std::fs::write(
            accounts_dir.join("accounts.json"),
            serde_json::json!({
                "accounts": [{
                    "account_id": "ollama-local",
                    "provider": "ollama",
                    "credential": {
                        "kind": "local_runtime",
                        "endpoint": stored_endpoint,
                    },
                    "rate_group": "local",
                    "status": "disconnected",
                    "models": [],
                }]
            })
            .to_string(),
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_heiwa"));
        command
            .env("HOME", home.path())
            .env_remove("HEIWA_HOME")
            .env("HEIWA_EVIDENCE_DIR", evidence.path())
            .env("HEIWA_OLLAMA_BASE", override_endpoint)
            .env_remove("HEIWA_MACHINE_AUTH_TOKEN")
            .env_remove("HEIWA_AUTH_TOKEN")
            .env_remove("HEIWA_JWT_SIGNING_SECRET")
            .env_remove("HEIWA_AUTH_SECRET")
            .args(["app", "start", "--port", &port.to_string(), "--no-open"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().expect("start test runtime");
        wait_for_port(port);
        Self {
            child,
            port,
            _home: home,
            evidence,
        }
    }

    fn start_with_provider_path(configured_auth: bool, provider_path: bool) -> Self {
        let port = reserve_port();
        let home = tempfile::tempdir().unwrap();
        let evidence = tempfile::tempdir().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_heiwa"));
        command
            .env("HOME", home.path())
            .env_remove("HEIWA_HOME")
            .env("HEIWA_EVIDENCE_DIR", evidence.path())
            .env_remove("HEIWA_MACHINE_AUTH_TOKEN")
            .env_remove("HEIWA_AUTH_TOKEN")
            .env_remove("HEIWA_JWT_SIGNING_SECRET")
            .env_remove("HEIWA_AUTH_SECRET")
            .args(["app", "start", "--port", &port.to_string(), "--no-open"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if configured_auth {
            command.env("HEIWA_MACHINE_AUTH_TOKEN", TOKEN);
        }
        if !provider_path {
            let empty_path = home.path().join("empty-path");
            std::fs::create_dir(&empty_path).unwrap();
            command.env("PATH", empty_path);
        }
        let child = command.spawn().expect("start test runtime");
        wait_for_port(port);
        Self {
            child,
            port,
            _home: home,
            evidence,
        }
    }

    fn external_sessions(&self) -> OperatorSessionService {
        OperatorSessionService::new(
            OperatorJournal::new(self.evidence.path().to_path_buf()).expect("operator journal"),
        )
    }

    fn calendar_hold_count(&self) -> usize {
        std::fs::read_dir(self._home.path().join(".heiwa/state/calendar/holds"))
            .map(|entries| entries.filter_map(Result::ok).count())
            .unwrap_or(0)
    }

    fn request(&self, method: &str, target: &str, token: Option<&str>, body: Value) -> Response {
        request(self.port, method, target, token, &body.to_string())
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(_) => break,
            }
        }
        // A failed/ignored graceful signal must never hang the test process
        // or leak a listener after an assertion failure.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Response {
    status: u16,
    body: Value,
}

struct HandshakeResponse {
    status: u16,
    head: String,
    body: Value,
}

#[test]
fn operator_routes_fail_closed_before_read_or_action() {
    let runtime = TestRuntime::start(true);

    for (method, target, body) in [
        ("GET", "/api/v1/operator/threads", json!(null)),
        (
            "POST",
            "/api/v1/operator/threads/default/turns",
            json!({"client_request_id": "unauthenticated", "prompt": "hi"}),
        ),
        (
            "POST",
            "/api/v1/operator/turns/turn-not-real/cancel",
            json!(null),
        ),
        ("POST", "/api/v1/repl", json!({"prompt": "hi"})),
        ("POST", "/api/v1/repl/stream", json!({"prompt": "hi"})),
        (
            "POST",
            "/api/v1/connectors/apple_calendar/connect",
            json!({}),
        ),
        (
            "POST",
            "/api/v1/connectors/apple_calendar/disconnect",
            json!({}),
        ),
        ("POST", "/api/v1/approvals/req_not_real/approve", json!({})),
        ("POST", "/api/v1/approvals/req_not_real/deny", json!({})),
    ] {
        let response = runtime.request(method, target, None, body);
        assert_eq!(response.status, 401, "{method} {target}: {}", response.body);
        assert_eq!(response.body["error"]["code"], "unauthorized");
    }
}

#[test]
fn agent_dispatch_authenticates_before_execution_and_accepts_valid_bearer() {
    let runtime = TestRuntime::start_without_providers();
    let request = json!({"task": "hi"});

    for token in [None, Some("wrong-token")] {
        let rejected = runtime.request("POST", "/api/v1/agents/dispatch", token, request.clone());
        assert_eq!(rejected.status, 401, "{}", rejected.body);
        assert_eq!(rejected.body["error"]["code"], "unauthorized");
    }
    assert!(
        runtime
            .external_sessions()
            .list_threads(10)
            .unwrap()
            .is_empty(),
        "rejected dispatches must not append operator events"
    );

    let accepted = runtime.request("POST", "/api/v1/agents/dispatch", Some(TOKEN), request);
    assert_eq!(accepted.status, 202, "{}", accepted.body);
    assert_eq!(accepted.body["data"]["status"], "accepted");
    assert_eq!(accepted.body["data"]["provider"], "auto");
    assert_eq!(accepted.body["data"]["model"], "router-selected");
}

#[test]
fn calendar_hold_authenticates_before_mutation_and_accepts_valid_bearer() {
    let runtime = TestRuntime::start_without_providers();
    let request = json!({
        "title": "Authenticated local hold",
        "date": "2026-07-20",
        "kind": "focus"
    });

    for token in [None, Some("wrong-token")] {
        let rejected = runtime.request("POST", "/api/v1/calendar/holds", token, request.clone());
        assert_eq!(rejected.status, 401, "{}", rejected.body);
        assert_eq!(rejected.body["error"]["code"], "unauthorized");
    }
    assert_eq!(
        runtime.calendar_hold_count(),
        0,
        "rejected calendar mutations must not write a hold"
    );

    let accepted = runtime.request("POST", "/api/v1/calendar/holds", Some(TOKEN), request);
    assert_eq!(accepted.status, 201, "{}", accepted.body);
    assert_eq!(accepted.body["data"]["hold"]["status"], "draft");
    assert_eq!(runtime.calendar_hold_count(), 1);
}

#[test]
fn missing_operator_auth_configuration_is_distinct_from_bad_credentials() {
    let runtime = TestRuntime::start(false);

    let missing = runtime.request("GET", "/api/v1/operator/threads", None, json!(null));
    assert_eq!(missing.status, 500);
    assert_eq!(missing.body["error"]["code"], "auth_not_configured");

    let configured = TestRuntime::start(true);
    let bad = configured.request(
        "GET",
        "/api/v1/operator/threads",
        Some("wrong-token"),
        json!(null),
    );
    assert_eq!(bad.status, 401);
    assert_eq!(bad.body["error"]["code"], "unauthorized");
    assert!(!bad.body.to_string().contains(TOKEN));
}

#[test]
fn signed_local_requests_are_port_bound_and_replay_rejected_before_action() {
    let runtime = TestRuntime::start(true);
    let target = "/api/v1/calendar/holds";
    let body = json!({
        "title": "Signed local hold",
        "date": "2026-07-21",
        "kind": "focus"
    })
    .to_string();
    let timestamp = unix_timestamp_now();
    let signed = sign_local_request(
        LocalRequestParts {
            method: "POST",
            port: runtime.port,
            target,
            body: body.as_bytes(),
        },
        timestamp,
        "0123456789abcdef0123456789abcdef",
        TOKEN,
    )
    .unwrap();

    let accepted = signed_request(runtime.port, "POST", target, &body, &signed);
    assert_eq!(accepted.status, 201, "{}", accepted.body);
    let replay = signed_request(runtime.port, "POST", target, &body, &signed);
    assert_eq!(replay.status, 401, "{}", replay.body);
    assert_eq!(replay.body["error"]["code"], "unauthorized");
    assert_eq!(runtime.calendar_hold_count(), 1, "replay reached mutation");

    let wrong_port = if runtime.port == u16::MAX {
        runtime.port - 1
    } else {
        runtime.port + 1
    };
    let relayed = sign_local_request(
        LocalRequestParts {
            method: "GET",
            port: wrong_port,
            target: "/api/v1/operator/threads",
            body: b"",
        },
        timestamp,
        "fedcba9876543210fedcba9876543210",
        TOKEN,
    )
    .unwrap();
    let rejected = signed_request(
        runtime.port,
        "GET",
        "/api/v1/operator/threads",
        "",
        &relayed,
    );
    assert_eq!(rejected.status, 401, "{}", rejected.body);
    assert_eq!(rejected.body["error"]["code"], "unauthorized");
}

#[test]
fn attacker_loopback_host_cannot_relay_authenticated_post_or_websocket() {
    let runtime = TestRuntime::start_without_providers();
    let attacker_port = if runtime.port == u16::MAX {
        runtime.port - 1
    } else {
        runtime.port + 1
    };
    let hostile_host = format!("127.0.0.1:{attacker_port}");
    let post = request_with_host(
        runtime.port,
        &hostile_host,
        "POST",
        "/api/v1/calendar/holds",
        Some(TOKEN),
        &json!({
            "title": "must not stage",
            "date": "2026-07-26",
            "kind": "focus"
        })
        .to_string(),
    );
    assert_eq!(post.status, 401, "{}", post.body);
    assert_eq!(runtime.calendar_hold_count(), 0);

    let ws = websocket_handshake_with_host(
        runtime.port,
        &hostile_host,
        "/ws/v1/operator",
        &format!("Authorization: Bearer {TOKEN}\r\n"),
    );
    assert_eq!(ws.status, 401, "{}", ws.head);
    assert!(!ws.head.contains("101 Switching Protocols"));
}

#[test]
fn operator_websocket_authenticates_before_any_upgrade() {
    let unconfigured = TestRuntime::start(false);
    let missing_config = websocket_handshake(unconfigured.port, None);
    assert_eq!(missing_config.status, 500, "{}", missing_config.head);
    assert!(!missing_config.head.contains("101 Switching Protocols"));
    assert_eq!(missing_config.body["error"]["code"], "auth_not_configured");

    let configured = TestRuntime::start(true);
    for token in [None, Some("wrong-token")] {
        let unauthorized = websocket_handshake(configured.port, token);
        assert_eq!(unauthorized.status, 401, "{}", unauthorized.head);
        assert!(!unauthorized.head.contains("101 Switching Protocols"));
        assert_eq!(unauthorized.body["error"]["code"], "unauthorized");
    }

    let target = "/ws/v1/operator";
    let signed = sign_local_request(
        LocalRequestParts {
            method: "GET",
            port: configured.port,
            target,
            body: b"",
        },
        unix_timestamp_now(),
        "89abcdef0123456789abcdef01234567",
        TOKEN,
    )
    .unwrap();
    let signed_headers = format!(
        "X-Heiwa-Local-Auth-Version: {}\r\nX-Heiwa-Local-Auth-Timestamp: {}\r\nX-Heiwa-Local-Auth-Nonce: {}\r\nX-Heiwa-Local-Auth-Signature: {}\r\n",
        signed.version, signed.timestamp, signed.nonce, signed.signature,
    );
    let signed_only = websocket_handshake_with_headers(configured.port, target, &signed_headers);
    assert_eq!(signed_only.status, 101, "{}", signed_only.head);
    assert!(signed_only.head.contains("101 Switching Protocols"));
    let signed_replay = websocket_handshake_with_headers(configured.port, target, &signed_headers);
    assert_eq!(signed_replay.status, 401, "{}", signed_replay.head);
    assert!(!signed_replay.head.contains("101 Switching Protocols"));
    assert_eq!(signed_replay.body["error"]["code"], "unauthorized");

    let authorized = websocket_handshake(configured.port, Some(TOKEN));
    assert_eq!(authorized.status, 101, "{}", authorized.head);
}

#[test]
fn operator_websocket_contract_replays_resumes_and_tails_shared_journal() {
    let runtime = TestRuntime::start(true);
    let external = runtime.external_sessions();
    external.ensure_thread("contract-thread").unwrap();

    let (mut first, status) = connect_operator_websocket(
        runtime.port,
        "/ws/v1/operator?thread_id=contract-thread",
        TOKEN,
    );
    assert_eq!(status, 101);
    let created = read_websocket_json(&mut first);
    assert_eq!(created["type"], "event");
    assert_eq!(created["event"]["event_type"], "thread_created");
    let created_cursor = created["cursor"].as_str().unwrap().to_string();
    assert_eq!(read_websocket_json(&mut first)["type"], "caught_up");

    external
        .start_turn(
            "contract-thread",
            StartTurnRequest::auto("contract-live-1", "shared journal append"),
        )
        .unwrap();
    for expected in ["turn_started", "user_message"] {
        let frame = read_websocket_json(&mut first);
        assert_eq!(frame["type"], "event");
        assert_eq!(frame["event"]["event_type"], expected);
    }
    drop(first);

    let (mut resumed, status) = connect_operator_websocket(
        runtime.port,
        &format!("/ws/v1/operator?thread_id=contract-thread&after={created_cursor}"),
        TOKEN,
    );
    assert_eq!(status, 101);
    for expected in ["turn_started", "user_message"] {
        let frame = read_websocket_json(&mut resumed);
        assert_eq!(frame["event"]["event_type"], expected);
    }
    assert_eq!(read_websocket_json(&mut resumed)["type"], "caught_up");
}

#[test]
fn authenticated_operator_routes_share_one_idempotent_runner() {
    let runtime = TestRuntime::start(true);

    let created = runtime.request(
        "POST",
        "/api/v1/operator/threads",
        Some(TOKEN),
        json!({"thread_id": "default"}),
    );
    assert_eq!(created.status, 200, "{}", created.body);
    assert_eq!(created.body["data"]["thread_id"], "default");
    let created_list = runtime.request("GET", "/api/v1/operator/threads", Some(TOKEN), json!(null));
    assert_eq!(created_list.status, 200, "{}", created_list.body);
    assert_eq!(
        created_list.body["data"]["threads"][0]["thread_id"], "default",
        "POST /threads must durably ensure the thread"
    );
    assert_eq!(created_list.body["data"]["threads"][0]["turn_count"], 0);

    let request_body = json!({
        "client_request_id": "api-idempotency-1",
        "prompt": "hi",
        "route_policy": {
            "mode": "auto",
            "preferred_provider": " claude ",
            "allowed_models": ["model-b", "model-a"],
            "minimum_quality_class": 1,
            "maximum_marginal_cost_usd": 0.0,
            "turn_budget_usd": 0.0,
            "privacy": "standard"
        }
    });
    let first = runtime.request(
        "POST",
        "/api/v1/operator/threads/default/turns",
        Some(TOKEN),
        request_body.clone(),
    );
    assert_eq!(first.status, 202, "{}", first.body);
    assert_eq!(first.body["data"]["thread_id"], "default");
    assert_eq!(first.body["data"]["duplicate"], false);
    assert!(
        first.body["data"]["cursor"]
            .as_str()
            .is_some_and(|cursor| !cursor.is_empty()),
        "first submission must expose the durable user-message cursor"
    );
    assert!(first.body["data"]["stream_url"]
        .as_str()
        .unwrap()
        .starts_with("/ws/v1/operator?"));

    let mut normalized_duplicate = request_body;
    normalized_duplicate["route_policy"]["preferred_provider"] = json!("claude");
    normalized_duplicate["route_policy"]["allowed_models"] =
        json!(["model-a", "model-b", "model-a"]);
    let second = runtime.request(
        "POST",
        "/api/v1/operator/threads/default/turns",
        Some(TOKEN),
        normalized_duplicate,
    );
    assert_eq!(second.status, 202, "{}", second.body);
    assert_eq!(
        second.body["data"]["turn_id"],
        first.body["data"]["turn_id"]
    );
    assert_eq!(second.body["data"]["duplicate"], true);
    assert_eq!(second.body["data"]["cursor"], first.body["data"]["cursor"]);
    assert_eq!(
        second.body["data"]["stream_url"],
        first.body["data"]["stream_url"]
    );

    let turn_id = first.body["data"]["turn_id"].as_str().unwrap();
    let cancel = runtime.request(
        "POST",
        &format!("/api/v1/operator/turns/{turn_id}/cancel"),
        Some(TOKEN),
        json!(null),
    );
    assert!(
        matches!(cancel.status, 200 | 202),
        "authenticated cancel response: {}",
        cancel.body
    );
    assert_eq!(cancel.body["data"]["turn_id"], turn_id);
    assert!(cancel.body["data"]["cancel_requested"].is_boolean());

    let listed = runtime.request("GET", "/api/v1/operator/threads", Some(TOKEN), json!(null));
    assert_eq!(listed.status, 200, "{}", listed.body);
    assert_eq!(listed.body["data"]["threads"][0]["thread_id"], "default");

    let thread = runtime.request(
        "GET",
        "/api/v1/operator/threads/default",
        Some(TOKEN),
        json!(null),
    );
    assert_eq!(thread.status, 200, "{}", thread.body);
    assert_eq!(
        thread.body["data"]["thread"]["turns"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let events = runtime.request(
        "GET",
        "/api/v1/operator/threads/default/events?limit=100",
        Some(TOKEN),
        json!(null),
    );
    assert_eq!(events.status, 200, "{}", events.body);
    assert!(events.body["data"]["events"].as_array().unwrap().len() >= 3);
    assert!(events.body["data"]["next_cursor"].is_string());
}

#[test]
fn ollama_models_payload_uses_child_override_before_stored_live_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let override_endpoint = format!("http://{}/", listener.local_addr().unwrap());
    let fixture = thread::spawn(move || {
        // This fixture waits for a full runtime SUBPROCESS to boot, bind a port
        // and issue an HTTP request, so the deadline is generous. It exists to
        // stop a hung test, not to assert a latency budget: the loop returns
        // the instant a connection arrives.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // The LISTENER is non-blocking so the accept loop can poll.
                    // On macOS/BSD and Windows the ACCEPTED stream inherits that
                    // flag; on Linux it does not. Reading it non-blocking
                    // returns WouldBlock (os error 35) before the request has
                    // arrived, which is why this passed on Ubuntu and failed on
                    // macOS and Windows. Put the stream back in blocking mode
                    // and bound the read so a silent peer still cannot hang us.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    let mut request = [0_u8; 1024];
                    let bytes = stream.read(&mut request).unwrap();
                    let request = std::str::from_utf8(&request[..bytes]).unwrap();
                    assert!(request.starts_with("GET /api/tags HTTP/1.1"), "{request}");
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 31\r\nConnection: close\r\n\r\n{\"models\":[{\"name\":\"fixture\"}]}",
                        )
                        .unwrap();
                    return true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("fixture listener failed: {error}"),
            }
        }
    });

    let runtime =
        TestRuntime::start_with_ollama_override(&override_endpoint, "http://127.0.0.1:11434");
    let response = runtime.request("GET", "/api/v1/providers/ollama/models", None, json!(null));

    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(
        response.body["data"]["models"],
        json!([{ "name": "fixture" }])
    );
    assert!(
        fixture.join().unwrap(),
        "override fixture was never contacted"
    );
}

#[test]
fn operator_maps_typed_intake_rejections_without_appending_or_reexecuting() {
    let runtime = TestRuntime::start_without_providers();
    let original = json!({
        "client_request_id": "conflict-1",
        "prompt": "Develop a detailed migration strategy",
        "route_policy": {
            "mode": "explicit",
            "preferred_provider": "unavailable-test-provider"
        }
    });
    let accepted = runtime.request(
        "POST",
        "/api/v1/operator/threads/default/turns",
        Some(TOKEN),
        original.clone(),
    );
    assert_eq!(accepted.status, 202, "{}", accepted.body);
    let turn_id = accepted.body["data"]["turn_id"].as_str().unwrap();
    wait_for_terminal_event(&runtime, "default", turn_id);
    let before = operator_event_count(&runtime, "default");

    let mut changed_prompt = original.clone();
    changed_prompt["prompt"] = json!("Develop a different migration strategy");
    let prompt_conflict = runtime.request(
        "POST",
        "/api/v1/operator/threads/default/turns",
        Some(TOKEN),
        changed_prompt,
    );
    assert_eq!(prompt_conflict.status, 409, "{}", prompt_conflict.body);
    assert_eq!(
        prompt_conflict.body["error"]["code"],
        "idempotency_conflict"
    );

    let mut changed_policy = original;
    changed_policy["route_policy"]["preferred_provider"] = json!("other-provider");
    let policy_conflict = runtime.request(
        "POST",
        "/api/v1/operator/threads/default/turns",
        Some(TOKEN),
        changed_policy,
    );
    assert_eq!(policy_conflict.status, 409, "{}", policy_conflict.body);
    assert_eq!(
        policy_conflict.body["error"]["code"],
        "idempotency_conflict"
    );
    assert_eq!(operator_event_count(&runtime, "default"), before);

    for body in [
        json!({"client_request_id": "safe-id", "prompt": "ghp_live-token"}),
        json!({"client_request_id": "ghp_live-token", "prompt": "safe prompt"}),
        json!({
            "client_request_id": "safe-policy-id",
            "prompt": "safe prompt",
            "route_policy": {
                "mode": "explicit",
                "preferred_provider": "ghp_live-token"
            }
        }),
    ] {
        let rejected = runtime.request(
            "POST",
            "/api/v1/operator/threads/sensitive/turns",
            Some(TOKEN),
            body,
        );
        assert_eq!(rejected.status, 400, "{}", rejected.body);
        assert_eq!(rejected.body["error"]["code"], "sensitive_material");
    }
    assert_eq!(operator_event_count(&runtime, "sensitive"), 0);
}

#[test]
fn operator_http_accepts_work_id_syntax_and_rejects_unknown_scope_without_rows() {
    let runtime = TestRuntime::start_without_providers();
    let before = operator_event_count(&runtime, "thread-work");

    let rejected = runtime.request(
        "POST",
        "/api/v1/operator/threads/thread-work/turns",
        Some(TOKEN),
        json!({
            "client_request_id": "work-http-1",
            "prompt": "ship it",
            "work_id": "work-missing"
        }),
    );

    assert_eq!(rejected.status, 409, "{}", rejected.body);
    assert_eq!(rejected.body["error"]["code"], "invalid_work_scope");
    assert_eq!(operator_event_count(&runtime, "thread-work"), before);
}

#[test]
fn operator_http_propagates_known_work_scope_through_terminal_execution() {
    let runtime = TestRuntime::start_without_providers();
    let external = runtime.external_sessions();
    external.ensure_thread("thread-work-known").unwrap();
    external
        .append_event(work_created_event(
            &WorkId::parse("work-known").unwrap(),
            "thread-work-known",
            "exercise HTTP Work scope",
            "installation-test",
            "2026-08-25T00:00:00Z",
            || "evt-work-known".to_string(),
        ))
        .unwrap();

    let accepted = runtime.request(
        "POST",
        "/api/v1/operator/threads/thread-work-known/turns",
        Some(TOKEN),
        json!({
            "client_request_id": "work-http-known",
            "prompt": "hi",
            "work_id": "work-known"
        }),
    );
    assert_eq!(accepted.status, 202, "{}", accepted.body);
    let turn_id = accepted.body["data"]["turn_id"].as_str().unwrap();
    wait_for_terminal_event(&runtime, "thread-work-known", turn_id);

    let turn_rows = external
        .events_after("thread-work-known", None, 128)
        .unwrap()
        .events
        .into_iter()
        .filter(|row| row.event.turn_id.as_deref() == Some(turn_id))
        .collect::<Vec<_>>();
    assert!(!turn_rows.is_empty());
    assert!(turn_rows
        .iter()
        .all(|row| row.event.work_id.as_deref() == Some("work-known")));
}

#[test]
fn operator_boundary_rejects_bad_cursor_ids_and_turn_policy() {
    let runtime = TestRuntime::start(true);

    let bad_cursor = runtime.request(
        "GET",
        "/api/v1/operator/threads/default/events?after=not-a-cursor",
        Some(TOKEN),
        json!(null),
    );
    assert_eq!(bad_cursor.status, 400, "{}", bad_cursor.body);
    assert_eq!(bad_cursor.body["error"]["code"], "invalid_cursor");

    for hostile in ["..", "%2Ftmp", "hello%2Fworld", "a%5Cb"] {
        let response = runtime.request(
            "GET",
            &format!("/api/v1/operator/threads/{hostile}"),
            Some(TOKEN),
            json!(null),
        );
        assert_eq!(
            response.status, 400,
            "hostile id {hostile}: {}",
            response.body
        );
        assert_eq!(response.body["error"]["code"], "invalid_id");
    }
    let long_id = "a".repeat(129);
    let response = runtime.request(
        "GET",
        &format!("/api/v1/operator/threads/{long_id}"),
        Some(TOKEN),
        json!(null),
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"]["code"], "invalid_id");

    for body in [json!([]), json!({"thread_id": 7})] {
        let response = runtime.request("POST", "/api/v1/operator/threads", Some(TOKEN), body);
        assert_eq!(response.status, 400, "{}", response.body);
        assert_eq!(response.body["error"]["code"], "invalid_request");
    }

    let sensitive_thread = runtime.request(
        "POST",
        "/api/v1/operator/threads",
        Some(TOKEN),
        json!({"thread_id": "ghp_live-token"}),
    );
    assert_eq!(sensitive_thread.status, 400, "{}", sensitive_thread.body);
    assert_eq!(sensitive_thread.body["error"]["code"], "invalid_id");

    let invalid_requests = [
        json!({"client_request_id": "", "prompt": "hi"}),
        json!({"client_request_id": "request-1", "prompt": ""}),
        json!({"client_request_id": "request-1", "prompt": "hi", "route_policy": {"mode": "cheapest"}}),
        json!({"client_request_id": "request-1", "prompt": "hi", "route_policy": {"mode": 7}}),
        json!({"client_request_id": "request-1", "prompt": "hi", "route_policy": {"privacy": false}}),
        json!({"client_request_id": "request-1", "prompt": "hi", "route_policy": {"mode": "auto", "minimum_quality_class": 0}}),
        json!({"client_request_id": "request-1", "prompt": "hi", "route_policy": {"mode": "auto", "maximum_marginal_cost_usd": -0.01}}),
        json!({"client_request_id": "request-1", "prompt": "hi", "route_policy": {"mode": "auto", "turn_budget_usd": -0.01}}),
    ];
    for body in invalid_requests {
        let response = runtime.request(
            "POST",
            "/api/v1/operator/threads/default/turns",
            Some(TOKEN),
            body,
        );
        assert_eq!(response.status, 400, "{}", response.body);
        assert_eq!(response.body["error"]["code"], "invalid_request");
    }
}

#[test]
fn operator_stream_url_encodes_valid_reserved_thread_characters() {
    let runtime = TestRuntime::start(true);
    let created = runtime.request(
        "POST",
        "/api/v1/operator/threads",
        Some(TOKEN),
        json!({"thread_id": "team&special"}),
    );
    assert_eq!(created.status, 200, "{}", created.body);

    let submitted = runtime.request(
        "POST",
        "/api/v1/operator/threads/team%26special/turns",
        Some(TOKEN),
        json!({"client_request_id": "reserved-url-1", "prompt": "hi"}),
    );
    assert_eq!(submitted.status, 202, "{}", submitted.body);
    assert_eq!(submitted.body["data"]["thread_id"], "team&special");
    assert!(submitted.body["data"]["stream_url"]
        .as_str()
        .unwrap()
        .contains("thread_id=team%26special&"));
}

#[test]
fn model_submission_is_accepted_before_provider_preparation_and_fails_durably() {
    let runtime = TestRuntime::start_without_providers();
    let submitted = runtime.request(
        "POST",
        "/api/v1/operator/threads/default/turns",
        Some(TOKEN),
        json!({
            "client_request_id": "deferred-provider-preparation-1",
            "prompt": "Develop a detailed strategy for migrating a distributed database safely",
            "route_policy": {
                "mode": "explicit",
                "preferred_provider": "unavailable-test-provider"
            }
        }),
    );
    assert_eq!(submitted.status, 202, "{}", submitted.body);
    let turn_id = submitted.body["data"]["turn_id"].as_str().unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let events = runtime.request(
            "GET",
            "/api/v1/operator/threads/default/events?limit=100",
            Some(TOKEN),
            json!(null),
        );
        assert_eq!(events.status, 200, "{}", events.body);
        let turn_events = events.body["data"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["event"]["turn_id"].as_str() == Some(turn_id))
            .collect::<Vec<_>>();
        let event_types = turn_events
            .iter()
            .filter_map(|row| row["event"]["event_type"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            event_types.get(..2),
            Some(&["turn_started", "user_message"][..])
        );
        if let Some(terminal) = turn_events
            .iter()
            .find(|row| row["event"]["event_type"] == "turn_interrupted")
        {
            assert_eq!(terminal["event"]["payload"]["reason"], "EXECUTION_FAILED");
            // The durable-failure property is what this test is named for, and
            // it holds on every machine. The MESSAGE does not: which failure
            // the turn hits first depends on what the host has, and the wording
            // moves with the guidance copy. A machine with providers configured
            // rejects the unknown preferred provider ("... is not available"), a
            // runner with an unusable account explains that account ("Connect a
            // provider ... `ollama` is not installed"), and a machine with no
            // accounts at all short-circuits earlier. Matching any fixed subset
            // of those strings is a test of the environment, so assert the
            // contract instead: the turn fails durably and says something about
            // why.
            let message = terminal["event"]["payload"]["message"]
                .as_str()
                .unwrap_or_default();
            assert!(
                !message.trim().is_empty(),
                "a durable failure must carry an actionable message: {terminal}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "turn did not terminate: {event_types:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn app_boot_recovers_open_turn_exactly_once_across_restarts() {
    let home = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    let sessions = OperatorSessionService::new(
        OperatorJournal::new(evidence.path().to_path_buf()).expect("operator journal"),
    );
    let submission = sessions
        .start_turn(
            "restart-thread",
            StartTurnRequest::auto("restart-request", "resume after restart"),
        )
        .unwrap();
    drop(sessions);

    for restart in 0..2 {
        let state = tempfile::tempdir().unwrap();
        let port = reserve_port();
        let mut child = spawn_runtime(port, home.path(), evidence.path(), Some(state.path()));
        wait_for_file(&state.path().join("workers.json"));
        let reader = OperatorSessionService::new(
            OperatorJournal::new(evidence.path().to_path_buf()).expect("operator journal"),
        );
        let events = reader
            .events_after("restart-thread", None, 100)
            .unwrap()
            .events;
        let interruptions = events
            .iter()
            .filter(|row| {
                row.event.turn_id.as_deref() == Some(submission.turn_id.as_str())
                    && row.event.event_type == heiwa_evidence::OperatorEventType::TurnInterrupted
                    && row.event.payload["reason"] == "RUNTIME_RESTART"
            })
            .count();
        assert_eq!(
            interruptions, 1,
            "restart {restart} must leave exactly one durable recovery event"
        );
        drop(reader);
        child.stop_and_assert_closed();
    }
}

#[test]
fn live_non_app_session_writer_blocks_app_recovery_until_release() {
    let home = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    let sessions = OperatorSessionService::new(
        OperatorJournal::new(evidence.path().to_path_buf()).expect("operator journal"),
    );
    let submission = sessions
        .start_turn(
            "cli-thread",
            StartTurnRequest::auto("cli-live-turn", "owned by a non-app writer"),
        )
        .unwrap();
    let events_before = sessions
        .events_after("cli-thread", None, 100)
        .unwrap()
        .events
        .len();

    let rejected_state = tempfile::tempdir().unwrap();
    let rejected_port = reserve_port();
    let mut rejected = spawn_runtime(
        rejected_port,
        home.path(),
        evidence.path(),
        Some(rejected_state.path()),
    );
    let status = rejected
        .wait_for_exit(Duration::from_secs(5))
        .expect("app startup must fail while a non-app session writer is live");
    assert!(!status.success());
    wait_for_port_closed(rejected_port);
    assert!(!rejected_state.path().join("workers.json").exists());
    assert_eq!(
        sessions
            .events_after("cli-thread", None, 100)
            .unwrap()
            .events
            .len(),
        events_before
    );
    assert_eq!(
        sessions.thread("cli-thread").unwrap().turns[0].status,
        "open"
    );
    drop(sessions);

    for restart in 0..2 {
        let state = tempfile::tempdir().unwrap();
        let port = reserve_port();
        let mut runtime = spawn_runtime(port, home.path(), evidence.path(), Some(state.path()));
        wait_for_file(&state.path().join("workers.json"));
        let reader = OperatorSessionService::new(
            OperatorJournal::new(evidence.path().to_path_buf()).expect("operator journal"),
        );
        let interruptions = reader
            .events_after("cli-thread", None, 100)
            .unwrap()
            .events
            .iter()
            .filter(|row| {
                row.event.turn_id.as_deref() == Some(submission.turn_id.as_str())
                    && row.event.event_type == heiwa_evidence::OperatorEventType::TurnInterrupted
                    && row.event.payload["reason"] == "RUNTIME_RESTART"
            })
            .count();
        assert_eq!(interruptions, 1, "restart {restart} duplicated recovery");
        drop(reader);
        runtime.stop_and_assert_closed();
    }
}

#[test]
fn operator_runtime_lease_blocks_second_owner_then_allows_recovery() {
    let home = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();

    let first_state = tempfile::tempdir().unwrap();
    let first_port = reserve_port();
    let mut first = spawn_runtime(
        first_port,
        home.path(),
        evidence.path(),
        Some(first_state.path()),
    );
    wait_for_file(&first_state.path().join("workers.json"));

    let sessions = OperatorSessionService::new(
        OperatorJournal::new(evidence.path().to_path_buf()).expect("operator journal"),
    );
    let submission = sessions
        .start_turn(
            "lease-thread",
            StartTurnRequest::auto("lease-live-turn", "remain open while owner is live"),
        )
        .unwrap();
    let events_before_rejected_start = sessions
        .events_after("lease-thread", None, 100)
        .unwrap()
        .events
        .len();

    let isolated_evidence = tempfile::tempdir().unwrap();
    let isolated_state = tempfile::tempdir().unwrap();
    let isolated_port = reserve_port();
    let mut isolated = spawn_runtime(
        isolated_port,
        home.path(),
        isolated_evidence.path(),
        Some(isolated_state.path()),
    );
    wait_for_file(&isolated_state.path().join("workers.json"));

    let second_state = tempfile::tempdir().unwrap();
    let second_port = reserve_port();
    let mut second = spawn_runtime(
        second_port,
        home.path(),
        evidence.path(),
        Some(second_state.path()),
    );
    let second_status = second
        .wait_for_exit(Duration::from_secs(5))
        .expect("second runtime sharing an evidence root must fail closed");
    assert!(!second_status.success());
    wait_for_port_closed(second_port);
    assert!(
        !second_state.path().join("workers.json").exists(),
        "lease rejection must happen before heartbeat"
    );
    let still_open = sessions.thread("lease-thread").unwrap();
    assert_eq!(still_open.turns[0].status, "open");
    assert_eq!(
        sessions
            .events_after("lease-thread", None, 100)
            .unwrap()
            .events
            .len(),
        events_before_rejected_start,
        "rejected second runtime must not append to another owner's live stream"
    );

    isolated.stop_and_assert_closed();

    drop(sessions);
    first.stop_and_assert_closed();

    for restart in 0..2 {
        let recovery_state = tempfile::tempdir().unwrap();
        let recovery_port = reserve_port();
        let mut recovery = spawn_runtime(
            recovery_port,
            home.path(),
            evidence.path(),
            Some(recovery_state.path()),
        );
        wait_for_file(&recovery_state.path().join("workers.json"));
        let reader = OperatorSessionService::new(
            OperatorJournal::new(evidence.path().to_path_buf()).expect("operator journal"),
        );
        let interruptions = reader
            .events_after("lease-thread", None, 100)
            .unwrap()
            .events
            .iter()
            .filter(|row| {
                row.event.turn_id.as_deref() == Some(submission.turn_id.as_str())
                    && row.event.event_type == heiwa_evidence::OperatorEventType::TurnInterrupted
                    && row.event.payload["reason"] == "RUNTIME_RESTART"
            })
            .count();
        assert_eq!(
            interruptions, 1,
            "successful restart {restart} must not duplicate recovery"
        );
        // Emptiness via metadata, not fs::read: these sidecars are held under
        // an exclusive lock, and Windows locks are mandatory - reading a locked
        // range fails with ERROR_LOCK_VIOLATION (os error 33). Unix flock is
        // advisory, so reads succeeded there and hid this on macOS and Linux.
        assert_eq!(
            std::fs::metadata(evidence.path().join(".operator_runtime.lock"))
                .unwrap()
                .len(),
            0,
            "runtime lease sidecar must never contain identity or auth material"
        );
        assert_eq!(
            std::fs::metadata(evidence.path().join(".operator_activity.lock"))
                .unwrap()
                .len(),
            0,
            "activity lease sidecar must never contain identity or auth material"
        );
        drop(reader);
        recovery.stop_and_assert_closed();
    }
}

#[test]
fn app_start_honors_explicit_state_dir_without_touching_home_state() {
    let home = tempfile::tempdir().unwrap();
    let evidence = tempfile::tempdir().unwrap();
    let isolated_state = tempfile::tempdir().unwrap();
    let port = reserve_port();
    let mut child = spawn_runtime(
        port,
        home.path(),
        evidence.path(),
        Some(isolated_state.path()),
    );
    wait_for_file(&isolated_state.path().join("workers.json"));

    assert!(
        isolated_state.path().join("workers.json").is_file(),
        "app heartbeat must land in the explicit state directory"
    );
    assert!(
        !home.path().join(".heiwa/state/workers.json").exists(),
        "isolated app start must not touch HOME-derived state"
    );
    child.stop_and_assert_closed();
}

struct SpawnedRuntime {
    child: Child,
    port: u16,
}

impl SpawnedRuntime {
    fn stop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();

        if self.wait_for_exit(Duration::from_secs(2)).is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                Ok(None) | Err(_) => return None,
            }
        }
    }

    fn stop_and_assert_closed(&mut self) {
        self.stop();
        wait_for_port_closed(self.port);
    }
}

impl Drop for SpawnedRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_runtime(
    port: u16,
    home: &std::path::Path,
    evidence: &std::path::Path,
    state: Option<&std::path::Path>,
) -> SpawnedRuntime {
    let mut command = Command::new(env!("CARGO_BIN_EXE_heiwa"));
    command
        .env("HOME", home)
        .env_remove("HEIWA_HOME")
        .env("HEIWA_EVIDENCE_DIR", evidence)
        .env("HEIWA_MACHINE_AUTH_TOKEN", TOKEN)
        .args(["app", "start", "--port", &port.to_string(), "--no-open"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(state) = state {
        command.env("HEIWA_STATE_DIR", state);
    } else {
        command.env_remove("HEIWA_STATE_DIR");
    }
    SpawnedRuntime {
        child: command.spawn().expect("start test runtime"),
        port,
    }
}

fn reserve_port() -> u16 {
    static ISSUED_PORTS: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();
    let issued = ISSUED_PORTS.get_or_init(|| Mutex::new(HashSet::new()));

    loop {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut ports = issued
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if ports.insert(port) {
            return port;
        }
    }
}

#[test]
fn reserved_runtime_ports_are_unique_across_parallel_callers() {
    let callers = (0..32)
        .map(|_| thread::spawn(reserve_port))
        .collect::<Vec<_>>();
    let ports = callers
        .into_iter()
        .map(|caller| caller.join().expect("port reservation caller"))
        .collect::<HashSet<_>>();

    assert_eq!(ports.len(), 32);
}

fn wait_for_file(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if path.is_file() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("test runtime did not create {}", path.display());
}

fn wait_for_port_closed(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("test runtime still listens on port {port} after shutdown");
}

fn operator_event_count(runtime: &TestRuntime, thread_id: &str) -> usize {
    let response = runtime.request(
        "GET",
        &format!("/api/v1/operator/threads/{thread_id}/events?limit=500"),
        Some(TOKEN),
        json!(null),
    );
    assert_eq!(response.status, 200, "{}", response.body);
    response.body["data"]["events"].as_array().unwrap().len()
}

fn wait_for_terminal_event(runtime: &TestRuntime, thread_id: &str, turn_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = runtime.request(
            "GET",
            &format!("/api/v1/operator/threads/{thread_id}/events?limit=500"),
            Some(TOKEN),
            json!(null),
        );
        assert_eq!(response.status, 200, "{}", response.body);
        if response.body["data"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| {
                row["event"]["turn_id"].as_str() == Some(turn_id)
                    && matches!(
                        row["event"]["event_type"].as_str(),
                        Some("turn_completed" | "turn_interrupted" | "blocker")
                    )
            })
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "turn {turn_id} did not terminate"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_port(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("test runtime did not listen on port {port}");
}

fn request(port: u16, method: &str, target: &str, token: Option<&str>, body: &str) -> Response {
    request_with_host(
        port,
        &format!("127.0.0.1:{port}"),
        method,
        target,
        token,
        body,
    )
}

fn request_with_host(
    port: u16,
    host: &str,
    method: &str,
    target: &str,
    token: Option<&str>,
    body: &str,
) -> Response {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} {target} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nContent-Type: application/json\r\n{authorization}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let raw = read_http_response(&mut stream);
    parse_response(&raw)
}

fn signed_request(
    port: u16,
    method: &str,
    target: &str,
    body: &str,
    signed: &LocalRequestSignature,
) -> Response {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let request = format!(
        "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nContent-Type: application/json\r\nX-Heiwa-Local-Auth-Version: {}\r\nX-Heiwa-Local-Auth-Timestamp: {}\r\nX-Heiwa-Local-Auth-Nonce: {}\r\nX-Heiwa-Local-Auth-Signature: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        signed.version,
        signed.timestamp,
        signed.nonce,
        signed.signature,
        body.len(),
    );
    stream.write_all(request.as_bytes()).unwrap();
    let raw = read_http_response(&mut stream);
    parse_response(&raw)
}

fn read_http_response(stream: &mut TcpStream) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if http_response_is_complete(&raw) {
            return raw;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return raw,
            Ok(read) => raw.extend_from_slice(&chunk[..read]),
            Err(error)
                if error.kind() == std::io::ErrorKind::ConnectionReset
                    && http_response_is_complete(&raw) =>
            {
                return raw;
            }
            Err(error) => panic!("read HTTP response: {error}"),
        }
    }
}

fn http_response_is_complete(raw: &[u8]) -> bool {
    let Some(split) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&raw[..split]);
    let content_length = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    content_length.is_some_and(|length| raw.len() >= split + 4 + length)
}

fn parse_response(raw: &[u8]) -> Response {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("response headers");
    let head = String::from_utf8_lossy(&raw[..split]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap()
        .parse()
        .unwrap();
    let body = serde_json::from_slice(&raw[split + 4..]).unwrap_or_else(|error| {
        panic!(
            "response body was not JSON: {error}: {}",
            String::from_utf8_lossy(&raw[split + 4..])
        )
    });
    Response { status, body }
}

fn unix_timestamp_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn websocket_handshake(port: u16, token: Option<&str>) -> HandshakeResponse {
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    websocket_handshake_with_headers(port, "/ws/v1/operator", &authorization)
}

fn websocket_handshake_with_headers(
    port: u16,
    target: &str,
    additional_headers: &str,
) -> HandshakeResponse {
    websocket_handshake_with_host(
        port,
        &format!("127.0.0.1:{port}"),
        target,
        additional_headers,
    )
}

fn websocket_handshake_with_host(
    port: u16,
    host: &str,
    target: &str,
    additional_headers: &str,
) -> HandshakeResponse {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let request = format!(
        "GET {target} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{additional_headers}\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();

    let mut raw = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                raw.extend_from_slice(&buffer[..read]);
                if let Some(split) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&raw[..split]);
                    let status = head
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1));
                    let content_length = head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    });
                    if status == Some("101")
                        || content_length.is_some_and(|length| raw.len() >= split + 4 + length)
                    {
                        break;
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => break,
            Err(error) => panic!("websocket handshake read failed: {error}"),
        }
    }
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("websocket response headers");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap()
        .parse()
        .unwrap();
    let body = if status == 101 || raw[split + 4..].is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&raw[split + 4..]).unwrap()
    };
    HandshakeResponse { status, head, body }
}

fn connect_operator_websocket(port: u16, target: &str, token: &str) -> (TcpStream, u16) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = format!(
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nAuthorization: Bearer {token}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let status = String::from_utf8(head)
        .unwrap()
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap()
        .parse()
        .unwrap();
    (stream, status)
}

fn read_websocket_json(stream: &mut TcpStream) -> Value {
    let mut header = [0_u8; 2];
    stream.read_exact(&mut header).unwrap();
    assert_eq!(header[0], 0x81);
    assert_eq!(header[1] & 0x80, 0);
    let length = match header[1] & 0x7f {
        value @ 0..=125 => value as usize,
        126 => {
            let mut bytes = [0_u8; 2];
            stream.read_exact(&mut bytes).unwrap();
            u16::from_be_bytes(bytes) as usize
        }
        127 => {
            let mut bytes = [0_u8; 8];
            stream.read_exact(&mut bytes).unwrap();
            usize::try_from(u64::from_be_bytes(bytes)).unwrap()
        }
        _ => unreachable!(),
    };
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).unwrap();
    serde_json::from_slice(&payload).unwrap()
}
