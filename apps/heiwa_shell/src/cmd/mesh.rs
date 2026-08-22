//! `heiwa mesh` — this machine's node identity and who it is paired with.
//!
//! Read-only plus one explicit enrolment action. There is no transport yet
//! (D4 is unresolved), so a node established here has no peers and the command
//! says so plainly rather than implying a fabric exists.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use heiwa_mesh::keystore::{KeyStore, VaultKeyStore};
use heiwa_mesh::node::{self, NodeClass, NodeEnrolment, Platform};
use heiwa_mesh::peers;

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("status") | None => status(args),
        Some("enroll") => enroll(&args[1..]),
        Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(anyhow!("unknown mesh command: {other}")),
    }
}

fn print_help() {
    println!("heiwa mesh — node identity for this machine");
    println!();
    println!("  heiwa mesh status [--json]   what this node is, and who it is paired with");
    println!("  heiwa mesh enroll [--json]   mint this machine's node keypair (idempotent)");
    println!();
    println!("Peer pairing and replication are not built. An enrolled node here");
    println!("can sign and chain its own events; it cannot yet reach another device.");
}

fn status(args: &[String]) -> Result<()> {
    let root = crate::home::heiwa_runtime_dir();
    let summary = summarize(&root)?;
    if has_flag(args, "--json") {
        println!("{summary}");
        return Ok(());
    }
    print_summary(&summary);
    Ok(())
}

fn enroll(args: &[String]) -> Result<()> {
    let root = crate::home::heiwa_runtime_dir();
    let identity = heiwa_identity::load_from(&root)
        .map_err(|error| anyhow!("{error}"))?
        .ok_or_else(|| {
            anyhow!("no local identity on this installation; run first-run setup before enrolling")
        })?;
    let display_name = flag_value(args, "--name").unwrap_or_else(|| identity.display_name.clone());
    let node = establish(
        &root,
        &VaultKeyStore::new(),
        &identity.installation_id,
        &display_name,
    )
    .context("establish mesh node")?;

    let summary = summarize(&root)?;
    if has_flag(args, "--json") {
        println!("{summary}");
    } else {
        println!("enrolled this machine as a mesh node");
        println!("  node_id: {}", node.node_id);
        print_summary(&summary);
    }
    Ok(())
}

/// Mint or return this machine's node identity.
///
/// Split out so the enrolment path is exercised without the OS keychain.
pub(crate) fn establish(
    root: &Path,
    keys: &dyn KeyStore,
    installation_id: &str,
    display_name: &str,
) -> Result<heiwa_mesh::MeshNode> {
    let enrolled_at = chrono::Utc::now().to_rfc3339();
    node::establish_in(
        root,
        keys,
        &NodeEnrolment {
            installation_id,
            display_name,
            platform: Platform::host(),
            class: NodeClass::FullNode,
            enrolled_at: &enrolled_at,
        },
        node::random_seed,
    )
    .map_err(|error| anyhow!("{error}"))
}

/// The machine's mesh perspective, as a value both the CLI and the app read.
///
/// A registry that cannot be read is reported as unknown. Reading a locked or
/// corrupt file as "no peers" would turn a failure into a reassuring lie —
/// the failure mode AD-25 named in the connector plane.
pub(crate) fn summarize(root: &Path) -> Result<Value> {
    let mut errors: Vec<Value> = Vec::new();

    let node = match node::load_from(root) {
        Ok(Some(node)) => json!({
            "node_id": node.node_id.as_str(),
            "installation_id": node.installation_id,
            "display_name": node.display_name,
            "public_key": node.public_key,
            "platform": node.platform,
            "class": node.class,
            "background_reliability": node.background_reliability,
            "enrolled_at": node.enrolled_at,
        }),
        Ok(None) => Value::Null,
        Err(error) => {
            errors.push(json!({
                "code": "node_record_unreadable",
                "message": error.to_string(),
            }));
            Value::Null
        }
    };

    let peer_ids = match peers::load_from(root) {
        Ok(registry) => registry.active_peer_ids(),
        Err(error) => {
            errors.push(json!({
                "code": "peer_registry_unreadable",
                "message": error.to_string(),
            }));
            Vec::new()
        }
    };

    let sync_status = if !errors.is_empty() {
        "unknown"
    } else if peer_ids.is_empty() {
        "local_only"
    } else {
        "peer_enrolled"
    };

    let mut summary = json!({
        "node": node,
        "enrolled_peer_ids": peer_ids,
        "sync_status": sync_status,
        "transport": "not_configured",
    });
    if !errors.is_empty() {
        summary["errors"] = Value::Array(errors);
    }
    Ok(summary)
}

