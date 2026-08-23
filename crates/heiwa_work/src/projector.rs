//! Folding operator events into Work.
//!
//! Read-only and replayable: the same events always produce the same
//! aggregate. Damage is counted rather than smoothed over, because a Work that
//! silently loses an event looks identical to one that never had it.

use std::collections::BTreeMap;

use heiwa_evidence::{OperatorEvent, OperatorEventType};

use crate::events::{WorkCreatedPayload, WorkLinkedPayload};
use crate::model::{Work, WorkId, WorkStatus, SCHEMA_VERSION};

/// Every Work visible in one stream, plus what could not be folded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkProjection {
    works: BTreeMap<String, Work>,
    /// Work-scoped events that could not be applied: an unknown Work, a
    /// malformed payload, or a duplicate creation.
    pub skipped_events: usize,
}

impl WorkProjection {
    pub fn work(&self, work_id: &str) -> Option<&Work> {
        self.works.get(work_id)
    }

    pub fn all(&self) -> impl Iterator<Item = &Work> {
        self.works.values()
    }

    pub fn len(&self) -> usize {
        self.works.len()
    }

    pub fn is_empty(&self) -> bool {
        self.works.is_empty()
    }
}

/// Fold an ordered slice of operator events into every Work they describe.
pub fn fold(events: &[OperatorEvent]) -> WorkProjection {
    let mut projection = WorkProjection::default();

    for event in events {
        // Events with no work_id belong to some other projector.
        let Some(raw_id) = event.work_id.as_deref() else {
            continue;
        };
        let Some(work_id) = WorkId::parse(raw_id) else {
            projection.skipped_events += 1;
            continue;
        };

        match event.event_type {
            OperatorEventType::WorkCreated => {
                let Some(payload) = WorkCreatedPayload::from_event(event) else {
                    projection.skipped_events += 1;
                    continue;
                };
                if projection.works.contains_key(work_id.as_str()) {
                    // A second creation for one id is a conflict, never a
                    // reset: the first creation already owns the identity.
                    projection.skipped_events += 1;
                    continue;
                }
                projection.works.insert(
                    work_id.as_str().to_string(),
                    Work {
                        schema_version: SCHEMA_VERSION,
                        work_id,
                        revision: 1,
                        intent: payload.intent,
                        status: WorkStatus::Active,
                        origin_installation_id: payload.origin_installation_id,
                        origin_node: None,
                        coordinator_node: None,
                        primary_thread_id: payload.primary_thread_id,
                        related_thread_ids: Vec::new(),
                        created_at: event.occurred_at.clone(),
                        updated_at: event.occurred_at.clone(),
                    },
                );
            }
            OperatorEventType::WorkLinked => {
                let Some(payload) = WorkLinkedPayload::from_event(event) else {
                    projection.skipped_events += 1;
                    continue;
                };
                let Some(work) = projection.works.get_mut(work_id.as_str()) else {
                    projection.skipped_events += 1;
                    continue;
                };
                if payload.thread_id != work.primary_thread_id
                    && !work.related_thread_ids.contains(&payload.thread_id)
                {
                    work.related_thread_ids.push(payload.thread_id);
                }
                work.revision += 1;
                work.updated_at = event.occurred_at.clone();
            }
            _ => {}
        }
    }

    projection
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{work_created_event, work_linked_event, WorkLinkOrigin};

    fn created() -> heiwa_evidence::OperatorEvent {
        work_created_event(
            &WorkId::generate(|| "abc".to_string()),
            "thread-1",
            "prepare the release",
            "installation-1",
            "2026-08-22T00:00:00Z",
            || "evt-1".to_string(),
        )
    }

    #[test]
    fn a_created_event_folds_into_one_work_at_revision_one() {
        let projection = fold(&[created()]);
        let work = projection.work("work-abc").expect("work");

        assert_eq!(work.revision, 1);
        assert_eq!(work.intent, "prepare the release");
        assert_eq!(work.status, WorkStatus::Active);
        assert_eq!(work.primary_thread_id, "thread-1");
        assert_eq!(work.origin_installation_id, "installation-1");
        assert!(work.origin_node.is_none());
        assert_eq!(projection.skipped_events, 0);
    }

    #[test]
    fn a_linked_thread_joins_the_related_list_without_replacing_the_primary() {
        let events = vec![
            created(),
            work_linked_event(
                &WorkId::parse("work-abc").expect("id"),
                "thread-9",
                WorkLinkOrigin::Adopted,
                "2026-08-22T00:01:00Z",
                || "evt-2".to_string(),
            ),
        ];
        let projection = fold(&events);
        let work = projection.work("work-abc").expect("work");

        assert_eq!(work.primary_thread_id, "thread-1");
        assert_eq!(work.related_thread_ids, vec!["thread-9".to_string()]);
        assert_eq!(
            work.revision, 2,
            "every accepted event advances the revision"
        );
        assert_eq!(work.updated_at, "2026-08-22T00:01:00Z");
    }

    #[test]
    fn linking_the_same_thread_twice_does_not_duplicate_it() {
        let link = |id: &str| {
            work_linked_event(
                &WorkId::parse("work-abc").expect("id"),
                "thread-9",
                WorkLinkOrigin::Adopted,
                "2026-08-22T00:01:00Z",
                || id.to_string(),
            )
        };
        let projection = fold(&[created(), link("evt-2"), link("evt-3")]);
        let work = projection.work("work-abc").expect("work");
        assert_eq!(work.related_thread_ids, vec!["thread-9".to_string()]);
    }

    #[test]
    fn an_event_for_an_unknown_work_is_counted_rather_than_inventing_one() {
        let orphan = work_linked_event(
            &WorkId::parse("work-missing").expect("id"),
            "thread-9",
            WorkLinkOrigin::Minted,
            "2026-08-22T00:01:00Z",
            || "evt-2".to_string(),
        );
        let projection = fold(&[orphan]);

        assert!(projection.work("work-missing").is_none());
        assert_eq!(
            projection.skipped_events, 1,
            "a link with no creation is damage to report, not a Work to fabricate"
        );
    }

    #[test]
    fn unrelated_operator_events_are_ignored_without_counting_as_damage() {
        let mut turn = created();
        turn.event_type = heiwa_evidence::OperatorEventType::UserMessage;
        turn.work_id = None;
        let projection = fold(&[created(), turn]);

        assert_eq!(projection.work("work-abc").expect("work").revision, 1);
        assert_eq!(
            projection.skipped_events, 0,
            "an unscoped event is not this projector's business"
        );
    }
}
