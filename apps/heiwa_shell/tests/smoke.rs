use heiwa_evidence::{OperatorEventType, OperatorJournal};
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use tempfile::tempdir;

struct HermeticCommand {
    command: Command,
    _root: tempfile::TempDir,
    evidence_dir: PathBuf,
}

impl Deref for HermeticCommand {
    type Target = Command;

    fn deref(&self) -> &Self::Target {
        &self.command
    }
}

impl DerefMut for HermeticCommand {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.command
    }
}

fn write_fake_executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write provider fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("make provider fixture executable");
    }
}

fn heiwa_command() -> HermeticCommand {
    let root = tempfile::tempdir().expect("create hermetic shell root");
    let home = root.path().join("home");
    let evidence_dir = root.path().join("evidence");
    let state_dir = root.path().join("state");
    let index_dir = root.path().join("index");
    let fixture_bin = root.path().join("bin");
    let rust_sysroot = root.path().join("rust-sysroot");
    let rust_lld = rust_sysroot
        .join("lib")
        .join("rustlib")
        .join("aarch64-apple-darwin")
        .join("bin")
        .join("rust-lld");
    for directory in [&home, &evidence_dir, &state_dir, &index_dir, &fixture_bin] {
        std::fs::create_dir_all(directory).expect("create hermetic shell directory");
    }
    std::fs::create_dir_all(rust_lld.parent().expect("fake rust-lld parent"))
        .expect("create fake Rust sysroot");
    std::fs::write(&rust_lld, b"hermetic rust-lld fixture").expect("write fake bundled linker");
    write_fake_executable(
        &fixture_bin.join("rustc"),
        "#!/bin/sh\nif [ \"$1 $2\" = \"--print sysroot\" ]; then\n  printf '%s\\n' \"$HEIWA_TEST_RUST_SYSROOT\"\nelse\n  exit 1\nfi\n",
    );
    write_fake_executable(
        &fixture_bin.join("ollama"),
        "#!/bin/sh\ncase \"$1\" in\n  list) printf 'NAME ID SIZE MODIFIED\\n' ;;\n  ps) printf 'NAME ID SIZE PROCESSOR UNTIL\\n' ;;\n  *) exit 1 ;;\nesac\n",
    );
    for provider_cli in ["claude", "codex", "gemini"] {
        write_fake_executable(
            &fixture_bin.join(provider_cli),
            "#!/bin/sh\nprintf 'hermetic provider fixture\\n'\n",
        );
    }

    let mut command = Command::new(env!("CARGO_BIN_EXE_heiwa"));
    command
        .env_clear()
        .env("HOME", &home)
        .env("USER", "heiwa-test")
        .env("LOGNAME", "heiwa-test")
        .env("TMPDIR", root.path())
        .env("HEIWA_EVIDENCE_DIR", &evidence_dir)
        .env("HEIWA_STATE_DIR", &state_dir)
        .env("HEIWA_INDEX_DIR", &index_dir)
        .env("HEIWA_DISABLE_KEYCHAIN", "1")
        .env("HEIWA_TEST_RUST_SYSROOT", &rust_sysroot)
        .env("HEIWA_OLLAMA_BASE", "disabled-for-hermetic-tests")
        .env("NO_COLOR", "1")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("PATH", format!("{}:/usr/bin:/bin", fixture_bin.display()));
    HermeticCommand {
        command,
        _root: root,
        evidence_dir,
    }
}

#[test]
fn test_heiwa_help() {
    let output = heiwa_command()
        .arg("--help")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("BYOK terminal agent"));
}

