//! Sole-writer operator session service.
//!
//! Wraps a [`heiwa_evidence::OperatorJournal`] with the in-process
//! invariants a single runtime process must enforce on top of a dumb
//! append-only log: idempotent turn submission, event validation, and
//! deterministic materialization of threads/turns from the raw event
//! stream.
//!
//! The journal itself is the *only* durable truth. Nothing here caches
//! state across calls: writers take the service transaction mutex, fold
//! whatever is on disk right now, and append before releasing it. Reads
//! replay the journal without that mutex; [`OperatorJournal`] already owns
//! its append-side lock. That keeps writer read-modify-append sequences
//! atomic after a crash with no recovery step beyond
//! [`OperatorSessionService::recover_interrupted`], while retaining
//! lock-free read-only replay.
//!
//! Every service that mutates the stream holds a shared cross-process activity
//! lease for its remaining lifetime. Restart recovery temporarily requires an
//! exclusive activity lease, so it fails closed while any other writer may
//! still own live work. A separate root-wide transaction lock serializes each
//! materialize/validate/append sequence across processes; the journal's own
//! lock remains responsible only for one framed append.
//!
//! Cancellation contract: operator cancellation makes intent durable
//! FIRST — a `turn_cancel_requested` event is appended *before* the runner
//! is signalled — and the cancelled turn then terminates as
//! `turn_interrupted` with payload reason `OPERATOR_CANCELLED`.
//! `turn_cancel_requested` is therefore never terminal itself: it records
//! intent, and the follow-up `turn_interrupted` is the closing record —
//! the same closure shape [`OperatorSessionService::recover_interrupted`]
//! appends with reason `RUNTIME_RESTART`.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use heiwa_evidence::{
    find_sensitive, now_iso, CursorError, CursorEvent, OperatorActor, OperatorEvent,
    OperatorEventType, OperatorJournal, OperatorPage, OperatorRisk, OperatorSensitivity,
    OPERATOR_EVENT_SCHEMA_VERSION,
};

const OPERATOR_APP_RUNTIME_LEASE_FILE: &str = ".operator_runtime.lock";
const OPERATOR_ACTIVITY_LEASE_FILE: &str = ".operator_activity.lock";
const OPERATOR_TRANSACTION_LOCK_FILE: &str = ".operator_transaction.lock";

/// Typed cross-process ownership failures for operator session writers.
#[derive(Debug, thiserror::Error)]
pub enum OperatorOwnershipError {
    #[error("operator_runtime_lease_held: evidence root {root} already has a live app runtime")]
    RuntimeAlreadyHeld { root: PathBuf },
    #[error("operator_activity_lease_held: evidence root {root} has another live session writer")]
    ActivityAlreadyHeld { root: PathBuf },
    #[error("operator ownership lease storage error at {root}: {source}")]
    Storage {
        root: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Exclusive app-server ownership for one evidence root. The zero-content
/// sidecar stores no identity or auth data; the OS lock is the only state.
#[derive(Debug)]
pub struct OperatorAppRuntimeLease {
    _file: File,
}

impl OperatorAppRuntimeLease {
    pub fn acquire(root: impl AsRef<Path>) -> std::result::Result<Self, OperatorOwnershipError> {
        let root = root.as_ref().to_path_buf();
        let file = open_ownership_file(&root, OPERATOR_APP_RUNTIME_LEASE_FILE)?;
        file.try_lock().map_err(|source| match source {
            std::fs::TryLockError::WouldBlock => {
                OperatorOwnershipError::RuntimeAlreadyHeld { root: root.clone() }
            }
            std::fs::TryLockError::Error(source) => OperatorOwnershipError::Storage {
                root: root.clone(),
                source,
            },
        })?;
        Ok(Self { _file: file })
    }
}

#[derive(Debug)]
struct OperatorActivityLease {
    root: PathBuf,
    shared_file: Option<File>,
}

impl OperatorActivityLease {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            shared_file: None,
        }
    }

    fn ensure_shared(&mut self) -> std::result::Result<(), OperatorOwnershipError> {
        if self.shared_file.is_some() {
            return Ok(());
        }
        let file = open_ownership_file(&self.root, OPERATOR_ACTIVITY_LEASE_FILE)?;
        file.try_lock_shared().map_err(|source| match source {
            std::fs::TryLockError::WouldBlock => OperatorOwnershipError::ActivityAlreadyHeld {
                root: self.root.clone(),
            },
            std::fs::TryLockError::Error(source) => OperatorOwnershipError::Storage {
                root: self.root.clone(),
                source,
            },
        })?;
        self.shared_file = Some(file);
        Ok(())
    }

    fn with_exclusive<T>(&mut self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        let exclusive_file = open_ownership_file(&self.root, OPERATOR_ACTIVITY_LEASE_FILE)?;
        drop(self.shared_file.take());
        let acquired = exclusive_file.try_lock().map_err(|source| match source {
            std::fs::TryLockError::WouldBlock => OperatorOwnershipError::ActivityAlreadyHeld {
                root: self.root.clone(),
            },
            std::fs::TryLockError::Error(source) => OperatorOwnershipError::Storage {
                root: self.root.clone(),
                source,
            },
        });
        if let Err(error) = acquired {
            drop(exclusive_file);
            self.restore_shared()?;
            return Err(anyhow!(error));
        }

        let operation_result = operation();
        drop(exclusive_file);
        self.restore_shared()?;
        operation_result
    }

    fn restore_shared(&mut self) -> Result<()> {
        let file = open_ownership_file(&self.root, OPERATOR_ACTIVITY_LEASE_FILE)?;
        file.lock_shared().map_err(|source| {
            anyhow!(OperatorOwnershipError::Storage {
                root: self.root.clone(),
                source,
            })
        })?;
        self.shared_file = Some(file);
        Ok(())
    }
}

fn open_ownership_file(
    root: &Path,
    name: &str,
) -> std::result::Result<File, OperatorOwnershipError> {
    std::fs::create_dir_all(root).map_err(|source| OperatorOwnershipError::Storage {
        root: root.to_path_buf(),
        source,
    })?;
    // truncate(false): this is an ownership-lock file. Another process may
    // already hold it, and clobbering its contents on open would defeat the
    // lock. Stated explicitly so clippy::suspicious_open_options does not have
    // to infer intent from create+write.
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(root.join(name))
        .map_err(|source| OperatorOwnershipError::Storage {
            root: root.to_path_buf(),
            source,
        })
}

/// How a turn should be routed to a provider/model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    Auto,
    LocalOnly,
    RemoteOnly,
    Explicit,
}

/// Routing constraints attached to a single turn. The normalized policy is
/// carried inside the `turn_started` event payload so it is durable and
/// replayable, and echoed back on [`OperatorTurnView`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TurnRoutePolicy {
    pub mode: RouteMode,
    pub preferred_provider: Option<String>,
    pub preferred_model: Option<String>,
    pub allowed_models: Vec<String>,
    pub excluded_models: Vec<String>,
    pub minimum_quality_class: u8,
    pub maximum_marginal_cost_usd: Option<f64>,
    pub turn_budget_usd: Option<f64>,
    pub privacy: String,
}

impl TurnRoutePolicy {
    /// Canonicalize set-like and operator-entered fields before durable
    /// binding. Model allow/exclude order has no routing meaning, while
    /// provider/model/privacy whitespace and privacy casing must not turn a
    /// safe retry into a different request.
    pub fn normalized(&self) -> Self {
        fn normalized_optional(value: &Option<String>) -> Option<String> {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        }

        fn normalized_models(values: &[String]) -> Vec<String> {
            let mut values = values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            values.sort_unstable();
            values.dedup();
            values
        }

        Self {
            mode: self.mode.clone(),
            preferred_provider: normalized_optional(&self.preferred_provider),
            preferred_model: normalized_optional(&self.preferred_model),
            allowed_models: normalized_models(&self.allowed_models),
            excluded_models: normalized_models(&self.excluded_models),
            minimum_quality_class: self.minimum_quality_class,
            maximum_marginal_cost_usd: self.maximum_marginal_cost_usd.map(|value| {
                if value == 0.0 {
                    0.0
                } else {
                    value
                }
            }),
            turn_budget_usd: self
                .turn_budget_usd
                .map(|value| if value == 0.0 { 0.0 } else { value }),
            privacy: self.privacy.trim().to_ascii_lowercase(),
        }
    }
}

/// A caller's request to start (or resubmit) one turn.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct StartTurnRequest {
    pub client_request_id: String,
    pub prompt: String,
    pub route_policy: TurnRoutePolicy,
    /// Durable Work scope for this turn. Omitted for legacy/system turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_id: Option<String>,
}

impl StartTurnRequest {
    /// Convenience constructor for the common case: automatic routing, no
    /// caller-specified spending ceiling, standard privacy tier, and the
    /// lowest quality floor (`1`, i.e. no floor beyond "works").
    pub fn auto(client_request_id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            client_request_id: client_request_id.into(),
            prompt: prompt.into(),
            route_policy: TurnRoutePolicy {
                mode: RouteMode::Auto,
                preferred_provider: None,
                preferred_model: None,
                allowed_models: Vec::new(),
                excluded_models: Vec::new(),
                minimum_quality_class: 1,
                maximum_marginal_cost_usd: None,
                turn_budget_usd: None,
                privacy: "standard".to_string(),
            },
            work_id: None,
        }
    }
}