fn print_summary(summary: &Value) {
    match &summary["node"] {
        Value::Null => {
            println!("mesh node: not enrolled on this machine");
            println!("  run `heiwa mesh enroll` to mint this machine's node keypair");
        }
        node => {
            println!("mesh node: {}", node["node_id"].as_str().unwrap_or("?"));
            println!(
                "  installation: {}",
                node["installation_id"].as_str().unwrap_or("?")
            );
            println!(
                "  platform: {}   class: {}   background: {}",
                node["platform"].as_str().unwrap_or("?"),
                node["class"].as_str().unwrap_or("?"),
                node["background_reliability"].as_str().unwrap_or("?"),
            );
        }
    }

    let peers = summary["enrolled_peer_ids"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if peers.is_empty() {
        println!("  peers: none enrolled");
    } else {
        println!("  peers: {}", peers.len());
        for peer in peers {
            println!("    {}", peer.as_str().unwrap_or("?"));
        }
    }
    println!(
        "  sync: {}   transport: {}",
        summary["sync_status"].as_str().unwrap_or("?"),
        summary["transport"].as_str().unwrap_or("?"),
    );

    if let Some(errors) = summary["errors"].as_array() {
        for error in errors {
            println!(
                "  ! {}: {}",
                error["code"].as_str().unwrap_or("error"),
                error["message"].as_str().unwrap_or(""),
            );
        }
    }

    println!();
    println!("Peer pairing and replication are not built: this node cannot reach another device.");
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use heiwa_mesh::MemoryKeyStore;
    use std::fs;

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn an_unenrolled_machine_reports_no_node_and_no_peers() {
        let dir = root();
        let summary = summarize(dir.path()).expect("summarize");

        assert!(summary["node"].is_null(), "no node before enrolment");
        assert_eq!(
            summary["enrolled_peer_ids"].as_array().map(Vec::len),
            Some(0)
        );
        assert_eq!(summary["sync_status"], "local_only");
        assert_eq!(summary["transport"], "not_configured");
        assert!(summary.get("errors").is_none(), "{summary}");
    }

    #[test]
    fn an_enrolled_machine_reports_its_node_id() {
        let dir = root();
        let keys = MemoryKeyStore::new();
        let node = establish(dir.path(), &keys, "installation-1", "MacBook").expect("establish");

        let summary = summarize(dir.path()).expect("summarize");
        assert_eq!(summary["node"]["node_id"], node.node_id.as_str());
        assert_eq!(summary["node"]["installation_id"], "installation-1");
        assert_eq!(summary["node"]["background_reliability"], "continuous");
        assert_eq!(
            summary["sync_status"], "local_only",
            "one node is not a mesh"
        );
    }

    #[test]
    fn enrolling_twice_keeps_the_same_node_id() {
        let dir = root();
        let keys = MemoryKeyStore::new();
        let first = establish(dir.path(), &keys, "installation-1", "MacBook").expect("first");
        let second = establish(dir.path(), &keys, "installation-1", "Renamed").expect("second");
        assert_eq!(first.node_id, second.node_id);
    }

    #[test]
    fn a_registry_with_an_active_peer_reports_peer_enrolled() {
        let dir = root();
        fs::write(
            heiwa_mesh::peers::registry_path_in(dir.path()),
            serde_json::to_string_pretty(&json!({
                "version": heiwa_mesh::peers::SCHEMA_VERSION,
                "peers": [{
                    "node_id": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                    "display_name": "Windows desktop",
                    "public_key": "22",
                    "enrolled_at": "2026-08-22T00:00:00Z",
                    "revoked_at": null,
                }],
            }))
            .expect("serialize registry"),
        )
        .expect("write registry");

        let summary = summarize(dir.path()).expect("summarize");
        assert_eq!(summary["sync_status"], "peer_enrolled");
        assert_eq!(
            summary["enrolled_peer_ids"].as_array().map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn a_corrupt_peer_registry_is_reported_rather_than_read_as_no_peers() {
        let dir = root();
        fs::write(
            heiwa_mesh::peers::registry_path_in(dir.path()),
            "{ not json",
        )
        .expect("write corrupt registry");

        let summary = summarize(dir.path()).expect("summarize must still answer");
        assert_eq!(
            summary["sync_status"], "unknown",
            "an unreadable registry must never be rendered as 'no peers': {summary}"
        );
        let errors = summary["errors"].as_array().expect("errors: {summary}");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0]["code"], "peer_registry_unreadable");
    }

    #[test]
    fn a_future_node_schema_is_reported_rather_than_ignored() {
        let dir = root();
        let keys = MemoryKeyStore::new();
        establish(dir.path(), &keys, "installation-1", "MacBook").expect("establish");
        let path = heiwa_mesh::node::node_path_in(dir.path());
        let mut record: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
        record["version"] = json!(heiwa_mesh::node::SCHEMA_VERSION + 1);
        fs::write(
            &path,
            serde_json::to_string_pretty(&record).expect("serialize"),
        )
        .expect("write future record");

        let summary = summarize(dir.path()).expect("summarize must still answer");
        assert!(summary["node"].is_null());
        assert_eq!(summary["sync_status"], "unknown");
        let errors = summary["errors"].as_array().expect("errors: {summary}");
        assert_eq!(errors[0]["code"], "node_record_unreadable");
    }
}