#[test]
fn test_heiwa_providers_lists_wrapped_and_loop_capable_surfaces_honestly() {
    let output = heiwa_command()
        .arg("providers")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Controlled fixture PATH surfaces wrapped CLIs without contacting live
    // provider runtimes or operator account state.
    assert!(stdout.contains("claude"), "expected fixture CLI: {stdout}");
    assert!(
        stdout.contains("[loop]"),
        "expected explicit loop capability marker: {stdout}"
    );
    // CLI discovery should surface known providers not yet in the registry
    assert!(
        stdout.contains("claude")
            || stdout.contains("antigravity")
            || stdout.contains("CLI Discovery"),
        "expected CLI discovery section or known providers: {stdout}"
    );
}

fn run_shell_script(script: &str) -> std::process::Output {
    let mut child = heiwa_command()
        .arg("shell")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn heiwa shell");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write shell script");

    child.wait_with_output().expect("wait for shell output")
}

#[test]
fn test_shell_supports_model_and_provider_slash_commands() {
    let output = run_shell_script("/model\n/provider\nquit\n");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Unknown slash command: /model"),
        "expected /model to be handled: {stdout}"
    );
    assert!(
        !stdout.contains("Unknown slash command: /provider"),
        "expected /provider to be handled: {stdout}"
    );
}

#[test]
fn test_shell_supports_route_status_and_clear_slash_commands() {
    let output = run_shell_script("/route\n/status\n/clear\nquit\n");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Unknown slash command: /route"),
        "expected /route to be handled: {stdout}"
    );
    assert!(
        !stdout.contains("Unknown slash command: /status"),
        "expected /status to be handled: {stdout}"
    );
    assert!(
        !stdout.contains("Unknown slash command: /clear"),
        "expected /clear to be handled: {stdout}"
    );
}

#[test]
fn test_shell_supports_cwd_and_directory_scope_commands() {
    let output = run_shell_script("/cwd\n/add-dir ~/*\n/dirs\nquit\n");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cwd:"),
        "expected /cwd to report the working directory: {stdout}"
    );
    assert!(
        stdout.contains("allowed dirs:"),
        "expected /dirs to report allowed directories: {stdout}"
    );
    assert!(
        !stdout.contains("unknown command: /cwd"),
        "expected /cwd to be handled: {stdout}"
    );
    assert!(
        !stdout.contains("unknown command: /add-dir"),
        "expected /add-dir to be handled: {stdout}"
    );
}

#[test]
fn test_shell_greeting_is_handled_without_model_requirement() {
    let output = run_shell_script("hi\nquit\n");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Ready"),
        "expected deterministic greeting response: {stdout}"
    );
    assert!(
        !stdout.contains("No models available. Connect a provider first."),
        "greetings should not require a model: {stdout}"
    );
}

#[test]
fn plain_repl_success_has_one_canonical_closed_turn() {
    let mut command = heiwa_command();
    let evidence_dir = command.evidence_dir.clone();
    let mut child = command
        .arg("shell")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hermetic heiwa shell");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"hi\nquit\n")
        .expect("write deterministic turn");
    let output = child.wait_with_output().expect("wait for shell");
    assert!(output.status.success(), "{output:?}");

    let rows = OperatorJournal::new(evidence_dir)
        .unwrap()
        .read_after(None, 64)
        .unwrap()
        .events;
    let count = |event_type| {
        rows.iter()
            .filter(|row| row.event.event_type == event_type)
            .count()
    };
    assert_eq!(count(OperatorEventType::UserMessage), 1);
    assert_eq!(count(OperatorEventType::AssistantCompleted), 1);
    assert_eq!(count(OperatorEventType::TurnCompleted), 1);
    assert_eq!(count(OperatorEventType::TurnInterrupted), 0);
}

#[test]
fn test_route_preview_greeting_does_not_execute_model() {
    let output = heiwa_command()
        .args(["route", "preview", "hi"])
        .output()
        .expect("failed to execute route preview");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("mode: deterministic"),
        "expected deterministic route preview: {stdout}"
    );
    assert!(
        stdout.contains("Ready"),
        "expected deterministic response body: {stdout}"
    );
}

