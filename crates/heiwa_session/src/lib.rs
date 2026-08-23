use anyhow::Result;
use heiwa_config::load as load_config;
use heiwa_evidence::{
    now_iso, OperatorActor, OperatorEvent, OperatorEventType, OperatorJournal, OperatorRisk,
    OperatorSensitivity, OPERATOR_EVENT_SCHEMA_VERSION,
};
use heiwa_protocol::TranscriptBlock;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(unix)]
use uuid::Uuid;

pub mod migration;
pub mod operator;
pub mod operator_index;

pub use operator_index::{
    operator_event_key, rebuild_operator_indexes, rebuild_operator_indexes_at, EmbeddingSink,
    IndexReport, ProductionEmbeddingSink,
};

pub const PERSISTED_TRANSCRIPT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub socket_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingRef {
    pub model: String,
    pub dim: u16,
    pub row_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub id: u64,
    pub ts_unix_ms: i64,
    pub char_len: usize,
    pub block: TranscriptBlock,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_ref: Option<EmbeddingRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTranscript {
    pub version: u32,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub next_entry_id: u64,
    pub entries: Vec<TranscriptEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSearchHit {
    pub session_id: String,
    /// Durable operator event identity. `entry_id` remains the stable numeric
    /// projection key used by the vector index.
    #[serde(default)]
    pub event_id: String,
    pub entry_id: u64,
    pub role: String,
    pub content: String,
}

impl PersistedTranscript {
    pub fn empty(session_id: &str) -> Self {
        Self {
            version: PERSISTED_TRANSCRIPT_VERSION,
            session_id: session_id.to_string(),
            parent_session_id: None,
            next_entry_id: 0,
            entries: Vec::new(),
        }
    }

    pub fn blocks(&self) -> Vec<TranscriptBlock> {
        self.entries.iter().map(|e| e.block.clone()).collect()
    }
}

pub fn get_session_dir() -> PathBuf {
    load_config().paths.sessions_dir
}

pub fn get_session_index_path() -> PathBuf {
    load_config().paths.state_dir.join("sessions.sqlite3")
}

pub fn block_raw_char_len(block: &TranscriptBlock) -> usize {
    match block {
        TranscriptBlock::User(text)
        | TranscriptBlock::Assistant(text)
        | TranscriptBlock::Evidence(text) => text.len(),
        TranscriptBlock::Tool(name, output) => name.len() + output.len(),
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn load_transcript(session_id: &str) -> Result<PersistedTranscript> {
    let service = default_operator_service()?;
    import_legacy_sessions_with_service(&service, &get_session_dir())?;
    transcript_from_events(&service, session_id)
}

/// Import immutable legacy transcript JSON into the configured operator journal.
pub fn import_legacy_sessions(source: &std::path::Path) -> Result<ImportReport> {
    import_legacy_sessions_with_service(&default_operator_service()?, source)
}

/// Injected evidence-root seam for import tests and sandboxed migrations.
pub fn import_legacy_sessions_with_service(
    service: &operator::OperatorSessionService,
    source: &std::path::Path,
) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    for path in legacy_source_paths(source)? {
        let raw = fs::read(&path)?;
        let fingerprint = sha256_hex(&raw);
        let value = serde_json::from_slice(&raw)?;
        if heiwa_evidence::find_sensitive(&value).is_some() {
            anyhow::bail!(
                "refused to import {}: source contains sensitive material",
                path.display()
            );
        }
        let fallback = path
            .file_stem()
            .and_then(|part| part.to_str())
            .unwrap_or("legacy");
        let transcript = migration::parse_persisted(fallback, value)?;
        let session_id = transcript.session_id;
        let existing = service.events_after(&session_id, None, usize::MAX)?;
        if existing.events.iter().any(|row| {
            row.event.event_type == OperatorEventType::LegacySessionImported
                && row
                    .event
                    .payload
                    .get("source_fingerprint")
                    .and_then(|value| value.as_str())
                    == Some(fingerprint.as_str())
        }) {
            report.skipped_files += 1;
            continue;
        }
        let existing_by_id: std::collections::HashMap<String, OperatorEvent> = existing
            .events
            .iter()
            .map(|row| (row.event.event_id.clone(), row.event.clone()))
            .collect();
        let event_ids: std::collections::HashSet<String> = existing
            .events
            .iter()
            .map(|row| row.event.event_id.clone())
            .collect();
        let mut current_turn = None;
        let mut pending = Vec::new();
        for entry in &transcript.entries {
            if matches!(entry.block, TranscriptBlock::User(_)) || current_turn.is_none() {
                let turn_id = legacy_event_id(&session_id, entry.id, "turn");
                pending.push(legacy_event(
                        &session_id,
                        Some(turn_id.clone()),
                        None,
                        OperatorEventType::TurnStarted,
                        legacy_event_id(&session_id, entry.id, "turn_started"),
                        entry.ts_unix_ms,
                        serde_json::json!({"client_request_id": format!("legacy:{fingerprint}:{}", entry.id), "legacy_ts_unix_ms": entry.ts_unix_ms, "compat_entry": compat_entry(entry)}),
                    ));
                current_turn = Some(turn_id);
            }
            if let TranscriptBlock::Tool(name, _) = &entry.block {
                pending.push(legacy_event(
                    &session_id,
                    current_turn.clone(),
                    Some(legacy_event_id(&session_id, entry.id, "call")),
                    OperatorEventType::ToolCallStarted,
                    legacy_event_id(&session_id, entry.id, "tool_started"),
                    entry.ts_unix_ms,
                    serde_json::json!({"name": name, "legacy_ts_unix_ms": entry.ts_unix_ms}),
                ));
            }
            let (event_type, call_id, payload, role) = match &entry.block {
                TranscriptBlock::User(text) => (
                    OperatorEventType::UserMessage,
                    None,
                    serde_json::json!({"text": text, "legacy_ts_unix_ms": entry.ts_unix_ms, "compat_entry": compat_entry(entry)}),
                    "user",
                ),
                TranscriptBlock::Assistant(text) => (
                    OperatorEventType::AssistantCompleted,
                    None,
                    serde_json::json!({"text": text, "legacy_ts_unix_ms": entry.ts_unix_ms, "compat_entry": compat_entry(entry)}),
                    "assistant",
                ),
                TranscriptBlock::Tool(name, output) => (
                    OperatorEventType::ToolCallCompleted,
                    Some(legacy_event_id(&session_id, entry.id, "call")),
                    serde_json::json!({"name": name, "output": output, "legacy_ts_unix_ms": entry.ts_unix_ms, "compat_entry": compat_entry(entry)}),
                    "tool",
                ),
                TranscriptBlock::Evidence(text) => (
                    OperatorEventType::ReceiptLinked,
                    None,
                    serde_json::json!({"text": text, "legacy_ts_unix_ms": entry.ts_unix_ms, "compat_entry": compat_entry(entry)}),
                    "evidence",
                ),
            };
            pending.push(legacy_event(
                &session_id,
                current_turn.clone(),
                call_id,
                event_type,
                legacy_event_id(&session_id, entry.id, role),
                entry.ts_unix_ms,
                payload,
            ));
        }
        pending.push(legacy_event(
            &session_id,
            None,
            None,
            OperatorEventType::LegacySessionImported,
            legacy_event_id(&session_id, transcript.next_entry_id, "marker"),
            0,
            serde_json::json!({"source_fingerprint": fingerprint, "source": path.file_name().and_then(|name| name.to_str()).unwrap_or("legacy"), "compat_transcript": {"parent_session_id": transcript.parent_session_id, "next_entry_id": transcript.next_entry_id}}),
        ));
        for event in &pending {
            if !event_ids.contains(&event.event_id)
                && heiwa_evidence::find_sensitive(&event.payload).is_some()
            {
                anyhow::bail!(
                    "refused to import {}: {:?} payload contains sensitive material",
                    path.display(),
                    event.event_type
                );
            }
            if let Some(existing) = existing_by_id.get(&event.event_id) {
                if existing != event {
                    anyhow::bail!("refused to import {}: deterministic event {} conflicts with existing journal content", path.display(), event.event_id);
                }
            }
        }
        for event in pending {
            if event.event_type != OperatorEventType::LegacySessionImported
                && !event_ids.contains(&event.event_id)
            {
                report.imported_entries += usize::from(!matches!(
                    event.event_type,
                    OperatorEventType::TurnStarted | OperatorEventType::ToolCallStarted
                ));
            }
            if !event_ids.contains(&event.event_id) {
                service.append_event(event)?;
            }
        }
        report.imported_files += 1;
    }
    Ok(report)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub imported_files: usize,
    pub skipped_files: usize,
    pub imported_entries: usize,
}

pub fn save_entries(persisted: &PersistedTranscript) -> Result<()> {
    let existing = load_transcript(&persisted.session_id)?;
    if persisted.entries.len() < existing.entries.len() {
        anyhow::bail!("legacy transcript truncation is unavailable after operator-stream cutover");
    }
    for (old, supplied) in existing.entries.iter().zip(&persisted.entries) {
        if serde_json::to_value(old)? != serde_json::to_value(supplied)? {
            anyhow::bail!("legacy transcript rewrite is unavailable after operator-stream cutover");
        }
    }
    for entry in persisted.entries.iter().skip(existing.entries.len()) {
        append_exact_entry(&persisted.session_id, entry.clone())?;
    }
    set_compat_transcript_metadata(
        &persisted.session_id,
        persisted.parent_session_id.clone(),
        Some(persisted.next_entry_id),
    )?;
    let _ = rebuild_operator_indexes(&default_operator_service()?, &ProductionEmbeddingSink)?;
    Ok(())
}

pub fn set_parent_session_id(session_id: &str, parent_session_id: Option<String>) -> Result<()> {
    set_compat_transcript_metadata(session_id, parent_session_id, None)
}

fn set_compat_transcript_metadata(
    session_id: &str,
    parent_session_id: Option<String>,
    next_entry_id: Option<u64>,
) -> Result<()> {
    default_operator_service()?.append_event(new_operator_event(
        session_id,
        None,
        None,
        OperatorEventType::ArtifactCreated,
        serde_json::json!({"compat_parent_session_id": parent_session_id, "compat_next_entry_id": next_entry_id}),
    ))?;
    Ok(())
}

pub fn rebuild_session_index(session_id: &str) -> Result<()> {
    let _ = session_id;
    rebuild_operator_indexes(&default_operator_service()?, &ProductionEmbeddingSink)?;
    Ok(())
}

pub fn search_session_messages(
    session_id: Option<&str>,
    query: &str,
    limit: usize,
) -> Result<Vec<SessionSearchHit>> {
    operator_index::search_session_messages_at(&get_session_index_path(), session_id, query, limit)
}

/// Compat shim for callers that still pass `&[TranscriptBlock]`.
///
/// Reads the previously persisted entries to preserve IDs, timestamps, and
/// embedding refs for blocks already on disk. New blocks beyond the prior
/// length get fresh IDs from `next_entry_id`. Callers today are append-only,
/// so overlapping positions are assumed unchanged.
pub fn save_transcript(session_id: &str, blocks: &[TranscriptBlock]) -> Result<()> {
    let existing = load_transcript(session_id)?;
    let prior = existing.blocks();
    if blocks.len() < prior.len() {
        anyhow::bail!("legacy transcript truncation is unavailable after operator-stream cutover");
    }
    if serde_json::to_value(&blocks[..prior.len()])? != serde_json::to_value(&prior)? {
        anyhow::bail!("legacy transcript rewrite is unavailable after operator-stream cutover");
    }
    for block in blocks.iter().skip(prior.len()) {
        append_entry(session_id, block.clone())?;
    }
    let service = default_operator_service()?;
    let _ = rebuild_operator_indexes(&service, &ProductionEmbeddingSink)?;
    Ok(())
}

/// Append a single block, returning the populated entry.
pub fn append_entry(session_id: &str, block: TranscriptBlock) -> Result<TranscriptEntry> {
    let service = default_operator_service()?;
    let id = transcript_from_events(&service, session_id)?.next_entry_id;
    let timestamp = now_unix_ms();
    let entry = TranscriptEntry {
        id,
        ts_unix_ms: timestamp,
        char_len: block_raw_char_len(&block),
        block: block.clone(),
        embedding_ref: None,
    };
    append_exact_entry(session_id, entry)
}

fn append_exact_entry(session_id: &str, entry: TranscriptEntry) -> Result<TranscriptEntry> {
    let service = default_operator_service()?;
    let block = entry.block.clone();
    match &block {
        TranscriptBlock::User(text) => {
            let turn_id = uuid::Uuid::new_v4().to_string();
            let started = new_operator_event(
                session_id,
                Some(turn_id.clone()),
                None,
                OperatorEventType::TurnStarted,
                serde_json::json!({"client_request_id": uuid::Uuid::new_v4().to_string()}),
            );
            let message = new_operator_event(
                session_id,
                Some(turn_id),
                None,
                OperatorEventType::UserMessage,
                serde_json::json!({"text": text, "compat_entry": compat_entry(&entry)}),
            );
            if heiwa_evidence::find_sensitive(&started.payload).is_some()
                || heiwa_evidence::find_sensitive(&message.payload).is_some()
            {
                anyhow::bail!(
                    "refused to append transcript entry: payload contains sensitive material"
                );
            }
            service.append_event(started)?;
            service.append_event(message)?;
        }
        TranscriptBlock::Assistant(text) => {
            let turn = latest_open_turn(&service, session_id)?;
            service.append_event(new_operator_event(
                session_id,
                Some(turn),
                None,
                OperatorEventType::AssistantCompleted,
                serde_json::json!({"text": text, "compat_entry": compat_entry(&entry)}),
            ))?;
        }
        TranscriptBlock::Tool(name, output) => {
            let turn = latest_open_turn(&service, session_id)?;
            let call_id = uuid::Uuid::new_v4().to_string();
            let started = new_operator_event(
                session_id,
                Some(turn.clone()),
                Some(call_id.clone()),
                OperatorEventType::ToolCallStarted,
                serde_json::json!({"name": name}),
            );
            let completed = new_operator_event(
                session_id,
                Some(turn),
                Some(call_id),
                OperatorEventType::ToolCallCompleted,
                serde_json::json!({"name": name, "output": output, "compat_entry": compat_entry(&entry)}),
            );
            if heiwa_evidence::find_sensitive(&serde_json::to_value(&started)?).is_some()
                || heiwa_evidence::find_sensitive(&serde_json::to_value(&completed)?).is_some()
            {
                anyhow::bail!(
                    "refused to append transcript entry: event contains sensitive material"
                );
            }
            service.append_event(started)?;
            service.append_event(completed)?;
        }
        TranscriptBlock::Evidence(text) => {
            let turn = latest_open_turn(&service, session_id).ok();
            service.append_event(new_operator_event(
                session_id,
                turn,
                None,
                OperatorEventType::ReceiptLinked,
                serde_json::json!({"text": text, "compat_entry": compat_entry(&entry)}),
            ))?;
        }
    }
    Ok(entry)
}

fn default_operator_service() -> Result<operator::OperatorSessionService> {
    Ok(operator::OperatorSessionService::new(OperatorJournal::new(
        heiwa_evidence::journal_root()?,
    )?))
}

fn legacy_source_paths(source: &std::path::Path) -> Result<Vec<PathBuf>> {
    if source.is_file() {
        return Ok(vec![source.to_path_buf()]);
    }
    if !source.exists() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(source)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn legacy_event_id(session_id: &str, entry_id: u64, role: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("heiwa:legacy:{session_id}:{entry_id}:{role}").as_bytes(),
    )
    .to_string()
}

fn compat_entry(entry: &TranscriptEntry) -> serde_json::Value {
    serde_json::json!({
        "id": entry.id,
        "ts_unix_ms": entry.ts_unix_ms,
        "char_len": entry.char_len,
        "embedding_ref": entry.embedding_ref,
    })
}

fn legacy_event(
    thread_id: &str,
    turn_id: Option<String>,
    call_id: Option<String>,
    event_type: OperatorEventType,
    event_id: String,
    legacy_ts_unix_ms: i64,
    payload: serde_json::Value,
) -> OperatorEvent {
    OperatorEvent {
        schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
        event_id,
        thread_id: thread_id.to_string(),
        turn_id,
        run_id: None,
        call_id,
        work_id: None,
        event_type,
        occurred_at: legacy_occurred_at(legacy_ts_unix_ms),
        actor: OperatorActor {
            kind: "legacy_import".to_string(),
            id: "transcript_json".to_string(),
        },
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: Vec::new(),
        evidence_refs: Vec::new(),
        payload,
    }
}

fn legacy_occurred_at(ts_unix_ms: i64) -> String {
    use time::{format_description::well_known::Rfc3339, OffsetDateTime};
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ts_unix_ms) * 1_000_000)
        .ok()
        .and_then(|timestamp| timestamp.format(&Rfc3339).ok())
        .unwrap_or_else(now_iso)
}

fn new_operator_event(
    thread_id: &str,
    turn_id: Option<String>,
    call_id: Option<String>,
    event_type: OperatorEventType,
    payload: serde_json::Value,
) -> OperatorEvent {
    OperatorEvent {
        schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
        event_id: uuid::Uuid::new_v4().to_string(),
        thread_id: thread_id.to_string(),
        turn_id,
        run_id: None,
        call_id,
        work_id: None,
        event_type,
        occurred_at: now_iso(),
        actor: OperatorActor {
            kind: "compat".to_string(),
            id: "heiwa-session".to_string(),
        },
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: Vec::new(),
        evidence_refs: Vec::new(),
        payload,
    }
}

fn latest_open_turn(
    service: &operator::OperatorSessionService,
    session_id: &str,
) -> Result<String> {
    service
        .thread(session_id)?
        .turns
        .into_iter()
        .rev()
        .find(|turn| turn.status == "open")
        .map(|turn| turn.turn_id)
        .ok_or_else(|| anyhow::anyhow!("cannot append transcript block without an open user turn"))
}

fn transcript_from_events(
    service: &operator::OperatorSessionService,
    session_id: &str,
) -> Result<PersistedTranscript> {
    let mut transcript = PersistedTranscript::empty(session_id);
    for row in service.events_after(session_id, None, usize::MAX)?.events {
        let event = row.event;
        if event.event_type == OperatorEventType::ArtifactCreated {
            if let Some(parent) = event.payload.get("compat_parent_session_id") {
                transcript.parent_session_id = serde_json::from_value(parent.clone()).ok();
            }
            if let Some(next) = event
                .payload
                .get("compat_next_entry_id")
                .and_then(|value| value.as_u64())
            {
                transcript.next_entry_id = next;
            }
        }
        if event.event_type == OperatorEventType::LegacySessionImported {
            if let Some(metadata) = event.payload.get("compat_transcript") {
                transcript.parent_session_id = metadata
                    .get("parent_session_id")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok());
                if let Some(next_entry_id) = metadata
                    .get("next_entry_id")
                    .and_then(|value| value.as_u64())
                {
                    transcript.next_entry_id = next_entry_id;
                }
            }
        }
        let block = match event.event_type {
            OperatorEventType::UserMessage => event
                .payload
                .get("text")
                .and_then(|value| value.as_str())
                .map(|text| TranscriptBlock::User(text.to_string())),
            OperatorEventType::AssistantCompleted => event
                .payload
                .get("text")
                .and_then(|value| value.as_str())
                .map(|text| TranscriptBlock::Assistant(text.to_string())),
            OperatorEventType::ToolCallCompleted => match (
                event.payload.get("name").and_then(|value| value.as_str()),
                event.payload.get("output").and_then(|value| value.as_str()),
            ) {
                (Some(name), Some(output)) => {
                    Some(TranscriptBlock::Tool(name.to_string(), output.to_string()))
                }
                _ => None,
            },
            OperatorEventType::ReceiptLinked => event
                .payload
                .get("text")
                .and_then(|value| value.as_str())
                .map(|text| TranscriptBlock::Evidence(text.to_string())),
            _ => None,
        };
        if let Some(block) = block {
            let compat = event.payload.get("compat_entry");
            let id = compat
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_u64())
                .unwrap_or(transcript.next_entry_id);
            let ts_unix_ms = compat
                .and_then(|value| value.get("ts_unix_ms"))
                .and_then(|value| value.as_i64())
                .or_else(|| {
                    event
                        .payload
                        .get("legacy_ts_unix_ms")
                        .or_else(|| event.payload.get("compat_ts_unix_ms"))
                        .and_then(|value| value.as_i64())
                })
                .unwrap_or(0);
            let embedding_ref = compat
                .and_then(|value| value.get("embedding_ref"))
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok());
            let char_len = compat
                .and_then(|value| value.get("char_len"))
                .and_then(|value| value.as_u64())
                .map(|value| value as usize)
                .unwrap_or_else(|| block_raw_char_len(&block));
            transcript.next_entry_id = transcript.next_entry_id.max(id.saturating_add(1));
            transcript.entries.push(TranscriptEntry {
                id,
                ts_unix_ms,
                char_len,
                block,
                embedding_ref,
            });
        }
    }
    Ok(transcript)
}

