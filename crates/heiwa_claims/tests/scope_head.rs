//! Scope resolution is commit-bound, even when the Git index is dirty.

use std::fs;
use std::path::Path;
use std::process::Command;

use heiwa_claims::scope;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git command starts");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/tracked.txt"), "committed\n").unwrap();

    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "claims@example.test"]);
    git(root, &["config", "user.name", "Heiwa Claims Test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "baseline"]);
    dir
}

#[test]
fn staged_index_changes_do_not_change_head_scope() {
    for mutation in ["modification", "addition", "deletion"] {
        let dir = repository();
        let root = dir.path();
        let declared_scope = vec!["src".to_string()];
        let committed = scope::resolve(root, &declared_scope).expect("resolve clean HEAD");

        match mutation {
            "modification" => {
                fs::write(root.join("src/tracked.txt"), "staged modification\n").unwrap();
                git(root, &["add", "src/tracked.txt"]);
            }
            "addition" => {
                fs::write(root.join("src/staged-only.txt"), "staged addition\n").unwrap();
                git(root, &["add", "src/staged-only.txt"]);
            }
            "deletion" => git(root, &["rm", "-q", "src/tracked.txt"]),
            _ => unreachable!(),
        }

        let with_dirty_index =
            scope::resolve(root, &declared_scope).expect("resolve the unchanged HEAD");
        assert_eq!(
            with_dirty_index, committed,
            "staged {mutation} changed commit-bound scope"
        );
        assert_eq!(
            scope::digest(&with_dirty_index),
            scope::digest(&committed),
            "staged {mutation} changed commit-bound digest"
        );
    }
}