#[test]
fn test_route_preview_surfaces_privacy_lane() {
    let output = heiwa_command()
        .args(["route", "preview", "summarize my priority mail privately"])
        .output()
        .expect("failed to execute route preview");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("privacy: sovereign"),
        "expected privacy-aware route preview: {stdout}"
    );
    assert!(
        stdout.contains("mode: local_model") || stdout.contains("mode: unavailable"),
        "private prompt should use a local model when available or honestly report unavailable: {stdout}"
    );
    assert!(
        !stdout.contains("mode: remote_model"),
        "private prompt must not route to a remote model: {stdout}"
    );
}

#[test]
fn test_life_status_json_reports_sources_and_evidence_mode() {
    let output = heiwa_command()
        .args(["life", "status", "--json"])
        .output()
        .expect("failed to execute life status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"life status\""),
        "expected life status json command marker: {stdout}"
    );
    assert!(
        stdout.contains("\"evidence_mode\":"),
        "expected evidence mode in life status json: {stdout}"
    );
    assert!(
        stdout.contains("\"home\""),
        "expected home source group in life status json: {stdout}"
    );
}

#[test]
fn test_life_today_json_reports_local_read_model_keys() {
    let output = heiwa_command()
        .args(["life", "today", "--json"])
        .output()
        .expect("failed to execute life today");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("life today --json must be valid JSON");

    assert_eq!(parsed["command"], "life today");
    assert_eq!(parsed["timezone"], "America/Vancouver");
    let date = parsed["date"].as_str().expect("date string");
    assert_eq!(date.len(), 10, "date must be YYYY-MM-DD: {date}");
    assert!(parsed["day_type"].is_string());
    assert!(parsed["work_shifts"].is_array());
    assert!(parsed["appointments"].is_array());
    assert!(parsed["stale_facts"].is_array());
    assert!(parsed["pending_approvals"].is_array());
    assert!(parsed["runtime"]["evidence_mode"].is_string());
}

#[test]
fn test_life_freshness_json_reports_source_slas() {
    let output = heiwa_command()
        .args(["life", "freshness", "--json"])
        .output()
        .expect("failed to execute life freshness");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("life freshness --json must be valid JSON");

    assert_eq!(parsed["command"], "life freshness");
    assert!(parsed["stale_sources"].is_number());
    let sources = parsed["sources"]
        .as_array()
        .expect("life freshness sources must be an array");
    let scorecard = sources
        .iter()
        .find(|source| source["label"] == "daily_scorecard.md")
        .expect("daily scorecard source must be reported");
    assert_eq!(scorecard["sla_days"], 1);
    assert!(scorecard["age_days"].is_number() || scorecard["age_days"].is_null());
    assert!(scorecard["stale"].is_boolean());

    let register = sources
        .iter()
        .find(|source| source["label"] == "current_state_register.md")
        .expect("current state register source must be reported");
    assert_eq!(register["sla_days"], 7);
}

#[test]
fn test_life_import_home_dry_run_jsonl_counts_rows() {
    let output = heiwa_command()
        .args(["life", "import", "home", "--dry-run", "--jsonl"])
        .output()
        .expect("failed to execute life import dry run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"dry_run\":true"),
        "expected dry-run marker in import jsonl: {stdout}"
    );
    assert!(
        stdout.contains("\"table\":\"life_sources\""),
        "expected life_sources row count in import jsonl: {stdout}"
    );
    assert!(
        stdout.contains("\"table\":\"life_memory_events\""),
        "expected life_memory_events row count in import jsonl: {stdout}"
    );
}

#[test]
fn test_app_help_exposes_boot_command_boundary() {
    let output = heiwa_command()
        .args(["app", "--help"])
        .output()
        .expect("failed to execute app help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("heiwa app"),
        "expected app help to expose boot command: {stdout}"
    );
    assert!(
        stdout.contains("runtime status"),
        "expected app help to include runtime status command: {stdout}"
    );
    assert!(
        stdout.contains("app update"),
        "expected app help to include local update command: {stdout}"
    );
    assert!(
        stdout.contains("--json"),
        "expected app help to include --json flag: {stdout}"
    );
}

