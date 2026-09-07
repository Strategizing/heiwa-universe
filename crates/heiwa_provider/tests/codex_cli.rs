#![cfg(unix)]

use heiwa_provider::adapter::{Message, ProviderAdapter, Role, StreamEvent};
use heiwa_provider::providers::codex_cli::CodexCliAdapter;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

// Exercise the public adapter in a child test process, so PATH and HOME cannot
// race other tests or discover the operator's real Codex installation/auth.
fn run_fixture(script: Option<&str>, cancel: bool) -> Value {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "heiwa-codex-stream-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir_all(&fixture.0).unwrap();
    if let Some(script) = script {
        let executable = fixture.0.join("codex");
        fs::write(
            &executable,
            format!("#!/bin/sh\nprintf '%s' \"$$\" > \"$HEIWA_CODEX_FIXTURE/pid\"\n{script}\n"),
        )
        .unwrap();
        fs::set_permissions(executable, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "codex_fixture_driver", "--nocapture"])
        .env_clear()
        .env("PATH", &fixture.0)
        .env("HOME", &fixture.0)
        .env("HEIWA_HOME", fixture.0.join("runtime"))
        .env("HEIWA_BIN_DIRS", "")
        .env("HEIWA_CODEX_FIXTURE", &fixture.0)
        .env("HEIWA_CODEX_CANCEL", if cancel { "1" } else { "0" })
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture driver failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if let Ok(pid) = fs::read_to_string(fixture.0.join("pid")) {
        assert!(
            !Command::new("/bin/kill")
                .args(["-0", &pid])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success(),
            "provider process {pid} outlived its adapter"
        );
    }
    serde_json::from_slice(&fs::read(fixture.0.join("result.json")).unwrap()).unwrap()
}

#[tokio::test]
async fn codex_fixture_driver() {
    let Some(root) = std::env::var_os("HEIWA_CODEX_FIXTURE") else {
        return;
    };
    let (tx, mut rx) = mpsc::channel(32);
    let send = async move {
        CodexCliAdapter::new()
            .send(
                "fixture-model",
                &[Message {
                    role: Role::User,
                    content: "--prompt-is-data".into(),
                }],
                tx,
            )
            .await
    };
    let cancel = std::env::var("HEIWA_CODEX_CANCEL").unwrap() == "1";
    let collect = async {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
            if cancel {
                break;
            }
        }
        drop(rx);
        events
    };
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let (events, result) = tokio::join!(collect, send);
        json!({"ok": result.is_ok(), "error": result.err().map(|e| e.to_string()), "events": events})
    })
    .await
    .expect("Codex adapter hung");
    fs::write(
        PathBuf::from(root).join("result.json"),
        serde_json::to_vec(&result).unwrap(),
    )
    .unwrap();
}

fn assert_failure(result: &Value, expected: &str) {
    assert_eq!(result["ok"], false, "{result}");
    let events: Vec<StreamEvent> = serde_json::from_value(result["events"].clone()).unwrap();
    let terminal: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, StreamEvent::Done(_) | StreamEvent::Error(_)))
        .collect();
    assert_eq!(terminal.len(), 1, "{result}");
    assert!(
        matches!(terminal[0], StreamEvent::Error(message) if message.contains(expected)),
        "{result}"
    );
    assert!(result["error"].as_str().unwrap().contains(expected));
}

#[test]
fn current_codex_events_preserve_text_and_usage() {
    let result = run_fixture(
        Some(
            r#"
/bin/cat <<'JSONL'
{"type":"thread.started","thread_id":"fixture-thread"}
{"type":"turn.started"}
{"type":"item.started","item":{"id":"a","type":"agent_message","text":"partial"}}
{"type":"item.updated","item":{"id":"a","type":"agent_message","text":"partial update"}}
{"type":"item.completed","item":{"id":"r","type":"reasoning","text":"private reasoning"}}
{"type":"item.completed","item":{"id":"a","type":"agent_message","text":"Verified answer"}}
{"type":"turn.completed","usage":{"input_tokens":120,"cached_input_tokens":100,"output_tokens":12}}
JSONL
"#,
        ),
        false,
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["events"].as_array().unwrap().len(), 2, "{result}");
    assert_eq!(result["events"][0], json!({"Token": "Verified answer"}));
    assert_eq!(result["events"][1]["Done"]["input_tokens"], 120);
    assert_eq!(result["events"][1]["Done"]["output_tokens"], 12);
    assert_eq!(result["events"][1]["Done"]["cache_read_tokens"], 100);
}