#[cfg(unix)]
pub fn start_daemon() -> Result<SessionInfo> {
    start_daemon_at(get_session_dir())
}

/// Start a daemon in an injected session root. The installed surface uses
/// [`start_daemon`]; tests and sandboxes must never touch the owner root.
#[cfg(unix)]
pub fn start_daemon_at(session_dir: PathBuf) -> Result<SessionInfo> {
    let session_id = Uuid::new_v4().to_string();
    fs::create_dir_all(&session_dir)?;

    let socket_path = session_dir.join(format!("{}.sock", session_id));
    let socket_path_clone = socket_path.clone();
    let listener = UnixListener::bind(&socket_path_clone)?;

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((_stream, _addr)) => {
                    // Handle control connection
                }
                Err(e) => eprintln!("Accept error: {}", e),
            }
        }
    });

    Ok(SessionInfo {
        session_id,
        socket_path,
    })
}

#[cfg(not(unix))]
pub fn start_daemon() -> Result<SessionInfo> {
    Err(anyhow::anyhow!(
        "session daemon sockets are not supported on this platform yet"
    ))
}

pub fn attach_session(_session_id: &str) -> Result<()> {
    Ok(())
}

pub struct PtySession {
    pub master: Box<dyn portable_pty::MasterPty + Send>,
}

impl PtySession {
    pub fn new() -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell = if cfg!(target_os = "windows") {
            "cmd.exe"
        } else {
            "bash"
        };

        let cmd = CommandBuilder::new(shell);
        let _child = pair.slave.spawn_command(cmd)?;

        Ok(Self {
            master: pair.master,
        })
    }
}
