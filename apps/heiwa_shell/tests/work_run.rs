use std::path::Path;
use std::process::{Command, Output};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn heiwa(runtime_root: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_heiwa"))
        .env_clear()
        .env("HOME", runtime_root.parent().expect("runtime parent"))
        .env("HEIWA_HOME", runtime_root)
        .env("HEIWA_DISABLE_KEYCHAIN", "1")
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("run heiwa")
}

fn successful(output: Output, command: &str) -> Output {
    assert!(
        output.status.success(),
        "{command}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn work_run_json_stdout_is_exactly_one_json_document_even_when_child_writes_both_streams() {
    let fixture = tempfile::tempdir().expect("fixture");
    let runtime_root = fixture.path().join("runtime");
    let repo = fixture.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    git(&repo, &["init", "-q", "-b", "main", "."]);
    git(&repo, &["config", "user.email", "test@heiwa.ltd"]);
    git(&repo, &["config", "user.name", "Heiwa Test"]);
    std::fs::write(repo.join("tracked.txt"), "tracked\n").expect("fixture file");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "fixture"]);

    heiwa_identity::establish_in(
        &runtime_root,
        "Test operator",
        "2026-08-28T00:00:00Z",
        || "install-test".to_string(),
    )
    .expect("identity");

    let created = successful(
        heiwa(
            &runtime_root,
            &repo,
            &["work", "create", "json output", "--json"],
        ),
        "work create",
    );
    let created: serde_json::Value = serde_json::from_slice(&created.stdout).expect("create JSON");
    let work_id = created["work_id"].as_str().expect("work id");

    successful(
        heiwa(
            &runtime_root,
            &repo,
            &["workspace", "prepare", work_id, "--json"],
        ),
        "workspace prepare",
    );
    let run = successful(
        heiwa(
            &runtime_root,
            &repo,
            &[
                "work",
                "run",
                work_id,
                "--json",
                "--",
                "/bin/sh",
                "-c",
                "printf child-out; printf child-err >&2",
            ],
        ),
        "work run",
    );

    let outcome: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout must be one JSON document: {error}: {:?}",
            String::from_utf8_lossy(&run.stdout)
        )
    });
    assert_eq!(outcome["exit_code"], 0);
    let tail = outcome["pane_tail"].as_array().expect("pane tail");
    assert_eq!(tail.len(), 2);
    assert!(tail.contains(&serde_json::json!("child-out")));
    assert!(tail.contains(&serde_json::json!("child-err")));
}