/// Result of [`OperatorSessionService::start_turn`].
///
/// `cursor` is the cursor positioned immediately after the turn's
/// `user_message` append — i.e. the boundary a caller should hand to
/// [`OperatorSessionService::events_after`] to observe exactly that turn's
/// execution events (route/tool/assistant/completion) without replaying the
/// submission itself. For a duplicate submission this is the *stored*
/// cursor recorded when the original `user_message` was appended, which is
/// why materialization retains a per-turn `user_message` cursor: it is the
/// only way to serve this idempotently without re-appending anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSubmission {
    pub thread_id: String,
    pub turn_id: String,
    pub work_id: Option<String>,
    pub cursor: String,
    pub duplicate: bool,
}

/// Typed admission failures for [`OperatorSessionService::start_turn`].
/// Public callers can map operator mistakes without parsing display text;
/// runtime/storage sources remain available internally without being exposed
/// through the HTTP contract.
#[derive(Debug, thiserror::Error)]
pub enum TurnSubmissionError {
    #[error("idempotency conflict for turn {turn_id}: {reason}")]
    IdempotencyConflict {
        turn_id: String,
        reason: &'static str,
    },
    #[error("refused to start turn: {context} contains sensitive material")]
    SensitiveMaterial { context: &'static str },
    #[error(
        "refused to start turn: Work {work_id} is unknown or is not linked to thread {thread_id}"
    )]
    InvalidWorkScope { work_id: String, thread_id: String },
    #[error(transparent)]
    Runtime(#[from] anyhow::Error),
}

/// Materialized view of one turn, folded from the operator event stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperatorTurnView {
    pub turn_id: String,
    pub client_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_id: Option<String>,
    /// One of `"open"`, `"completed"`, `"interrupted"`, `"blocked"`. The
    /// latter three are terminal (see [`is_turn_terminal`]).
    pub status: String,
    pub prompt: Option<String>,
    /// Cursor positioned immediately after this turn's `user_message`
    /// append. `None` only if materialization somehow never saw a
    /// `user_message` for a `turn_started` it did see (should not happen
    /// via `start_turn`, but replay stays defensive about it).
    pub user_message_cursor: Option<String>,
    pub started_at: String,
    pub route_policy: Option<TurnRoutePolicy>,
}

/// Materialized view of one thread: its turns in creation order, plus two
/// tolerance counters — one per damage domain — for anything that could
/// not be projected. Neither counter ever fails a read (write-side is
/// strict via [`OperatorSessionService::append_event`], read-side is
/// tolerant).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperatorThreadView {
    pub thread_id: String,
    pub turns: Vec<OperatorTurnView>,
    /// Schema/state-level rejects: events whose line parsed fine but that
    /// could not be projected — unsupported schema versions, or
    /// nonterminal events addressed to a turn that was already terminal.
    /// Thread-scoped, because a parsed event carries its `thread_id`.
    pub skipped_events: usize,
    /// Journal-level damage: lines in the underlying stream that never
    /// parsed as operator events at all — torn tails, garbage bytes, or
    /// envelopes whose `event_type` string is unknown to this build. A
    /// damaged line has no readable `thread_id`, so this count is
    /// stream-wide, not thread-scoped: every thread view (including views
    /// of threads with no events yet) reports the same number.
    pub skipped_lines: usize,
}

/// Lightweight summary of one thread for [`OperatorSessionService::list_threads`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperatorThreadSummary {
    pub thread_id: String,
    pub turn_count: usize,
    pub latest_turn_id: Option<String>,
    pub latest_status: Option<String>,
}

/// Sole-writer service over one [`OperatorJournal`]. See the module docs for
/// the durability and concurrency contract.
#[derive(Debug)]
pub struct OperatorSessionService {
    journal: OperatorJournal,
    activity_lease: Mutex<OperatorActivityLease>,
    /// Serializes in-process read-modify-append writer transactions. It is
    /// deliberately separate from the journal's own append lock, so replay
    /// and materialization never wait behind a slow writer.
    write_transaction: Mutex<()>,
    /// Disposable cursor-caught-up fold. JSONL remains authority; this state
    /// can always reset and rebuild after cursor lineage changes.
    projection: Mutex<MaterializedJournal>,
}

struct OperatorWriteTransaction<'a> {
    // Release the root-wide lock before the in-process mutex.
    _cross_process: File,
    _local: std::sync::MutexGuard<'a, ()>,
}

impl OperatorSessionService {
    pub fn new(journal: OperatorJournal) -> Self {
        let activity_lease = OperatorActivityLease::new(journal.root().to_path_buf());
        Self {
            journal,
            activity_lease: Mutex::new(activity_lease),
            write_transaction: Mutex::new(()),
            projection: Mutex::new(MaterializedJournal::default()),
        }
    }

    /// Durably create an empty operator thread if it does not already exist.
    ///
    /// Returns `true` only when this call appended `thread_created`. The
    /// read-check-append sequence shares the sole-writer transaction lock
    /// with turn submission, so concurrent create/submit calls cannot append
    /// duplicate lifecycle rows inside one runtime process.
    pub fn ensure_thread(&self, thread_id: &str) -> Result<bool> {
        let _write_transaction = self.lock_writer_transaction()?;
        let projection = self.materialized()?;
        if projection.threads.contains_key(thread_id) {
            return Ok(false);
        }
        let event = new_event(
            thread_id,
            None,
            None,
            OperatorEventType::ThreadCreated,
            now_iso(),
            OperatorActor {
                kind: "operator".to_string(),
                id: "local-operator".to_string(),
            },
            json!({}),
        );
        self.journal.append(&event)?;
        Ok(true)
    }

    /// Start a turn, or return the existing one if `request.client_request_id`
    /// already has a matching `turn_started` in this thread.
    ///
    /// Holds the service mutex for the whole read-modify-write: materialize
    /// the thread, check for a duplicate, and (if none) append
    /// `thread_created` (only the first time a thread is used), then
    /// `turn_started`, then `user_message`. Every prospective payload is
    /// screened before the first append. Idempotency is decided purely by
    /// what is already on disk, per the module docs.
    pub fn start_turn(
        &self,
        thread_id: &str,
        request: StartTurnRequest,
    ) -> std::result::Result<TurnSubmission, TurnSubmissionError> {
        // Screen every payload this submission could append with the exact
        // evidence-journal gate before any write. Otherwise a rejected
        // later payload could leave durable scaffolding behind it.
        let thread_created_payload = json!({});
        let user_message_payload = json!({ "text": request.prompt });
        let prompt_fingerprint = prompt_fingerprint(&request.prompt);
        let normalized_route_policy = request.route_policy.normalized();
        let turn_started_payload = json!({
            "client_request_id": request.client_request_id.clone(),
            "prompt_fingerprint": prompt_fingerprint,
            "route_policy": normalized_route_policy.clone(),
        });
        for (event_type, payload) in [
            ("thread_created", &thread_created_payload),
            ("turn_started", &turn_started_payload),
            ("user_message", &user_message_payload),
        ] {
            if find_sensitive(payload).is_some() {
                return Err(TurnSubmissionError::SensitiveMaterial {
                    context: event_type,
                });
            }
        }

        let _write_transaction = self.lock_writer_transaction()?;
        let projection = self.materialized()?;
        let threads = &projection.threads;

        if let Some(folded) = threads.get(thread_id) {
            if let Some(turn) = folded.turns.iter().find(|turn| {
                turn.client_request_id.as_deref() == Some(request.client_request_id.as_str())
            }) {
                validate_retry_binding(
                    turn,
                    &prompt_fingerprint,
                    &normalized_route_policy,
                    request.work_id.as_deref(),
                )?;
                if let Some(cursor) = &turn.user_message_cursor {
                    return Ok(TurnSubmission {
                        thread_id: thread_id.to_string(),
                        turn_id: turn.turn_id.clone(),
                        work_id: turn.work_id.clone(),
                        cursor: cursor.clone(),
                        duplicate: true,
                    });
                }

                if turn.status != "open" || turn.cancel_requested {
                    return Err(TurnSubmissionError::IdempotencyConflict {
                        turn_id: turn.turn_id.clone(),
                        reason: "turn is no longer accepting a recovered user message",
                    });
                }

                // A prior crash/error after `turn_started` but before the
                // user message is recoverable only after the safe prompt
                // fingerprint check above. Append just the missing record.
                let mut recovered = new_event(
                    thread_id,
                    Some(turn.turn_id.clone()),
                    None,
                    OperatorEventType::UserMessage,
                    now_iso(),
                    OperatorActor {
                        kind: "operator".to_string(),
                        id: "local-operator".to_string(),
                    },
                    user_message_payload,
                );
                recovered.work_id = turn.work_id.clone();
                let appended = self.journal.append(&recovered)?;
                return Ok(TurnSubmission {
                    thread_id: thread_id.to_string(),
                    turn_id: turn.turn_id.clone(),
                    work_id: turn.work_id.clone(),
                    cursor: appended.cursor,
                    duplicate: true,
                });
            }
        }

        if let Some(work_id) = request.work_id.as_deref() {
            let linked = projection
                .work_threads
                .get(work_id)
                .is_some_and(|threads| threads.contains(thread_id));
            if !linked {
                return Err(TurnSubmissionError::InvalidWorkScope {
                    work_id: work_id.to_string(),
                    thread_id: thread_id.to_string(),
                });
            }
        }

        let thread_exists = threads.contains_key(thread_id);
        let now = now_iso();
        let operator_actor = OperatorActor {
            kind: "operator".to_string(),
            id: "local-operator".to_string(),
        };

        if !thread_exists {
            let created = new_event(
                thread_id,
                None,
                None,
                OperatorEventType::ThreadCreated,
                now.clone(),
                operator_actor.clone(),
                thread_created_payload,
            );
            self.journal.append(&created)?;
        }

        let turn_id = deterministic_turn_id(thread_id, &request.client_request_id);

        let mut turn_started = new_event(
            thread_id,
            Some(turn_id.clone()),
            None,
            OperatorEventType::TurnStarted,
            now.clone(),
            operator_actor.clone(),
            turn_started_payload,
        );
        turn_started.work_id = request.work_id.clone();
        self.journal.append(&turn_started)?;

        let mut user_message = new_event(
            thread_id,
            Some(turn_id.clone()),
            None,
            OperatorEventType::UserMessage,
            now,
            operator_actor,
            user_message_payload,
        );
        user_message.work_id = request.work_id.clone();
        let appended = self.journal.append(&user_message)?;

        Ok(TurnSubmission {
            thread_id: thread_id.to_string(),
            turn_id,
            work_id: request.work_id,
            cursor: appended.cursor,
            duplicate: false,
        })
    }