#[test]
fn test_app_update_dry_run_defaults_to_github_release_source() {
    let output = heiwa_command()
        .args(["app", "update", "--dry-run"])
        .output()
        .expect("failed to execute app update dry-run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("source_mode: github-release"),
        "expected update dry-run to identify GitHub release source mode: {stdout}"
    );
    assert!(
        stdout.contains("source: https://github.com/Heiwa-Limited/heiwa-universe/releases"),
        "expected update dry-run to identify GitHub Releases as source: {stdout}"
    );
    assert!(
        stdout.contains(
            "release_api: https://api.github.com/repos/Heiwa-Limited/heiwa-universe/releases/latest"
        ),
        "expected update dry-run to expose latest release API: {stdout}"
    );
    assert!(
        stdout.contains("restart_policy: prompt-before-restart"),
        "expected update dry-run to expose restart policy: {stdout}"
    );
    assert!(
        stdout.contains("dry_run: true"),
        "expected dry-run marker: {stdout}"
    );
}

#[test]
fn test_app_update_checkout_source_reports_dev_reinstall_target() {
    let output = heiwa_command()
        .args(["app", "update", "--source", "checkout", "--dry-run"])
        .output()
        .expect("failed to execute app update checkout dry-run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cargo install --path apps/heiwa_shell --root ~/.heiwa --locked --force"),
        "expected checkout update dry-run to expose install command: {stdout}"
    );
    assert!(
        stdout.contains("source_mode: checkout-dev"),
        "expected checkout update dry-run to identify checkout-dev source mode: {stdout}"
    );
    assert!(
        stdout.contains("official_source: GitHub Releases"),
        "expected checkout update dry-run to identify official GitHub release source: {stdout}"
    );
    assert!(
        stdout.contains("dry_run: true"),
        "expected dry-run marker: {stdout}"
    );
}

