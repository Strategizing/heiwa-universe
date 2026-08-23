//! Deciding a thread's `work_id` when it is first promoted into Work.
//!
//! The order is normative, not advisory: minting while an adoptable id exists
//! orphans every row that already carries the old one, and nothing in the data
//! afterwards shows that it happened.

use std::collections::BTreeSet;

use crate::events::WorkLinkOrigin;
use crate::model::WorkId;

/// A thread that cannot be promoted without a human decision.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MigrationConflict {
    #[error(
        "thread carries {} distinct or unusable work ids ({}); resolve them before promoting it \
         — Heiwa will not merge them or mint a third",
        found.len(),
        found.join(", ")
    )]
    AmbiguousWorkId { found: Vec<String> },
}

/// What promoting a thread decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkIdResolution {
    pub work_id: WorkId,
    pub origin: WorkLinkOrigin,
}

/// Resolve the `work_id` for a thread from the ids its own rows already carry.
///
/// `existing_ids` is every `work_id` found on that thread's task, connector,
/// and evidence rows, in any order and with duplicates.
pub fn resolve_work_id(
    existing_ids: &[String],
    new_uuid: impl FnOnce() -> String,
) -> Result<WorkIdResolution, MigrationConflict> {
    let distinct: BTreeSet<&String> = existing_ids.iter().collect();

    // 1. Adopt.
    if !distinct.is_empty() {
        if distinct.len() == 1 {
            let only = distinct.iter().next().expect("one element");
            if let Some(work_id) = WorkId::parse(only) {
                return Ok(WorkIdResolution {
                    work_id,
                    origin: WorkLinkOrigin::Adopted,
                });
            }
        }
        return Err(MigrationConflict::AmbiguousWorkId {
            found: distinct.into_iter().cloned().collect(),
        });
    }

    // 2. Mint — only because nothing was adoptable.
    Ok(WorkIdResolution {
        work_id: WorkId::generate(new_uuid),
        origin: WorkLinkOrigin::Minted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_already_on_the_threads_rows_is_adopted() {
        let outcome = resolve_work_id(&["work-abc".to_string(), "work-abc".to_string()], || {
            "never".to_string()
        })
        .expect("consistent rows resolve");

        assert_eq!(outcome.work_id.as_str(), "work-abc");
        assert_eq!(outcome.origin, WorkLinkOrigin::Adopted);
    }

    #[test]
    fn an_id_is_minted_only_when_no_row_carries_one() {
        let outcome = resolve_work_id(&[], || "fresh".to_string()).expect("empty resolves");

        assert_eq!(outcome.work_id.as_str(), "work-fresh");
        assert_eq!(outcome.origin, WorkLinkOrigin::Minted);
    }

    #[test]
    fn adopting_takes_precedence_over_minting() {
        // The whole point of the rule: minting here would orphan the rows
        // that already carry work-abc.
        let outcome = resolve_work_id(&["work-abc".to_string()], || {
            panic!("must not mint while an adoptable id exists")
        })
        .expect("adoptable rows resolve");
        assert_eq!(outcome.work_id.as_str(), "work-abc");
    }

    #[test]
    fn conflicting_ids_on_one_thread_are_refused_not_merged() {
        let error = resolve_work_id(&["work-abc".to_string(), "work-def".to_string()], || {
            "fresh".to_string()
        })
        .expect_err("two ids on one thread is a conflict");

        let MigrationConflict::AmbiguousWorkId { found } = error;
        assert_eq!(found, vec!["work-abc".to_string(), "work-def".to_string()]);
    }

    #[test]
    fn a_malformed_id_is_a_conflict_rather_than_a_reason_to_mint() {
        let error = resolve_work_id(&["thread-abc".to_string()], || "fresh".to_string())
            .expect_err("an unparseable id must not be quietly replaced");

        let MigrationConflict::AmbiguousWorkId { found } = error;
        assert_eq!(found, vec!["thread-abc".to_string()]);
    }
}