    /// Append one caller-constructed event after validating it. See
    /// [`validate_event`] for the exact rules. Write-side is strict: a
    /// rejected event never reaches the journal.
    pub fn append_event(&self, event: OperatorEvent) -> Result<CursorEvent> {
        let _write_transaction = self.lock_writer_transaction()?;
        let projection = self.materialized()?;
        validate_event(&projection.threads, &projection.work_threads, &event)?;
        self.journal.append(&event)
    }

    /// Replay events for one thread starting strictly after `cursor`.
    ///
    /// Reads the underlying journal in fixed-size pages (bounded, not
    /// `usize::MAX`) and filters each page down to rows matching
    /// `thread_id`, advancing the cursor across nonmatching rows from other
    /// threads so a caller polling this thread's cursor neither misses a
    /// later matching row nor rereads unrelated rows on every poll. Stops
    /// as soon as `limit` matching events have been collected or the
    /// journal is exhausted.
    pub fn events_after(
        &self,
        thread_id: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> std::result::Result<OperatorPage, CursorError> {
        const PAGE_SIZE: usize = 256;

        let mut collected: Vec<CursorEvent> = Vec::new();
        let mut skipped_lines = 0usize;
        let mut raw_cursor: Option<String> = cursor.map(str::to_string);
        let mut next_cursor: Option<String> = raw_cursor.clone();

        if limit == 0 {
            return Ok(OperatorPage {
                events: collected,
                next_cursor,
                skipped_lines,
            });
        }

        loop {
            let page = self.journal.read_after(raw_cursor.as_deref(), PAGE_SIZE)?;
            if page.events.is_empty() {
                // EOF (or a stalled torn tail): fully consumed, safe to fold
                // in its skip count. next_cursor echoes the input per the
                // journal's own contract.
                skipped_lines += page.skipped_lines;
                next_cursor = page.next_cursor;
                break;
            }

            let mut reached_limit_at: Option<usize> = None;
            for (index, row) in page.events.iter().enumerate() {
                if row.event.thread_id == thread_id {
                    collected.push(row.clone());
                    if collected.len() == limit {
                        reached_limit_at = Some(index);
                        break;
                    }
                }
            }

            if let Some(index) = reached_limit_at {
                // Stop exactly at the limit-th match's own cursor, not at
                // the raw page boundary, so a resumed read never skips
                // matching rows that happened to share this page.
                next_cursor = Some(page.events[index].cursor.clone());
                if index == page.events.len() - 1 {
                    skipped_lines += page.skipped_lines;
                }
                break;
            }

            // Whole page scanned without reaching the limit: every row in
            // it (matching or not) has now been yielded to this call, so
            // its skip count is safe to fold in and we can advance past it
            // entirely. `next_cursor` is left alone here — it is only ever
            // read after a `break`, and both break paths set it themselves,
            // so updating it on this non-terminal path would just be a
            // dead store.
            skipped_lines += page.skipped_lines;
            raw_cursor = page.next_cursor;
        }

        Ok(OperatorPage {
            events: collected,
            next_cursor,
            skipped_lines,
        })
    }

    /// Crate-internal global page for rebuildable projections. Domain clients
    /// use thread-filtered `events_after`; index rebuilds need one bounded
    /// append-order pass across the canonical stream.
    pub(crate) fn journal_page_after(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> std::result::Result<OperatorPage, CursorError> {
        self.journal.read_after(cursor, limit)
    }

    /// Scan the operator journal once for durable artifact links.
    ///
    /// Artifact reconciliation deliberately asks the journal rather than
    /// trusting a filesystem marker: the event stream is the sole authority
    /// for whether raw output may survive a restart. Pages are bounded so a
    /// large history never requires one unbounded journal read.
    pub fn artifact_links(&self) -> Result<HashSet<(String, String)>> {
        const PAGE_SIZE: usize = 256;

        let mut links = HashSet::new();
        let mut cursor = None;
        loop {
            let page = self.journal.read_after(cursor.as_deref(), PAGE_SIZE)?;
            for row in &page.events {
                if row.event.event_type != OperatorEventType::ArtifactCreated {
                    continue;
                }
                if let Some(artifact_id) = row
                    .event
                    .payload
                    .get("artifact_id")
                    .and_then(|id| id.as_str())
                {
                    links.insert((row.event.thread_id.clone(), artifact_id.to_string()));
                }
            }
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            if page.events.is_empty() || cursor.as_deref() == Some(next_cursor.as_str()) {
                break;
            }
            cursor = Some(next_cursor);
        }
        Ok(links)
    }

    /// Return whether the durable journal has linked `artifact_id` to this
    /// thread. Kept as a narrow convenience wrapper for callers that need
    /// one lookup; batch recovery should use [`Self::artifact_links`].
    pub fn has_artifact_link(&self, thread_id: &str, artifact_id: &str) -> Result<bool> {
        Ok(self
            .artifact_links()?
            .contains(&(thread_id.to_string(), artifact_id.to_string())))
    }

    /// Materialized view of one thread. Threads with no events yet return
    /// an empty view rather than an error. `skipped_lines` on the view is
    /// stream-wide journal damage (see [`OperatorThreadView::skipped_lines`])
    /// and is reported even on the empty-thread branch.
    pub fn thread(&self, thread_id: &str) -> Result<OperatorThreadView> {
        let materialized = self.materialized()?;
        Ok(match materialized.threads.get(thread_id) {
            Some(folded) => folded.to_view(
                materialized.skipped_lines(),
                materialized
                    .unsupported_schema_events
                    .get(thread_id)
                    .copied()
                    .unwrap_or(0)
                    + materialized
                        .rejected_current_schema_events
                        .get(thread_id)
                        .copied()
                        .unwrap_or(0),
            ),
            None => OperatorThreadView {
                thread_id: thread_id.to_string(),
                turns: Vec::new(),
                skipped_events: materialized
                    .unsupported_schema_events
                    .get(thread_id)
                    .copied()
                    .unwrap_or(0)
                    + materialized
                        .rejected_current_schema_events
                        .get(thread_id)
                        .copied()
                        .unwrap_or(0),
                skipped_lines: materialized.skipped_lines(),
            },
        })
    }

    /// Summaries of the most recently active threads, most recent first,
    /// bounded to `limit`.
    pub fn list_threads(&self, limit: usize) -> Result<Vec<OperatorThreadSummary>> {
        let materialized = self.materialized()?;
        let mut folded: Vec<&FoldedThread> = materialized.threads.values().collect();
        folded.sort_by_key(|thread| std::cmp::Reverse(thread.last_order));
        Ok(folded
            .into_iter()
            .take(limit)
            .map(FoldedThread::to_summary)
            .collect())
    }

    /// Close out every nonterminal turn with a `turn_interrupted` event, as
    /// if the runtime had just restarted. Pending cancellation closes with
    /// `OPERATOR_CANCELLED`; every other open turn closes with
    /// `RUNTIME_RESTART`.
    /// Idempotent: once every turn is terminal, subsequent calls return
    /// `0` and append nothing.
    pub fn recover_interrupted(&self) -> Result<usize> {
        let _write_transaction = self.lock_write_transaction()?;
        let mut activity_lease = self
            .activity_lease
            .lock()
            .map_err(|_| anyhow!("operator activity lease mutex poisoned"))?;
        activity_lease.with_exclusive(|| {
            let _cross_process_transaction = self.lock_cross_process_transaction()?;
            let projection = self.materialized()?;
            let threads = &projection.threads;
            let runtime_actor = OperatorActor {
                kind: "runtime".to_string(),
                id: "heiwa-core".to_string(),
            };

            let mut closed = 0usize;
            for (thread_id, folded) in threads {
                for turn in &folded.turns {
                    if is_turn_terminal(&turn.status) {
                        continue;
                    }
                    let event = new_event(
                        thread_id,
                        Some(turn.turn_id.clone()),
                        None,
                        OperatorEventType::TurnInterrupted,
                        now_iso(),
                        runtime_actor.clone(),
                        json!({
                            "reason": if turn.cancel_requested {
                                "OPERATOR_CANCELLED"
                            } else {
                                "RUNTIME_RESTART"
                            }
                        }),
                    );
                    self.journal.append(&event)?;
                    closed += 1;
                }
            }
            Ok(closed)
        })
    }

    /// Close one orphan after the caller has independently proved ownership
    /// of the failed runner. This service cannot establish that proof from
    /// journal state alone; callers must never use process-local absence as
    /// evidence that another runtime is dead.
    pub fn recover_proven_orphan(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<Option<CursorEvent>> {
        let _write_transaction = self.lock_writer_transaction()?;
        let projection = self.materialized()?;
        let Some(turn) = projection
            .threads
            .get(thread_id)
            .and_then(|thread| thread.turns.iter().find(|turn| turn.turn_id == turn_id))
        else {
            bail!("cannot recover unknown turn {turn_id} in thread {thread_id}");
        };
        if is_turn_terminal(&turn.status) {
            return Ok(None);
        }
        let event = new_event(
            thread_id,
            Some(turn_id.to_string()),
            None,
            OperatorEventType::TurnInterrupted,
            now_iso(),
            OperatorActor {
                kind: "runtime".to_string(),
                id: "operator-session-recovery".to_string(),
            },
            json!({
                "reason": if turn.cancel_requested {
                    "OPERATOR_CANCELLED"
                } else {
                    "RUNTIME_RESTART"
                }
            }),
        );
        Ok(Some(self.journal.append(&event)?))
    }

    fn lock_write_transaction(&self) -> Result<std::sync::MutexGuard<'_, ()>> {
        self.write_transaction
            .lock()
            .map_err(|_| anyhow!("operator session write transaction mutex poisoned"))
    }

    fn lock_cross_process_transaction(&self) -> Result<File> {
        let root = self.journal.root().to_path_buf();
        let file = open_ownership_file(&root, OPERATOR_TRANSACTION_LOCK_FILE)
            .map_err(anyhow::Error::new)?;
        file.lock()
            .map_err(|source| anyhow!(OperatorOwnershipError::Storage { root, source }))?;
        Ok(file)
    }

    fn lock_writer_transaction(&self) -> Result<OperatorWriteTransaction<'_>> {
        let local = self.lock_write_transaction()?;
        self.activity_lease
            .lock()
            .map_err(|_| anyhow!("operator activity lease mutex poisoned"))?
            .ensure_shared()
            .map_err(anyhow::Error::new)?;
        let cross_process = self.lock_cross_process_transaction()?;
        Ok(OperatorWriteTransaction {
            _cross_process: cross_process,
            _local: local,
        })
    }

    fn materialized(&self) -> Result<std::sync::MutexGuard<'_, MaterializedJournal>> {
        let mut projection = self
            .projection
            .lock()
            .map_err(|_| anyhow!("operator materialized projection mutex poisoned"))?;
        sync_materialized(&self.journal, &mut projection)?;
        Ok(projection)
    }
}