#[test]
fn test_app_update_checkout_dry_run_json_reports_promotion_contract() {
    let output = heiwa_command()
        .args([
            "app",
            "update",
            "--source",
            "checkout",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("failed to execute app update checkout dry-run json");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("expected JSON update plan, got {err}: {stdout}"));

    assert_eq!(
        payload.get("command").and_then(serde_json::Value::as_str),
        Some("app update")
    );
    assert_eq!(
        payload
            .get("source_mode")
            .and_then(serde_json::Value::as_str),
        Some("checkout-dev")
    );
    assert_eq!(
        payload.get("dry_run").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        payload
            .get("restart_policy")
            .and_then(serde_json::Value::as_str),
        Some("prompt-before-restart")
    );
    let expected_source = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .expect("resolve checkout root for update-plan assertion");
    assert!(expected_source.status.success());
    let expected_source = String::from_utf8(expected_source.stdout)
        .expect("checkout root is UTF-8")
        .trim()
        .to_string();
    assert_eq!(
        payload.get("source").and_then(serde_json::Value::as_str),
        Some(expected_source.as_str())
    );
    assert!(payload
        .get("source_branch")
        .and_then(serde_json::Value::as_str)
        .is_some());
    assert!(payload
        .get("source_commit")
        .and_then(serde_json::Value::as_str)
        .is_some());
    assert!(payload
        .get("source_dirty")
        .and_then(serde_json::Value::as_bool)
        .is_some());
    // Normalize separators: Windows joins with backslashes.
    assert!(payload
        .get("installed_bin")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|path| path.replace('\\', "/").ends_with("/.heiwa/bin/heiwa")));
    assert!(payload
        .get("installed_app")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|path| path.replace('\\', "/").ends_with("/.heiwa/app/Heiwa.app")));
    assert!(payload
        .get("app_bundle_update")
        .and_then(serde_json::Value::as_object)
        .and_then(|update| update.get("wired").and_then(serde_json::Value::as_bool))
        .is_some());
    assert!(payload
        .get("install_command")
        .and_then(serde_json::Value::as_array)
        .is_some());
    let cargo_environment = payload
        .get("cargo_environment")
        .and_then(serde_json::Value::as_object)
        .expect("checkout cargo environment contract object");
    assert!(cargo_environment
        .get("strategy")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|strategy| !strategy.is_empty()));
    assert!(payload
        .get("verification_commands")
        .and_then(serde_json::Value::as_array)
        .is_some());
    assert!(payload
        .get("active_work")
        .and_then(serde_json::Value::as_object)
        .is_some());
    let promotion_receipt = payload
        .get("promotion_receipt")
        .and_then(serde_json::Value::as_object)
        .expect("promotion receipt contract object");
    assert_eq!(
        promotion_receipt
            .get("schema_version")
            .and_then(serde_json::Value::as_str),
        Some("heiwa_promotion_receipt_v1")
    );
    assert!(promotion_receipt
        .get("source")
        .is_some_and(serde_json::Value::is_object));
    assert!(promotion_receipt
        .get("target")
        .is_some_and(serde_json::Value::is_object));
    assert!(promotion_receipt
        .get("runtime_probes")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|probes| probes.iter().any(|probe| probe
            .get("endpoint")
            .and_then(serde_json::Value::as_str)
            == Some("/api/v1/capabilities"))));
    assert!(promotion_receipt
        .get("codesign")
        .is_some_and(serde_json::Value::is_object));
    assert_eq!(
        promotion_receipt.get("cargo_environment"),
        payload.get("cargo_environment")
    );
    assert_eq!(
        promotion_receipt
            .get("would_write")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    let evidence_plane = promotion_receipt
        .get("evidence_plane")
        .and_then(serde_json::Value::as_object)
        .expect("evidence plane contract");
    assert_eq!(
        evidence_plane
            .get("backend")
            .and_then(serde_json::Value::as_str),
        Some("local-jsonl")
    );
    assert!(evidence_plane
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some());
}

#[test]
fn test_doctor_json_reports_runtimes_providers_and_app_probe() {
    let output = heiwa_command()
        .args(["doctor", "--json"])
        .output()
        .expect("failed to execute doctor --json");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"doctor\""),
        "expected doctor json command marker: {stdout}"
    );
    assert!(
        stdout.contains("\"runtimes\""),
        "expected runtimes block in doctor json: {stdout}"
    );
    assert!(
        stdout.contains("\"heiwa_app\""),
        "expected heiwa_app probe block in doctor json: {stdout}"
    );
    assert!(
        stdout.contains("\"reachable\""),
        "expected reachable field in doctor json: {stdout}"
    );
    assert!(
        stdout.contains("\"port\":7474"),
        "expected default 7474 port in doctor json: {stdout}"
    );
    assert!(
        !stdout.contains("\"auth_token\""),
        "doctor json must not leak auth_token: {stdout}"
    );
    assert!(
        stdout.contains("\"layout\""),
        "expected layout block in doctor json: {stdout}"
    );
    assert!(
        stdout.contains("\"directories\""),
        "expected directories array in doctor layout: {stdout}"
    );
    for name in [
        "\"name\":\"bin\"",
        "\"name\":\"logs\"",
        "\"name\":\"sessions\"",
        "\"name\":\"cache\"",
        "\"name\":\"state\"",
        "\"name\":\"secrets\"",
        "\"name\":\"plugins\"",
    ] {
        assert!(
            stdout.contains(name),
            "expected {name} in doctor layout: {stdout}"
        );
    }
    assert!(
        stdout.contains("\"evidence\""),
        "expected evidence block in doctor json: {stdout}"
    );
    assert!(
        stdout.contains("\"backend\":\"local-jsonl\""),
        "expected local-jsonl backend in doctor evidence: {stdout}"
    );
    assert!(
        !stdout.contains("\"token\":\""),
        "doctor json must never contain a raw token: {stdout}"
    );
    assert!(
        stdout.contains("\"providers\":["),
        "expected providers array in doctor json: {stdout}"
    );
    assert!(
        stdout.contains("\"provider_accounts\":["),
        "expected provider account summaries in doctor json: {stdout}"
    );
    assert!(
        stdout.contains("\"provider_id\":\"ollama\""),
        "expected ollama in providers array: {stdout}"
    );
    assert!(
        stdout.contains("\"auth_kind\":\"local_runtime\""),
        "expected ollama auth_kind=local_runtime: {stdout}"
    );
    // status values come from heiwa_provider; just assert the field exists
    // on at least one entry (avoid coupling tests to a specific status)
    assert!(
        stdout.matches("\"status\":\"").count() >= 1,
        "expected at least one provider status string: {stdout}"
    );
}

