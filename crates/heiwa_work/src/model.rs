//! The aggregate itself.

use serde::{Deserialize, Serialize};

/// Schema version of the folded aggregate.
pub const SCHEMA_VERSION: u32 = 1;

const WORK_ID_PREFIX: &str = "work-";

/// A Work's stable primary identity, across threads, tasks, repositories,
/// provider sessions, nodes, and surfaces.
///
/// Prefixed so a journal line, a receipt, or a mesh envelope says what kind of
/// id it is holding. A thread id must never be accepted here: threads attach
/// to Work, they do not name it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkId(String);

impl WorkId {
    /// Mint a new id. The uuid source is injected so tests are reproducible.
    pub fn generate(new_uuid: impl FnOnce() -> String) -> Self {
        Self(format!("{WORK_ID_PREFIX}{}", new_uuid()))
    }

    /// Accept an existing id, or refuse it.
    pub fn parse(value: &str) -> Option<Self> {
        let rest = value.strip_prefix(WORK_ID_PREFIX)?;
        if rest.is_empty()
            || rest.len() > 128
            || !rest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return None;
        }
        Some(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a Work stands. Distinct from a Work Session's rendered phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Active,
    Blocked,
    Cancelled,
    Failed,
    Complete,
}

/// The durable coordination aggregate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    pub schema_version: u32,
    pub work_id: WorkId,
    /// Monotonic. Every mutating command supplies its expected revision, so a
    /// stale writer reloads or replans rather than overwriting.
    pub revision: u64,
    pub intent: String,
    pub status: WorkStatus,
    /// Bound to the installation before any node key exists. Work with no
    /// `origin_node` is local-only and refused at the replication boundary.
    pub origin_installation_id: String,
    pub origin_node: Option<String>,
    pub coordinator_node: Option<String>,
    /// V1 creates exactly one thread atomically with the Work.
    pub primary_thread_id: String,
    /// Review, handoff, or channel threads added later without changing
    /// `work_id`.
    pub related_thread_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Work {
    /// Whether this Work may cross the mesh replication boundary.
    pub fn is_replicable(&self) -> bool {
        self.origin_node.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_work_id_is_prefixed_so_it_is_never_mistaken_for_a_thread_id() {
        let id = WorkId::generate(|| "0192ac31-1f4e-7c9a-9d2b-5f6a7b8c9d0e".to_string());
        assert!(id.as_str().starts_with("work-"), "{id}");
        assert_eq!(id.as_str(), "work-0192ac31-1f4e-7c9a-9d2b-5f6a7b8c9d0e");
    }

    #[test]
    fn a_work_id_round_trips_as_a_bare_string() {
        let id = WorkId::generate(|| "abc".to_string());
        let json = serde_json::to_value(&id).expect("serialize");
        assert_eq!(json, "work-abc");
        let restored: WorkId = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, id);
    }

    #[test]
    fn a_parsed_work_id_refuses_an_unprefixed_string() {
        assert!(WorkId::parse("work-abc").is_some());
        assert!(
            WorkId::parse("thread-abc").is_none(),
            "a thread id must never silently become a work id"
        );
        assert!(WorkId::parse("").is_none());
    }

    #[test]
    fn a_work_id_is_safe_as_one_path_and_git_ref_component() {
        for invalid in [
            "work-../escape",
            "work-nested/value",
            "work-has space",
            "work-has\nnewline",
            "work-💥",
        ] {
            assert!(
                WorkId::parse(invalid).is_none(),
                "unsafe identity must be refused: {invalid:?}"
            );
        }
        assert!(WorkId::parse("work-abc_123-def").is_some());
    }

    fn work() -> Work {
        Work {
            schema_version: SCHEMA_VERSION,
            work_id: WorkId::generate(|| "abc".to_string()),
            revision: 1,
            intent: "prepare the release".to_string(),
            status: WorkStatus::Active,
            origin_installation_id: "installation-1".to_string(),
            origin_node: None,
            coordinator_node: None,
            primary_thread_id: "thread-1".to_string(),
            related_thread_ids: Vec::new(),
            created_at: "2026-08-22T00:00:00Z".to_string(),
            updated_at: "2026-08-22T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn unbound_work_is_not_replicable() {
        assert!(
            !work().is_replicable(),
            "work created before enrolment must never cross the mesh boundary"
        );
    }

    #[test]
    fn node_bound_work_is_replicable() {
        let mut bound = work();
        bound.origin_node = Some("sha256:ff".to_string());
        assert!(bound.is_replicable());
    }
}