// ---------------------------------------------------------------------
// Event construction.
// ---------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn new_event(
    thread_id: &str,
    turn_id: Option<String>,
    call_id: Option<String>,
    event_type: OperatorEventType,
    occurred_at: String,
    actor: OperatorActor,
    payload: serde_json::Value,
) -> OperatorEvent {
    OperatorEvent {
        schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
        event_id: Uuid::new_v4().to_string(),
        thread_id: thread_id.to_string(),
        turn_id,
        run_id: None,
        call_id,
        work_id: None,
        event_type,
        occurred_at,
        actor,
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: vec![],
        evidence_refs: vec![],
        payload,
    }
}

/// Deterministic turn id derived from `(thread_id, client_request_id)`.
///
/// Idempotency is decided by materializing the journal and matching on the
/// stored `client_request_id` (see [`OperatorSessionService::start_turn`]);
/// this determinism is a defense-in-depth property on top of that, not a
/// substitute for it. It gives the same `client_request_id` resubmitted
/// against the same thread a stable, reproducible turn id even if the
/// journal were ever inspected out of band, and makes the id collision-free
/// across threads without a shared counter.
fn deterministic_turn_id(thread_id: &str, client_request_id: &str) -> String {
    let namespace = operator_turn_namespace();
    // A unit separator can't appear in either input via normal usage and
    // keeps `("a", "b:c")` distinct from `("a:b", "c")`.
    let name = format!("{thread_id}\u{1f}{client_request_id}");
    Uuid::new_v5(&namespace, name.as_bytes()).to_string()
}