#[test]
fn test_app_runtime_status_json_reports_local_probe() {
    let output = heiwa_command()
        .args(["app", "runtime", "status", "--json"])
        .output()
        .expect("failed to execute app runtime status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"app runtime status\""),
        "expected app runtime status json command marker: {stdout}"
    );
    assert!(
        stdout.contains("\"policy\":\"local-only-no-side-effects\""),
        "expected local-only policy marker: {stdout}"
    );
    assert!(
        stdout.contains("\"hooks\""),
        "expected hooks summary in runtime status: {stdout}"
    );
    assert!(
        stdout.contains("\"source\":\"live-home-config\""),
        "expected live home hook source marker: {stdout}"
    );
    assert!(
        stdout.contains("\"workers\""),
        "expected workers summary in runtime status: {stdout}"
    );
    assert!(
        stdout.contains("\"approvals\""),
        "expected approvals summary in runtime status: {stdout}"
    );
    assert!(
        stdout.contains("\"mail\""),
        "expected mail summary in runtime status: {stdout}"
    );
    assert!(
        stdout.contains("\"keep_awake\""),
        "expected keep_awake probe in runtime status: {stdout}"
    );
    assert!(
        stdout.contains("\"local_app\""),
        "expected local_app reachability probe in runtime status: {stdout}"
    );
}

#[test]
fn test_workers_heartbeat_dry_run_emits_local_envelope() {
    let output = heiwa_command()
        .args([
            "workers",
            "heartbeat",
            "--class",
            "shell_machine",
            "--id",
            "smoke-test-worker",
            "--dry-run",
            "--json",
        ])
        .output()
        .expect("failed to execute workers heartbeat dry-run");

    assert!(
        output.status.success(),
        "workers heartbeat dry-run should succeed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"workers heartbeat\""),
        "expected workers heartbeat json marker: {stdout}"
    );
    assert!(
        stdout.contains("\"dry_run\":true"),
        "expected dry_run true in workers heartbeat: {stdout}"
    );
    assert!(
        stdout.contains("\"worker_id\":\"smoke-test-worker\""),
        "expected smoke-test-worker id: {stdout}"
    );
    assert!(
        stdout.contains("\"class\":\"shell_machine\""),
        "expected shell_machine class: {stdout}"
    );
}

#[test]
fn test_workers_status_json_reports_registry_path() {
    let output = heiwa_command()
        .args(["workers", "status", "--json"])
        .output()
        .expect("failed to execute workers status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"workers status\""),
        "expected workers status json marker: {stdout}"
    );
    assert!(
        stdout.contains("\"path\":"),
        "expected workers.json path in status: {stdout}"
    );
}

#[test]
fn test_approvals_list_json_reports_dispatch_paths() {
    let output = heiwa_command()
        .args(["approvals", "list", "--json"])
        .output()
        .expect("failed to execute approvals list");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"approvals list\""),
        "expected approvals list json marker: {stdout}"
    );
    let normalized_stdout = stdout.replace("\\\\", "/").replace('\\', "/");
    assert!(
        normalized_stdout.contains("dispatch/requests"),
        "expected dispatch/requests directory in approvals list: {stdout}"
    );
    assert!(
        normalized_stdout.contains("dispatch/approvals/decisions"),
        "expected dispatch/approvals/decisions directory in approvals list: {stdout}"
    );
}