#[test]
fn failed_turn_is_an_error_even_with_successful_exit() {
    let result = run_fixture(
        Some(r#"printf '%s\n' '{"type":"turn.failed","error":{"message":"quota exhausted"}}'"#),
        false,
    );
    assert_failure(&result, "quota exhausted");
}

#[test]
fn fatal_error_is_not_a_success() {
    let result = run_fixture(
        Some(r#"printf '%s\n' '{"type":"error","message":"authentication failed"}'; exit 1"#),
        false,
    );
    assert_failure(&result, "authentication failed");
}

#[test]
fn eof_without_completion_is_not_a_success() {
    assert_failure(&run_fixture(Some("exit 0"), false), "completion");
}

#[test]
fn nonzero_exit_overrides_completion() {
    let result = run_fixture(
        Some(r#"printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":7}}'; exit 9"#),
        false,
    );
    assert_failure(&result, "9");
}

#[test]
fn noisy_stderr_cannot_block_stdout() {
    let result = run_fixture(
        Some(
            r#"
i=0
while [ "$i" -lt 10000 ]; do
  printf '%s\n' 'provider diagnostic that must not enter the assistant transcript' >&2
  i=$((i + 1))
done
printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":9}}'
"#,
        ),
        false,
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["events"][0]["Done"]["input_tokens"], 9);
}

#[test]
fn legacy_events_remain_supported() {
    let result = run_fixture(
        Some(
            r#"
printf '%s\n' '{"type":"agent_message_delta","delta":"legacy "}' '{"type":"message_delta","delta":"answer"}' '{"type":"task_complete","usage":{"output_tokens":2}}'
"#,
        ),
        false,
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["events"][0], json!({"Token": "legacy "}));
    assert_eq!(result["events"][1], json!({"Token": "answer"}));
    assert_eq!(result["events"][2]["Done"]["output_tokens"], 2);
}

#[test]
fn spawn_failure_emits_a_terminal_error() {
    assert_failure(&run_fixture(None, false), "Codex");
}

#[test]
fn closed_consumer_stops_an_idle_provider() {
    let result = run_fixture(
        Some(
            r#"
printf '%s\n' '{"type":"agent_message","text":"started"}'
exec /bin/sleep 30
"#,
        ),
        true,
    );
    assert_eq!(result["ok"], true, "{result}");
}

#[test]
fn prompt_is_a_literal_argument() {
    let result = run_fixture(
        Some(
            r#"
[ "$1" = exec ] && [ "$2" = --json ] && [ "$3" = -m ] && [ "$4" = fixture-model ] && [ "$5" = -- ] && [ "$6" = --prompt-is-data ] || exit 23
printf '%s\n' '{"type":"turn.completed","usage":{"output_tokens":3}}'
"#,
        ),
        false,
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["events"][0]["Done"]["output_tokens"], 3, "{result}");
}

#[test]
fn malformed_json_cannot_be_hidden_by_a_later_completion() {
    let result = run_fixture(
        Some(
            r#"
printf '%s\n' 'private invalid payload' '{"type":"turn.completed","usage":{}}'
"#,
        ),
        false,
    );
    assert_failure(&result, "invalid Codex JSONL");
    assert!(!result.to_string().contains("private invalid payload"));
}

#[test]
fn invalid_utf8_emits_a_terminal_error() {
    let result = run_fixture(Some("printf '\\377\\n'"), false);
    assert_failure(&result, "could not read Codex output");
}

#[test]
fn provider_failure_overrides_an_earlier_completion() {
    let result = run_fixture(
        Some(
            r#"
printf '%s\n' '{"type":"turn.completed","usage":{}}' '{"type":"turn.failed","error":{"message":"late failure"}}'
"#,
        ),
        false,
    );
    assert_failure(&result, "late failure");
}

#[test]
fn completion_waits_for_provider_shutdown() {
    let result = run_fixture(
        Some(
            r#"
printf '%s\n' '{"type":"task_complete","usage":{}}'
exec 1>&-
/bin/sleep 0.1
exit 8
"#,
        ),
        false,
    );
    assert_failure(&result, "8");
}

#[test]
fn consumer_can_cancel_while_waiting_for_exit() {
    let result = run_fixture(
        Some(
            r#"
printf '%s\n' '{"type":"agent_message","text":"started"}'
exec 1>&-
exec /bin/sleep 30
"#,
        ),
        true,
    );
    assert_eq!(result["ok"], true, "{result}");
}