/// Stable, non-plaintext binding between an idempotency key and the prompt
/// it was first submitted with. This lets a retry repair a crash between
/// `turn_started` and `user_message` without treating a changed prompt as
/// the same turn.
fn prompt_fingerprint(prompt: &str) -> String {
    let digest = Sha256::digest(prompt.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_retry_binding(
    turn: &FoldedTurn,
    retry_fingerprint: &str,
    retry_route_policy: &TurnRoutePolicy,
    retry_work_id: Option<&str>,
) -> std::result::Result<(), TurnSubmissionError> {
    let stored_fingerprint = turn
        .prompt_fingerprint
        .clone()
        .or_else(|| turn.prompt.as_deref().map(prompt_fingerprint));
    let Some(stored_fingerprint) = stored_fingerprint else {
        return Err(TurnSubmissionError::IdempotencyConflict {
            turn_id: turn.turn_id.clone(),
            reason: "no durable prompt binding is available",
        });
    };
    if stored_fingerprint != retry_fingerprint {
        return Err(TurnSubmissionError::IdempotencyConflict {
            turn_id: turn.turn_id.clone(),
            reason: "client_request_id was previously submitted with a different prompt",
        });
    }
    let Some(stored_route_policy) = turn.route_policy.as_ref() else {
        return Err(TurnSubmissionError::IdempotencyConflict {
            turn_id: turn.turn_id.clone(),
            reason: "no durable route policy binding is available",
        });
    };
    if stored_route_policy.normalized() != *retry_route_policy {
        return Err(TurnSubmissionError::IdempotencyConflict {
            turn_id: turn.turn_id.clone(),
            reason: "client_request_id was previously submitted with a different route policy",
        });
    }
    if turn.work_id.as_deref() != retry_work_id {
        return Err(TurnSubmissionError::IdempotencyConflict {
            turn_id: turn.turn_id.clone(),
            reason: "client_request_id was previously submitted for a different Work",
        });
    }
    Ok(())
}

/// Fixed namespace UUID for turn-id derivation, itself derived (once per
/// call; cheap) from a SHA-256 of a stable domain string rather than a
/// hand-picked literal, so it is reproducible from source alone.
fn operator_turn_namespace() -> Uuid {
    let digest = Sha256::digest(b"heiwa.operator.turn.v1");
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

// ---------------------------------------------------------------------
// Validation (write-side, strict).
// ---------------------------------------------------------------------

/// Validate one event against the current materialized state before it is
/// allowed to reach the journal. `event_type` itself is always "known" by
/// construction (it is a closed Rust enum), so there is nothing to check
/// there beyond what the type system already guarantees; this function
/// covers schema version, required identifiers, and terminal-state
/// transitions.
fn validate_event(
    threads: &HashMap<String, FoldedThread>,
    work_threads: &HashMap<String, HashSet<String>>,
    event: &OperatorEvent,
) -> Result<()> {
    if event.schema_version != OPERATOR_EVENT_SCHEMA_VERSION {
        bail!(
            "rejected operator event {}: unsupported schema_version {} (expected {OPERATOR_EVENT_SCHEMA_VERSION})",
            event.event_id,
            event.schema_version
        );
    }

    if requires_turn_id(&event.event_type) && event.turn_id.is_none() {
        bail!(
            "rejected operator event {}: event type {:?} requires turn_id",
            event.event_id,
            event.event_type
        );
    }

    if requires_call_id(&event.event_type) && event.call_id.is_none() {
        bail!(
            "rejected operator event {}: event type {:?} requires call_id",
            event.event_id,
            event.event_type
        );
    }

    if requires_work_id(&event.event_type) && event.work_id.is_none() {
        bail!(
            "rejected operator event {}: event type {:?} requires work_id",
            event.event_id,
            event.event_type
        );
    }

    if event.event_type == OperatorEventType::TurnStarted {
        if let Some(work_id) = event.work_id.as_deref() {
            let linked = work_threads
                .get(work_id)
                .is_some_and(|linked_threads| linked_threads.contains(&event.thread_id));
            if !linked {
                bail!(
                    "rejected operator event {}: Work scope {work_id} is unknown or not linked to thread {}",
                    event.event_id,
                    event.thread_id
                );
            }
        }
        validate_turn_started(threads, event)?;
    } else if let Some(turn_id) = &event.turn_id {
        let turn = threads
            .get(&event.thread_id)
            .and_then(|folded| folded.turns.iter().find(|turn| &turn.turn_id == turn_id))
            .ok_or_else(|| {
                anyhow!(
                    "rejected operator event {}: turn {turn_id} does not exist in thread {}",
                    event.event_id,
                    event.thread_id
                )
            })?;

        if event.work_id != turn.work_id {
            bail!(
                "rejected operator event {}: Work scope {:?} does not match turn {turn_id} scope {:?}",
                event.event_id,
                event.work_id,
                turn.work_id
            );
        }

        if is_turn_terminal(&turn.status) {
            bail!(
                "rejected operator event {}: turn {turn_id} is already terminal ({}); event {:?} rejected",
                event.event_id,
                turn.status,
                event.event_type
            );
        }

        if event.event_type == OperatorEventType::ApprovalDecided {
            match approval_key(event) {
                Some(key) if turn.pending_approvals.contains(&key) => {}
                Some(_) => bail!(
                    "rejected operator event {}: approval decision has no matching pending request",
                    event.event_id
                ),
                None if event
                    .payload
                    .get("outcome")
                    .and_then(|value| value.as_str())
                    != Some("auto_approved") =>
                {
                    bail!(
                        "rejected operator event {}: approval decision has no matching pending request",
                        event.event_id
                    );
                }
                None => {}
            }
        }

        if event.event_type == OperatorEventType::ToolCallCompleted
            && !tool_key(event).is_some_and(|key| turn.pending_tools.contains(&key))
        {
            bail!(
                "rejected operator event {}: tool completion has no matching pending tool call",
                event.event_id
            );
        }

        if turn.cancel_requested {
            let is_cancel_audit = is_cancellation_approval_audit(turn, event);
            let is_cancel_tool_audit = is_cancellation_tool_audit(turn, event);
            if !is_cancel_audit
                && !is_cancel_tool_audit
                && (event.event_type != OperatorEventType::TurnInterrupted
                    || interruption_reason(event) != Some("OPERATOR_CANCELLED"))
            {
                bail!(
                    "rejected operator event {}: turn {turn_id} has a pending cancellation and must close with turn_interrupted reason OPERATOR_CANCELLED",
                    event.event_id
                );
            }
        } else if event.event_type == OperatorEventType::TurnInterrupted
            && interruption_reason(event) == Some("OPERATOR_CANCELLED")
        {
            bail!(
                "rejected operator event {}: OPERATOR_CANCELLED interruption requires a prior turn_cancel_requested",
                event.event_id
            );
        } else if event.event_type == OperatorEventType::TurnCancelRequested
            && turn.cancel_requested
        {
            bail!(
                "rejected operator event {}: turn {turn_id} already has a pending cancellation",
                event.event_id
            );
        }
    }

    Ok(())
}

/// A `turn_started` event is the one deliberate creation exception: legacy
/// import may synthesize a turn before its other historical events replay.
/// Once created, both its turn id and any supplied client request id are
/// immutable identities within the thread.
fn validate_turn_started(
    threads: &HashMap<String, FoldedThread>,
    event: &OperatorEvent,
) -> Result<()> {
    let turn_id = event
        .turn_id
        .as_deref()
        .expect("requires_turn_id checked before turn_started validation");
    let Some(thread) = threads.get(&event.thread_id) else {
        return Ok(());
    };

    if thread.turns.iter().any(|turn| turn.turn_id == turn_id) {
        bail!(
            "rejected operator event {}: turn {turn_id} already exists in thread {}",
            event.event_id,
            event.thread_id
        );
    }

    let client_request_id = event
        .payload
        .get("client_request_id")
        .and_then(|value| value.as_str());
    if let Some(client_request_id) = client_request_id {
        if thread
            .turns
            .iter()
            .any(|turn| turn.client_request_id.as_deref() == Some(client_request_id))
        {
            bail!(
                "rejected operator event {}: client_request_id {client_request_id:?} already belongs to a turn in thread {}",
                event.event_id,
                event.thread_id
            );
        }
    }

    Ok(())
}

fn requires_turn_id(event_type: &OperatorEventType) -> bool {
    matches!(
        event_type,
        OperatorEventType::TurnStarted
            | OperatorEventType::TurnCompleted
            | OperatorEventType::TurnCancelRequested
            | OperatorEventType::TurnInterrupted
            | OperatorEventType::UserMessage
            | OperatorEventType::RoutePlanned
            | OperatorEventType::RouteAttempted
            | OperatorEventType::RouteCompleted
            | OperatorEventType::RouteFailed
            | OperatorEventType::AssistantStarted
            | OperatorEventType::AssistantCompleted
            | OperatorEventType::ToolCallStarted
            | OperatorEventType::ToolCallCompleted
    )
}

/// Event types that describe a `Work` and are meaningless without naming it.
fn requires_work_id(event_type: &OperatorEventType) -> bool {
    matches!(
        event_type,
        OperatorEventType::WorkCreated
            | OperatorEventType::WorkLinked
            | OperatorEventType::WorkspacePrepared
            | OperatorEventType::WorkspaceReleased
    )
}

fn requires_call_id(event_type: &OperatorEventType) -> bool {
    matches!(
        event_type,
        OperatorEventType::RoutePlanned
            | OperatorEventType::RouteAttempted
            | OperatorEventType::RouteCompleted
            | OperatorEventType::RouteFailed
            | OperatorEventType::ToolCallStarted
            | OperatorEventType::ToolCallCompleted
    )
}

/// The three event types that close a turn out. `Blocker` may or may not
/// carry a `turn_id` (it is not in [`requires_turn_id`]'s list, since a
/// blocker can be thread-scoped); when it does target a turn, that turn
/// becomes terminal in status `"blocked"`.
///
/// `TurnCancelRequested` is deliberately absent: per the cancellation
/// contract (module docs), it is appended before the runner is signalled
/// and only records operator *intent* — the turn stays open until the
/// resulting `turn_interrupted` (payload reason `OPERATOR_CANCELLED`)
/// lands as the actual closing record.
fn is_turn_terminal(status: &str) -> bool {
    matches!(status, "completed" | "interrupted" | "blocked")
}

fn interruption_reason(event: &OperatorEvent) -> Option<&str> {
    event.payload.get("reason").and_then(|value| value.as_str())
}

fn approval_key(event: &OperatorEvent) -> Option<ApprovalKey> {
    let call_id = event.call_id.clone()?;
    if event.payload.get("call_id")?.as_str()? != call_id {
        return None;
    }
    Some(ApprovalKey {
        call_id,
        request_id: event.payload.get("request_id")?.as_str()?.to_string(),
        tool: event.payload.get("tool")?.as_str()?.to_string(),
    })
}

fn is_cancellation_approval_audit(turn: &FoldedTurn, event: &OperatorEvent) -> bool {
    event.event_type == OperatorEventType::ApprovalDecided
        && event
            .payload
            .get("outcome")
            .and_then(|value| value.as_str())
            == Some("cancelled")
        && event.payload.get("reason").and_then(|value| value.as_str())
            == Some("OPERATOR_CANCELLED")
        && approval_key(event).is_some_and(|key| {
            !key.request_id.is_empty()
                && !key.tool.is_empty()
                && event
                    .payload
                    .get("call_id")
                    .and_then(|value| value.as_str())
                    == Some(key.call_id.as_str())
                && turn.pending_approvals.contains(&key)
        })
}

// ---------------------------------------------------------------------
// Materialization (read-side, tolerant).
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
struct FoldedTurn {
    turn_id: String,
    client_request_id: Option<String>,
    work_id: Option<String>,
    prompt_fingerprint: Option<String>,
    status: String,
    cancel_requested: bool,
    prompt: Option<String>,
    user_message_cursor: Option<String>,
    started_at: String,
    route_policy: Option<TurnRoutePolicy>,
    pending_approvals: HashSet<ApprovalKey>,
    pending_tools: HashSet<ToolKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ApprovalKey {
    call_id: String,
    request_id: String,
    tool: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ToolKey {
    call_id: String,
    name: String,
}

#[derive(Debug, Clone)]
struct FoldedThread {
    thread_id: String,
    turns: Vec<FoldedTurn>,
    skipped_events: usize,
    /// Position of the last event this thread was touched by, in global
    /// append order. Used only to order [`OperatorSessionService::list_threads`]
    /// by recency; never exposed directly.
    last_order: usize,
}

impl FoldedThread {
    fn new(thread_id: &str) -> Self {
        Self {
            thread_id: thread_id.to_string(),
            turns: Vec::new(),
            skipped_events: 0,
            last_order: 0,
        }
    }

    /// `skipped_lines` is passed in rather than stored: it is stream-wide
    /// (owned by [`MaterializedJournal`]), not per-thread state.
    fn to_view(
        &self,
        skipped_lines: usize,
        unsupported_schema_events: usize,
    ) -> OperatorThreadView {
        OperatorThreadView {
            thread_id: self.thread_id.clone(),
            turns: self
                .turns
                .iter()
                .map(|turn| OperatorTurnView {
                    turn_id: turn.turn_id.clone(),
                    client_request_id: turn.client_request_id.clone(),
                    work_id: turn.work_id.clone(),
                    status: turn.status.clone(),
                    prompt: turn.prompt.clone(),
                    user_message_cursor: turn.user_message_cursor.clone(),
                    started_at: turn.started_at.clone(),
                    route_policy: turn.route_policy.clone(),
                })
                .collect(),
            skipped_events: self.skipped_events + unsupported_schema_events,
            skipped_lines,
        }
    }

    fn to_summary(&self) -> OperatorThreadSummary {
        OperatorThreadSummary {
            thread_id: self.thread_id.clone(),
            turn_count: self.turns.len(),
            latest_turn_id: self.turns.last().map(|turn| turn.turn_id.clone()),
            latest_status: self.turns.last().map(|turn| turn.status.clone()),
        }
    }
}

/// Result of folding the whole journal: per-thread projections plus the
/// stream-wide count of journal-level damage encountered along the way.
#[derive(Debug, Default)]
struct MaterializedJournal {
    threads: HashMap<String, FoldedThread>,
    /// Durable Work-to-thread relationships derived from accepted Work
    /// lifecycle events. Used only for scoped turn admission.
    work_threads: HashMap<String, HashSet<String>>,
    /// Parsed but unsupported-schema events, tracked by their declared
    /// thread without creating a valid thread projection or affecting
    /// recency. `thread()` may surface this diagnostic count.
    unsupported_schema_events: HashMap<String, usize>,
    /// Current-schema rows rejected by event-specific replay validation.
    /// They remain diagnostics, not valid thread state or recency.
    rejected_current_schema_events: HashMap<String, usize>,
    /// Event IDs already folded at or before `cursor`.
    seen_event_ids: HashSet<String>,
    /// Last durable journal cursor folded into this projection.
    cursor: Option<String>,
    /// Monotonic event order used for thread recency.
    order: usize,
    /// Damaged rows preceding a durable event.
    committed_skipped_lines: usize,
    /// Damaged rows after the current cursor. Replaced, not accumulated,
    /// until a later event moves them into the committed prefix.
    tail_skipped_lines: usize,
    /// Diagnostic proving catch-up work is incremental in tests.
    applied_event_rows: usize,
}

impl MaterializedJournal {
    fn skipped_lines(&self) -> usize {
        self.committed_skipped_lines
            .saturating_add(self.tail_skipped_lines)
    }
}

/// Catch a disposable projection up from its last durable cursor.
///
/// Applies, in append order: reader-side dedup of repeated `event_id`
/// values, skip-and-count for unsupported schema versions, and skip-and-
/// count for nonterminal events addressed to a turn that is already
/// terminal. Journal-level damage (lines that never parse as events) is
/// accumulated separately into `skipped_lines`. Never fails on content —
/// only a genuine I/O/storage error from the underlying journal
/// propagates. Unknown cursor lineage resets the derived fold and rebuilds
/// from stream start; callers' externally supplied cursors still receive the
/// journal's structured invalid-cursor error through `events_after`.
fn sync_materialized(
    journal: &OperatorJournal,
    projection: &mut MaterializedJournal,
) -> Result<()> {
    const PAGE_SIZE: usize = 256;
    loop {
        let page = match journal.read_after(projection.cursor.as_deref(), PAGE_SIZE) {
            Ok(page) => page,
            Err(CursorError::InvalidCursor { .. }) if projection.cursor.is_some() => {
                *projection = MaterializedJournal::default();
                continue;
            }
            Err(error) => return Err(anyhow!(error)),
        };
        if page.events.is_empty() {
            projection.tail_skipped_lines = page.skipped_lines;
            break;
        }

        // A short page may include damaged tail rows after its last event.
        // Probe from that event cursor to separate trailing damage from the
        // committed prefix without replaying any event bodies.
        let mut stable_tail = None;
        if page.events.len() < PAGE_SIZE {
            let probe = journal.read_after(page.next_cursor.as_deref(), 1)?;
            if probe.events.is_empty() {
                stable_tail = Some(probe.skipped_lines);
            }
        }
        let trailing = stable_tail.unwrap_or_default();
        projection.committed_skipped_lines = projection
            .committed_skipped_lines
            .saturating_add(page.skipped_lines.saturating_sub(trailing));
        projection.tail_skipped_lines = trailing;

        for row in &page.events {
            projection.order = projection.order.saturating_add(1);
            projection.applied_event_rows = projection.applied_event_rows.saturating_add(1);
            apply_event(
                &mut projection.threads,
                &mut projection.work_threads,
                &mut projection.unsupported_schema_events,
                &mut projection.rejected_current_schema_events,
                &mut projection.seen_event_ids,
                row,
                projection.order,
            );
        }
        projection.cursor = page.next_cursor;
        if stable_tail.is_some() {
            break;
        }
    }
    Ok(())
}

fn apply_event(
    threads: &mut HashMap<String, FoldedThread>,
    work_threads: &mut HashMap<String, HashSet<String>>,
    unsupported_schema_events: &mut HashMap<String, usize>,
    rejected_current_schema_events: &mut HashMap<String, usize>,
    seen_event_ids: &mut HashSet<String>,
    row: &CursorEvent,
    order: usize,
) {
    let event = &row.event;
    if !seen_event_ids.insert(event.event_id.clone()) {
        return; // Reader-side dedup of a repeated event_id.
    }

    if event.schema_version != OPERATOR_EVENT_SCHEMA_VERSION {
        *unsupported_schema_events
            .entry(event.thread_id.clone())
            .or_default() += 1;
        return;
    }

    if let Some(entry) = threads.get_mut(&event.thread_id) {
        if apply_to_existing_thread(entry, event, row) {
            entry.last_order = order;
            apply_work_membership(work_threads, event);
        } else {
            entry.skipped_events += 1;
        }
        return;
    }

    // Only explicit thread lifecycle and synthetic turn-start records may
    // establish a new projection. All other rows need existing state.
    let mut candidate = FoldedThread::new(&event.thread_id);
    let accepted = match event.event_type {
        OperatorEventType::ThreadCreated => true,
        OperatorEventType::TurnStarted => apply_turn_started(&mut candidate, event),
        _ => false,
    };
    if accepted {
        candidate.last_order = order;
        threads.insert(event.thread_id.clone(), candidate);
        apply_work_membership(work_threads, event);
    } else {
        *rejected_current_schema_events
            .entry(event.thread_id.clone())
            .or_default() += 1;
    }
}

fn apply_work_membership(
    work_threads: &mut HashMap<String, HashSet<String>>,
    event: &OperatorEvent,
) {
    let Some(work_id) = event.work_id.as_deref() else {
        return;
    };
    match event.event_type {
        OperatorEventType::WorkCreated
            if event
                .payload
                .get("primary_thread_id")
                .and_then(|value| value.as_str())
                == Some(event.thread_id.as_str())
                && !work_threads.contains_key(work_id) =>
        {
            work_threads.insert(
                work_id.to_string(),
                HashSet::from([event.thread_id.clone()]),
            );
        }
        OperatorEventType::WorkLinked
            if work_threads.contains_key(work_id)
                && event
                    .payload
                    .get("thread_id")
                    .and_then(|value| value.as_str())
                    == Some(event.thread_id.as_str()) =>
        {
            work_threads
                .get_mut(work_id)
                .expect("contains_key checked")
                .insert(event.thread_id.clone());
        }
        _ => {}
    }
}

fn apply_to_existing_thread(
    entry: &mut FoldedThread,
    event: &OperatorEvent,
    row: &CursorEvent,
) -> bool {
    match event.event_type {
        OperatorEventType::ThreadCreated => false,
        OperatorEventType::TurnStarted => apply_turn_started(entry, event),
        OperatorEventType::UserMessage => apply_user_message(entry, event, row),
        OperatorEventType::TurnCompleted => apply_terminal(entry, event, "completed"),
        OperatorEventType::TurnInterrupted => apply_terminal(entry, event, "interrupted"),
        OperatorEventType::Blocker => apply_terminal(entry, event, "blocked"),
        OperatorEventType::TurnCancelRequested => apply_turn_cancel_requested(entry, event),
        OperatorEventType::ToolCallStarted => apply_tool_started(entry, event),
        OperatorEventType::ToolCallCompleted => apply_tool_completed(entry, event),
        OperatorEventType::ApprovalRequested => apply_approval_requested(entry, event),
        OperatorEventType::ApprovalDecided => apply_approval_decided(entry, event),
        OperatorEventType::RoutePlanned
        | OperatorEventType::RouteAttempted
        | OperatorEventType::RouteCompleted
        | OperatorEventType::RouteFailed
        | OperatorEventType::AssistantStarted
        | OperatorEventType::AssistantCompleted
        | OperatorEventType::ArtifactCreated
        | OperatorEventType::TestResult
        | OperatorEventType::ReceiptLinked
        | OperatorEventType::LegacySessionImported
        // Work events name the thread without opening or closing a turn: the
        // thread is active because of them, but its turn state is untouched.
        | OperatorEventType::WorkCreated
        | OperatorEventType::WorkLinked
        | OperatorEventType::WorkspacePrepared
        | OperatorEventType::WorkspaceReleased
        // Worker and pane lifecycle is the same shape: a worker starting or
        // exiting keeps its thread active without opening or closing a turn.
        // A worker exiting is deliberately NOT terminal for the thread — the
        // operator can launch another worker against the same Work.
        | OperatorEventType::WorkerLaunched
        | OperatorEventType::WorkerHeartbeat
        | OperatorEventType::WorkerExited
        | OperatorEventType::PaneOpened
        | OperatorEventType::PaneClosed => apply_nonterminal_touch(entry, event),
    }
}

fn apply_turn_started(entry: &mut FoldedThread, event: &OperatorEvent) -> bool {
    let Some(turn_id) = event.turn_id.clone() else {
        return false;
    };
    let client_request_id = event
        .payload
        .get("client_request_id")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let prompt_fingerprint = event
        .payload
        .get("prompt_fingerprint")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    if entry.turns.iter().any(|turn| turn.turn_id == turn_id)
        || client_request_id
            .as_deref()
            .is_some_and(|client_request_id| {
                entry
                    .turns
                    .iter()
                    .any(|turn| turn.client_request_id.as_deref() == Some(client_request_id))
            })
    {
        return false;
    }
    let route_policy = event
        .payload
        .get("route_policy")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    entry.turns.push(FoldedTurn {
        turn_id,
        client_request_id,
        work_id: event.work_id.clone(),
        prompt_fingerprint,
        status: "open".to_string(),
        cancel_requested: false,
        prompt: None,
        user_message_cursor: None,
        started_at: event.occurred_at.clone(),
        route_policy,
        pending_approvals: HashSet::new(),
        pending_tools: HashSet::new(),
    });
    true
}

fn apply_user_message(entry: &mut FoldedThread, event: &OperatorEvent, row: &CursorEvent) -> bool {
    let Some(turn_id) = &event.turn_id else {
        return false;
    };
    let Some(turn) = entry.turns.iter_mut().find(|turn| &turn.turn_id == turn_id) else {
        return false;
    };
    if is_turn_terminal(&turn.status) || turn.cancel_requested {
        return false;
    }
    turn.prompt = event
        .payload
        .get("text")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    turn.user_message_cursor = Some(row.cursor.clone());
    true
}

fn apply_terminal(entry: &mut FoldedThread, event: &OperatorEvent, status: &str) -> bool {
    let Some(turn_id) = &event.turn_id else {
        // A thread-scoped Blocker has no turn to close but is still valid.
        return event.event_type == OperatorEventType::Blocker;
    };
    let Some(turn) = entry.turns.iter_mut().find(|turn| &turn.turn_id == turn_id) else {
        return false;
    };
    if is_turn_terminal(&turn.status) {
        return false;
    }
    if turn.cancel_requested
        && (status != "interrupted" || interruption_reason(event) != Some("OPERATOR_CANCELLED"))
    {
        return false;
    }
    if !turn.cancel_requested
        && event.event_type == OperatorEventType::TurnInterrupted
        && interruption_reason(event) == Some("OPERATOR_CANCELLED")
    {
        return false;
    }
    turn.status = status.to_string();
    true
}

fn apply_turn_cancel_requested(entry: &mut FoldedThread, event: &OperatorEvent) -> bool {
    let Some(turn_id) = &event.turn_id else {
        return false;
    };
    let Some(turn) = entry.turns.iter_mut().find(|turn| &turn.turn_id == turn_id) else {
        return false;
    };
    if is_turn_terminal(&turn.status) || turn.cancel_requested {
        return false;
    }
    turn.cancel_requested = true;
    true
}

fn tool_key(event: &OperatorEvent) -> Option<ToolKey> {
    Some(ToolKey {
        call_id: event.call_id.clone()?,
        name: event.payload.get("name")?.as_str()?.to_string(),
    })
}

fn is_cancellation_tool_audit(turn: &FoldedTurn, event: &OperatorEvent) -> bool {
    event.event_type == OperatorEventType::ToolCallCompleted
        && event.payload.get("status").and_then(|value| value.as_str()) == Some("uncertain")
        && event.payload.get("reason").and_then(|value| value.as_str())
            == Some("OPERATOR_CANCELLED")
        && tool_key(event).is_some_and(|key| turn.pending_tools.contains(&key))
}

fn apply_tool_started(entry: &mut FoldedThread, event: &OperatorEvent) -> bool {
    let Some(turn_id) = &event.turn_id else {
        return false;
    };
    let Some(turn) = entry.turns.iter_mut().find(|turn| &turn.turn_id == turn_id) else {
        return false;
    };
    if is_turn_terminal(&turn.status) || turn.cancel_requested {
        return false;
    }
    tool_key(event)
        .map(|key| turn.pending_tools.insert(key))
        .unwrap_or(false)
}

fn apply_tool_completed(entry: &mut FoldedThread, event: &OperatorEvent) -> bool {
    let Some(turn_id) = &event.turn_id else {
        return false;
    };
    let Some(turn) = entry.turns.iter_mut().find(|turn| &turn.turn_id == turn_id) else {
        return false;
    };
    if is_turn_terminal(&turn.status)
        || (turn.cancel_requested && !is_cancellation_tool_audit(turn, event))
    {
        return false;
    }
    tool_key(event)
        .map(|key| turn.pending_tools.remove(&key))
        .unwrap_or(false)
}

fn apply_approval_requested(entry: &mut FoldedThread, event: &OperatorEvent) -> bool {
    let Some(turn_id) = &event.turn_id else {
        return false;
    };
    let Some(turn) = entry.turns.iter_mut().find(|turn| &turn.turn_id == turn_id) else {
        return false;
    };
    if is_turn_terminal(&turn.status) || turn.cancel_requested {
        return false;
    }
    approval_key(event)
        .map(|key| turn.pending_approvals.insert(key))
        .unwrap_or(true)
}

fn apply_approval_decided(entry: &mut FoldedThread, event: &OperatorEvent) -> bool {
    let Some(turn_id) = &event.turn_id else {
        return false;
    };
    let Some(turn) = entry.turns.iter_mut().find(|turn| &turn.turn_id == turn_id) else {
        return false;
    };
    if is_turn_terminal(&turn.status) {
        return false;
    }
    if turn.cancel_requested && !is_cancellation_approval_audit(turn, event) {
        return false;
    }
    match approval_key(event) {
        Some(key) => turn.pending_approvals.remove(&key),
        None => {
            event
                .payload
                .get("outcome")
                .and_then(|value| value.as_str())
                == Some("auto_approved")
        }
    }
}

fn apply_nonterminal_touch(entry: &mut FoldedThread, event: &OperatorEvent) -> bool {
    let Some(turn_id) = &event.turn_id else {
        return true; // A valid thread-scoped note.
    };
    match entry.turns.iter_mut().find(|turn| &turn.turn_id == turn_id) {
        Some(turn)
            if is_turn_terminal(&turn.status)
                || (turn.cancel_requested && !is_cancellation_approval_audit(turn, event)) =>
        {
            false
        }
        Some(_) => true,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use heiwa_evidence::{OperatorActor, OperatorEvent, OperatorEventType, OperatorJournal};
    use serde_json::json;

    use super::{new_event, now_iso, OperatorSessionService, StartTurnRequest};

    #[test]
    fn read_only_replay_does_not_wait_for_a_write_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let service =
            OperatorSessionService::new(OperatorJournal::new(dir.path().to_path_buf()).unwrap());

        // Hold the service's writer transaction lock. A read-only replay
        // must still complete; the journal has its own append-side lock.
        let _write_transaction = service.write_transaction.lock().unwrap();
        let (sent, received) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| sent.send(service.thread("default")).unwrap());
            let view = received
                .recv_timeout(Duration::from_secs(2))
                .expect("read-only replay must not wait for a writer transaction")
                .unwrap();
            assert!(view.turns.is_empty());
        });
    }

    #[test]
    fn materialized_projection_catches_up_without_replaying_history() {
        let dir = tempfile::tempdir().unwrap();
        let service =
            OperatorSessionService::new(OperatorJournal::new(dir.path().to_path_buf()).unwrap());
        let submission = service
            .start_turn(
                "default",
                StartTurnRequest::auto("projection-once", "first message"),
            )
            .unwrap();

        service.thread("default").unwrap();
        let first_applied = service.projection.lock().unwrap().applied_event_rows;
        assert_eq!(first_applied, 3);

        service.thread("default").unwrap();
        assert_eq!(
            service.projection.lock().unwrap().applied_event_rows,
            first_applied,
            "a second read at the same cursor must apply zero historical rows"
        );

        service
            .append_event(new_event(
                "default",
                Some(submission.turn_id),
                None,
                OperatorEventType::AssistantStarted,
                now_iso(),
                OperatorActor {
                    kind: "runtime".into(),
                    id: "projection-test".into(),
                },
                json!({}),
            ))
            .unwrap();
        service.thread("default").unwrap();
        assert_eq!(
            service.projection.lock().unwrap().applied_event_rows,
            first_applied + 1
        );
    }

    #[test]
    fn pending_cancel_rejects_non_cancellation_approval_decision() {
        let dir = tempfile::tempdir().unwrap();
        let service =
            OperatorSessionService::new(OperatorJournal::new(dir.path().to_path_buf()).unwrap());
        let submission = service
            .start_turn("default", StartTurnRequest::auto("cancel-audit", "hello"))
            .unwrap();
        service
            .append_event(new_event(
                "default",
                Some(submission.turn_id.clone()),
                None,
                OperatorEventType::TurnCancelRequested,
                now_iso(),
                OperatorActor {
                    kind: "operator".into(),
                    id: "test".into(),
                },
                json!({"reason": "OPERATOR_REQUEST"}),
            ))
            .unwrap();
        let error = service
            .append_event(new_event(
                "default",
                Some(submission.turn_id),
                Some("call-1".into()),
                OperatorEventType::ApprovalDecided,
                now_iso(),
                OperatorActor {
                    kind: "runtime".into(),
                    id: "test".into(),
                },
                json!({
                    "request_id": "request-1",
                    "tool": "app.deploy",
                    "call_id": "call-1",
                    "outcome": "approved",
                    "reason": "operator approved",
                }),
            ))
            .unwrap_err();
        assert!(error.to_string().contains("matching pending request"));
    }

    fn append_approval_requested(
        service: &OperatorSessionService,
        turn_id: &str,
        call_id: &str,
        request_id: &str,
        tool: &str,
    ) {
        service
            .append_event(new_event(
                "default",
                Some(turn_id.to_string()),
                Some(call_id.to_string()),
                OperatorEventType::ApprovalRequested,
                now_iso(),
                OperatorActor {
                    kind: "runtime".into(),
                    id: "test".into(),
                },
                json!({"request_id": request_id, "tool": tool, "call_id": call_id}),
            ))
            .unwrap();
    }

    fn append_cancel_requested(service: &OperatorSessionService, turn_id: &str) {
        service
            .append_event(new_event(
                "default",
                Some(turn_id.to_string()),
                None,
                OperatorEventType::TurnCancelRequested,
                now_iso(),
                OperatorActor {
                    kind: "operator".into(),
                    id: "test".into(),
                },
                json!({"reason": "OPERATOR_REQUEST"}),
            ))
            .unwrap();
    }

    fn cancellation_decision(
        turn_id: &str,
        call_id: &str,
        request_id: &str,
        tool: &str,
    ) -> OperatorEvent {
        new_event(
            "default",
            Some(turn_id.to_string()),
            Some(call_id.to_string()),
            OperatorEventType::ApprovalDecided,
            now_iso(),
            OperatorActor {
                kind: "runtime".into(),
                id: "test".into(),
            },
            json!({
                "request_id": request_id,
                "tool": tool,
                "call_id": call_id,
                "outcome": "cancelled",
                "reason": "OPERATOR_CANCELLED",
            }),
        )
    }

    fn started_service() -> (tempfile::TempDir, OperatorSessionService, String) {
        let dir = tempfile::tempdir().unwrap();
        let service =
            OperatorSessionService::new(OperatorJournal::new(dir.path().to_path_buf()).unwrap());
        let turn_id = service
            .start_turn(
                "default",
                StartTurnRequest::auto("approval-correlation", "hello"),
            )
            .unwrap()
            .turn_id;
        (dir, service, turn_id)
    }

    #[test]
    fn pending_cancel_rejects_audit_without_prior_request() {
        let (_dir, service, turn_id) = started_service();
        append_cancel_requested(&service, &turn_id);
        assert!(service
            .append_event(cancellation_decision(
                &turn_id,
                "call-1",
                "request-1",
                "app.deploy"
            ))
            .is_err());
    }

    #[test]
    fn pending_cancel_rejects_mismatched_approval_audit() {
        let (_dir, service, turn_id) = started_service();
        append_approval_requested(&service, &turn_id, "call-1", "request-1", "app.deploy");
        append_cancel_requested(&service, &turn_id);
        assert!(service
            .append_event(cancellation_decision(
                &turn_id,
                "call-2",
                "request-1",
                "app.deploy"
            ))
            .is_err());
        assert!(service
            .append_event(cancellation_decision(
                &turn_id,
                "call-1",
                "request-2",
                "app.deploy"
            ))
            .is_err());
        assert!(service
            .append_event(cancellation_decision(
                &turn_id,
                "call-1",
                "request-1",
                "fs.write"
            ))
            .is_err());
    }

    #[test]
    fn pending_cancel_rejects_already_decided_approval_audit() {
        let (_dir, service, turn_id) = started_service();
        append_approval_requested(&service, &turn_id, "call-1", "request-1", "app.deploy");
        service
            .append_event(new_event(
                "default",
                Some(turn_id.clone()),
                Some("call-1".into()),
                OperatorEventType::ApprovalDecided,
                now_iso(),
                OperatorActor { kind: "runtime".into(), id: "test".into() },
                json!({"request_id": "request-1", "tool": "app.deploy", "call_id": "call-1", "outcome": "approved"}),
            ))
            .unwrap();
        append_cancel_requested(&service, &turn_id);
        assert!(service
            .append_event(cancellation_decision(
                &turn_id,
                "call-1",
                "request-1",
                "app.deploy"
            ))
            .is_err());
    }

    #[test]
    fn pending_cancel_accepts_matching_pending_approval_audit() {
        let (_dir, service, turn_id) = started_service();
        append_approval_requested(&service, &turn_id, "call-1", "request-1", "app.deploy");
        append_cancel_requested(&service, &turn_id);
        service
            .append_event(cancellation_decision(
                &turn_id,
                "call-1",
                "request-1",
                "app.deploy",
            ))
            .unwrap();
    }

    #[test]
    fn pending_cancel_accepts_only_correlated_uncertain_tool_receipt_before_terminal() {
        let (_dir, service, turn_id) = started_service();
        service
            .append_event(new_event(
                "default",
                Some(turn_id.clone()),
                Some("call-1".into()),
                OperatorEventType::ToolCallStarted,
                now_iso(),
                OperatorActor {
                    kind: "runtime".into(),
                    id: "test".into(),
                },
                json!({"name": "fs.write"}),
            ))
            .unwrap();
        append_cancel_requested(&service, &turn_id);
        service
            .append_event(new_event(
                "default",
                Some(turn_id.clone()),
                Some("call-1".into()),
                OperatorEventType::ToolCallCompleted,
                now_iso(),
                OperatorActor {
                    kind: "runtime".into(),
                    id: "test".into(),
                },
                json!({
                    "name": "fs.write",
                    "status": "uncertain",
                    "outcome": "uncertain",
                    "reason": "OPERATOR_CANCELLED",
                }),
            ))
            .unwrap();
        service
            .append_event(new_event(
                "default",
                Some(turn_id),
                None,
                OperatorEventType::TurnInterrupted,
                now_iso(),
                OperatorActor {
                    kind: "runtime".into(),
                    id: "test".into(),
                },
                json!({"reason": "OPERATOR_CANCELLED"}),
            ))
            .unwrap();
    }

    #[test]
    fn artifact_links_scans_across_bounded_pages() {
        let (_dir, service, turn_id) = started_service();
        for index in 0..257 {
            let artifact_id = format!("artifact-{index}");
            service
                .journal
                .append(&new_event(
                    "default",
                    Some(turn_id.clone()),
                    None,
                    OperatorEventType::ArtifactCreated,
                    now_iso(),
                    OperatorActor {
                        kind: "runtime".into(),
                        id: "test".into(),
                    },
                    json!({"artifact_id": artifact_id}),
                ))
                .unwrap();
        }

        let links = service.artifact_links().unwrap();
        assert!(links.contains(&("default".to_string(), "artifact-0".to_string())));
        assert!(links.contains(&("default".to_string(), "artifact-256".to_string())));
    }

    #[test]
    fn pending_cancel_rejects_fabricated_uncertain_tool_receipt() {
        let (_dir, service, turn_id) = started_service();
        append_cancel_requested(&service, &turn_id);
        assert!(service
            .append_event(new_event(
                "default",
                Some(turn_id),
                Some("call-1".into()),
                OperatorEventType::ToolCallCompleted,
                now_iso(),
                OperatorActor {
                    kind: "runtime".into(),
                    id: "test".into()
                },
                json!({
                    "name": "fs.write",
                    "status": "uncertain",
                    "outcome": "uncertain",
                    "reason": "OPERATOR_CANCELLED",
                }),
            ))
            .is_err());
    }

    #[test]
    fn a_work_event_without_a_work_id_is_rejected() {
        use heiwa_evidence::{OperatorRisk, OperatorSensitivity, OPERATOR_EVENT_SCHEMA_VERSION};
        use std::collections::HashMap;

        use super::validate_event;

        let threads = HashMap::new();
        let mut event = OperatorEvent {
            schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
            event_id: "evt-1".to_string(),
            thread_id: "thread-1".to_string(),
            turn_id: None,
            run_id: None,
            call_id: None,
            work_id: None,
            event_type: OperatorEventType::WorkCreated,
            occurred_at: "2026-08-22T00:00:00Z".to_string(),
            actor: OperatorActor {
                kind: "user".to_string(),
                id: "local".to_string(),
            },
            risk_class: OperatorRisk::Low,
            sensitivity: OperatorSensitivity::LocalPrivate,
            parent_event_id: None,
            correlation_id: None,
            source_refs: Vec::new(),
            evidence_refs: Vec::new(),
            payload: json!({}),
        };

        let error = validate_event(&threads, &HashMap::new(), &event)
            .expect_err("work events must be scoped");
        assert!(
            error.to_string().contains("requires work_id"),
            "the refusal must name the missing field: {error}"
        );

        event.work_id = Some("work-abc".to_string());
        validate_event(&threads, &HashMap::new(), &event).expect("a scoped work event is accepted");
    }
}