#[test]
fn test_approvals_list_json_reports_dispatch_v1_summary() {
    let temp = tempdir().expect("temp home");
    let requests_dir = temp.path().join(".heiwa/state/dispatch/requests");
    std::fs::create_dir_all(&requests_dir).expect("create requests dir");
    std::fs::write(
        requests_dir.join("req_123.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": "operator_dispatch_request_v1",
            "request_id": "req_123",
            "created_at": "2026-03-30T18:20:22.112520Z",
            "action": "write-file",
            "target_surface": "filesystem",
            "target_scope": "/tmp/example.txt",
            "requested_mode": "write"
        }))
        .expect("serialize request"),
    )
    .expect("write request");

    let output = heiwa_command()
        .env("HOME", temp.path())
        .env("HEIWA_HOME", temp.path().join(".heiwa"))
        .env("HEIWA_STATE_DIR", temp.path().join(".heiwa").join("state"))
        .args(["approvals", "list", "--json"])
        .output()
        .expect("failed to execute approvals list");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("approvals list --json must be valid JSON");
    let summary = parsed["pending_summary"]
        .as_array()
        .and_then(|items| items.first())
        .expect("pending_summary must include request");
    assert_eq!(summary["id"], "req_123");
    assert_eq!(summary["action"], "write-file");
    assert_eq!(summary["target"], "filesystem:/tmp/example.txt");
    assert_eq!(summary["risk"], "write");
    assert_eq!(summary["requested_at"], "2026-03-30T18:20:22.112520Z");
}

#[test]
fn test_mail_status_json_enforces_metadata_only_policy() {
    let output = heiwa_command()
        .args(["mail", "status", "--json"])
        .output()
        .expect("failed to execute mail status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"mail status\""),
        "expected mail status json marker: {stdout}"
    );
    assert!(
        stdout.contains("\"policy\":\"metadata-only-no-body\""),
        "expected metadata-only policy guarantee: {stdout}"
    );
    assert!(
        stdout.contains("\"fields\""),
        "expected fields whitelist (account/mailbox/sender/subject/date/unread): {stdout}"
    );
}

#[test]
fn test_capabilities_refresh_dry_run_reports_bounded_redacted_json() {
    let output = heiwa_command()
        .args(["capabilities", "refresh", "--json", "--dry-run"])
        .output()
        .expect("failed to execute capabilities refresh");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"capabilities refresh\""),
        "expected capabilities refresh json marker: {stdout}"
    );
    assert!(
        stdout.contains("\"dry_run\":true"),
        "dry-run must be reported and write nothing: {stdout}"
    );
    assert!(
        stdout.contains("\"redaction_applied\":true"),
        "expected redaction guarantee: {stdout}"
    );
    assert!(
        stdout.contains("\"counts\""),
        "expected bounded counts (not raw catalog body): {stdout}"
    );
    // Redaction: credential paths and live token shapes must never surface.
    for marker in ["auth.json", "ghp_", "Bearer ", "xoxb-", ".codex/auth"] {
        assert!(
            !stdout.contains(marker),
            "sensitive marker {marker:?} leaked into refresh output: {stdout}"
        );
    }
}

#[test]
fn test_capabilities_status_json_reports_catalog_counts() {
    let output = heiwa_command()
        .args(["capabilities", "status", "--json"])
        .output()
        .expect("failed to execute capabilities status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"capabilities status\""),
        "expected capabilities status json marker: {stdout}"
    );
    assert!(
        stdout.contains("\"path\":"),
        "expected capabilities state path in status: {stdout}"
    );
    assert!(
        stdout.contains("\"counts\""),
        "expected bounded counts object in status: {stdout}"
    );
}

