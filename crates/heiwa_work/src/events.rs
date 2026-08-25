//! Turning Work facts into operator-domain events, and back.
//!
//! The envelope carries `work_id` so a reader can scope without parsing a
//! payload; the payload carries the fields only that event type has.

use heiwa_evidence::{
    OperatorActor, OperatorEvent, OperatorEventType, OperatorRisk, OperatorSensitivity,
    OPERATOR_EVENT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::model::WorkId;

/// How a thread's `work_id` was decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkLinkOrigin {
    /// The thread's own rows already carried one consistent, valid id.
    Adopted,
    /// No id existed anywhere in the thread's rows.
    Minted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkCreatedPayload {
    pub intent: String,
    pub origin_installation_id: String,
    pub primary_thread_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkLinkedPayload {
    pub thread_id: String,
    pub origin: WorkLinkOrigin,
}

impl WorkCreatedPayload {
    pub fn from_event(event: &OperatorEvent) -> Option<Self> {
        if event.event_type != OperatorEventType::WorkCreated {
            return None;
        }
        let payload: Self = serde_json::from_value(event.payload.clone()).ok()?;
        (payload.primary_thread_id == event.thread_id).then_some(payload)
    }
}

impl WorkLinkedPayload {
    pub fn from_event(event: &OperatorEvent) -> Option<Self> {
        if event.event_type != OperatorEventType::WorkLinked {
            return None;
        }
        let payload: Self = serde_json::from_value(event.payload.clone()).ok()?;
        (payload.thread_id == event.thread_id).then_some(payload)
    }
}

fn local_actor() -> OperatorActor {
    OperatorActor {
        kind: "user".to_string(),
        id: "local".to_string(),
    }
}

fn scoped(
    work_id: &WorkId,
    thread_id: &str,
    event_type: OperatorEventType,
    occurred_at: &str,
    payload: serde_json::Value,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    OperatorEvent {
        schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
        event_id: new_event_id(),
        thread_id: thread_id.to_string(),
        turn_id: None,
        run_id: None,
        call_id: None,
        work_id: Some(work_id.as_str().to_string()),
        event_type,
        occurred_at: occurred_at.to_string(),
        actor: local_actor(),
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: Vec::new(),
        evidence_refs: Vec::new(),
        payload,
    }
}

pub fn work_created_event(
    work_id: &WorkId,
    primary_thread_id: &str,
    intent: &str,
    origin_installation_id: &str,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::json!({
        "intent": intent,
        "origin_installation_id": origin_installation_id,
        "primary_thread_id": primary_thread_id,
    });
    scoped(
        work_id,
        primary_thread_id,
        OperatorEventType::WorkCreated,
        occurred_at,
        payload,
        new_event_id,
    )
}

pub fn work_linked_event(
    work_id: &WorkId,
    thread_id: &str,
    origin: WorkLinkOrigin,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::json!({
        "thread_id": thread_id,
        "origin": origin,
    });
    scoped(
        work_id,
        thread_id,
        OperatorEventType::WorkLinked,
        occurred_at,
        payload,
        new_event_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_created_event_names_its_work_in_the_envelope_not_only_the_payload() {
        let work_id = WorkId::generate(|| "abc".to_string());
        let event = work_created_event(
            &work_id,
            "thread-1",
            "prepare the release",
            "installation-1",
            "2026-08-22T00:00:00Z",
            || "evt-1".to_string(),
        );

        assert_eq!(event.work_id.as_deref(), Some("work-abc"));
        assert_eq!(event.thread_id, "thread-1");
        assert_eq!(event.event_type, OperatorEventType::WorkCreated);
        assert_eq!(event.sensitivity, OperatorSensitivity::LocalPrivate);

        let payload = WorkCreatedPayload::from_event(&event).expect("payload");
        assert_eq!(payload.intent, "prepare the release");
        assert_eq!(payload.origin_installation_id, "installation-1");
        assert_eq!(payload.primary_thread_id, "thread-1");
    }

    #[test]
    fn a_linked_event_records_whether_the_id_was_adopted_or_minted() {
        let work_id = WorkId::generate(|| "abc".to_string());
        let event = work_linked_event(
            &work_id,
            "thread-9",
            WorkLinkOrigin::Adopted,
            "2026-08-22T00:00:00Z",
            || "evt-2".to_string(),
        );

        let payload = WorkLinkedPayload::from_event(&event).expect("payload");
        assert_eq!(payload.origin, WorkLinkOrigin::Adopted);
        assert_eq!(payload.thread_id, "thread-9");
    }

    #[test]
    fn a_payload_from_the_wrong_event_type_is_refused() {
        let work_id = WorkId::generate(|| "abc".to_string());
        let created = work_created_event(
            &work_id,
            "thread-1",
            "intent",
            "installation-1",
            "2026-08-22T00:00:00Z",
            || "evt-1".to_string(),
        );
        assert!(
            WorkLinkedPayload::from_event(&created).is_none(),
            "reading a payload must check the event type, not just the shape"
        );
    }

    #[test]
    fn a_payload_cannot_rebind_an_event_to_another_thread() {
        let work_id = WorkId::generate(|| "abc".to_string());
        let mut created = work_created_event(
            &work_id,
            "thread-envelope",
            "intent",
            "installation-1",
            "2026-08-22T00:00:00Z",
            || "evt-1".to_string(),
        );
        created.payload["primary_thread_id"] = serde_json::json!("thread-payload");
        assert!(WorkCreatedPayload::from_event(&created).is_none());

        let mut linked = work_linked_event(
            &work_id,
            "thread-envelope",
            WorkLinkOrigin::Adopted,
            "2026-08-22T00:01:00Z",
            || "evt-2".to_string(),
        );
        linked.payload["thread_id"] = serde_json::json!("thread-payload");
        assert!(WorkLinkedPayload::from_event(&linked).is_none());
    }
}