#[test]
fn test_calendar_status_json_reports_lanes_and_holds() {
    let output = heiwa_command()
        .args(["calendar", "status", "--json"])
        .output()
        .expect("failed to execute calendar status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"calendar summary\""),
        "expected calendar summary json marker: {stdout}"
    );
    assert!(
        stdout.contains("\"lanes\""),
        "expected connector lanes in calendar summary: {stdout}"
    );
    assert!(
        stdout.contains("\"holds\""),
        "expected holds read model in calendar summary: {stdout}"
    );
    assert!(
        stdout.contains("\"heiwa_holds\""),
        "expected local-first heiwa_holds lane: {stdout}"
    );
}

#[test]
fn test_mail_summary_json_reports_priority_read_model() {
    let output = heiwa_command()
        .args(["mail", "summary"])
        .output()
        .expect("failed to execute mail summary");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"mail summary\""),
        "expected mail summary json marker: {stdout}"
    );
    assert!(
        stdout.contains("\"policy\":\"metadata-only-no-body\""),
        "expected metadata-only policy on summary: {stdout}"
    );
    assert!(
        stdout.contains("\"priority\""),
        "expected priority rows in mail summary: {stdout}"
    );
    assert!(
        stdout.contains("\"snapshot\""),
        "expected snapshot probe in mail summary: {stdout}"
    );
}

#[test]
fn test_connect_status_json_reports_unified_registry() {
    let output = heiwa_command()
        .args(["connect", "status", "--json"])
        .output()
        .expect("failed to execute connect status");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"connectors\""),
        "expected connectors array: {stdout}"
    );
    assert!(
        stdout.contains("\"google_calendar\""),
        "expected google_calendar connector row: {stdout}"
    );
    assert!(
        stdout.contains("\"apple_mail\""),
        "expected apple_mail connector row: {stdout}"
    );
    assert!(
        stdout.contains("read models before external writes"),
        "expected read-model-first policy line: {stdout}"
    );
    assert!(
        stdout.contains("https://www.googleapis.com/auth/gmail.send"),
        "Gmail must expose only the approval-gated send scope: {stdout}"
    );
    assert!(
        !stdout.contains("https://www.googleapis.com/auth/gmail.readonly"),
        "restricted Gmail read scope is forbidden; Mail.app owns reads: {stdout}"
    );
    assert!(
        stdout.contains("OAuth tokens stay in the OS credential vault"),
        "connector registry must name the real secret boundary: {stdout}"
    );
    assert!(
        !stdout.contains("secrets stay local under ~/.heiwa/secrets"),
        "plaintext secret-file policy must not survive the vault migration: {stdout}"
    );
}

#[test]
fn test_mail_scan_dry_run_reports_source_readiness() {
    let output = heiwa_command()
        .args(["mail", "scan", "--dry-run", "--json"])
        .output()
        .expect("failed to execute mail scan dry run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"command\":\"mail scan\""),
        "expected mail scan json marker: {stdout}"
    );
    assert!(
        stdout.contains("\"dry_run\":true"),
        "expected dry run flag: {stdout}"
    );
    assert!(
        stdout.contains("\"apple\"") && !stdout.contains("\"gmail\""),
        "mail scan must expose only the local Apple read lane: {stdout}"
    );
    assert!(
        stdout.contains("\"policy\":\"metadata-only-no-body\""),
        "expected metadata-only policy on scan: {stdout}"
    );
}

#[test]
fn test_mail_scan_refuses_restricted_gmail_read_lane() {
    let output = heiwa_command()
        .args(["mail", "scan", "--source", "gmail", "--dry-run", "--json"])
        .output()
        .expect("failed to execute gmail read refusal");

    assert!(!output.status.success(), "Gmail reads must remain disabled");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Gmail reads are disabled"),
        "expected explicit local-Mail boundary: {stderr}"
    );
}
