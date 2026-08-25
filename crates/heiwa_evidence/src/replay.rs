//! Journal replay: read a stream back as versioned envelopes.
//!
//! Replay is corruption-tolerant by design: a crash mid-append can leave a
//! truncated tail line, and foreign tools can damage a stream. Malformed
//! lines are skipped and counted, never fatal — the journal must always be
//! readable up to its last good record.

use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::journal::{lock_stream, stream_path};

#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceEvent {
    /// Envelope schema version; `0` marks a legacy pre-versioned line.
    pub v: u32,
    pub at_ms: u64,
    pub kind: String,
    pub record: Value,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ReplayedStream {
    pub events: Vec<EvidenceEvent>,
    pub skipped_lines: usize,
}

/// Read every event of one stream, oldest first. Missing streams are empty,
/// not errors. Takes the stream's append lock so a concurrent writer cannot
/// hand us a torn tail.
pub fn read_stream(dir: &Path, kind: &str) -> Result<ReplayedStream> {
    let path = stream_path(dir, kind);
    if !path.exists() {
        return Ok(ReplayedStream::default());
    }

    let _stream_lock = lock_stream(dir, kind)?;
    read_stream_unlocked(dir, kind)
}

/// Read a stream when the caller already holds its append lock.
///
/// This is crate-private because skipping the lock is sound only inside an
/// existing stream transaction such as atomic worker-lease acquisition.
pub(crate) fn read_stream_unlocked(dir: &Path, kind: &str) -> Result<ReplayedStream> {
    let path = stream_path(dir, kind);
    if !path.exists() {
        return Ok(ReplayedStream::default());
    }

    let mut raw = Vec::new();
    OpenOptions::new()
        .read(true)
        .open(&path)?
        .read_to_end(&mut raw)?;

    let mut stream = ReplayedStream::default();
    for chunk in raw.split(|byte| *byte == b'\n') {
        if chunk.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(chunk);
        match serde_json::from_str::<Value>(&line)
            .ok()
            .and_then(parse_event)
        {
            Some(event) => stream.events.push(event),
            None => stream.skipped_lines += 1,
        }
    }
    Ok(stream)
}

/// Per-stream event counts for operator surfaces (`heiwa receipts`,
/// monitors). Streams are the `<kind>.jsonl` files under the journal root;
/// sidecar lock/compaction files are excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSummary {
    pub kind: String,
    pub events: usize,
    pub skipped_lines: usize,
}

pub fn journal_summary(dir: &Path) -> Result<Vec<StreamSummary>> {
    let mut summary = Vec::new();
    if !dir.exists() {
        return Ok(summary);
    }
    let mut kinds: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_str()?.to_string();
            if name.starts_with('.') {
                return None;
            }
            name.strip_suffix(".jsonl").map(str::to_string)
        })
        .collect();
    kinds.sort();
    for kind in kinds {
        let stream = read_stream(dir, &kind)?;
        summary.push(StreamSummary {
            kind,
            events: stream.events.len(),
            skipped_lines: stream.skipped_lines,
        });
    }
    Ok(summary)
}

fn parse_event(value: Value) -> Option<EvidenceEvent> {
    let object = value.as_object()?;
    let kind = object.get("kind")?.as_str()?.to_string();
    let record = object.get("record")?.clone();
    let v = object.get("v").and_then(Value::as_u64).unwrap_or(0) as u32;
    let at_ms = object.get("at_ms").and_then(Value::as_u64).unwrap_or(0);
    Some(EvidenceEvent {
        v,
        at_ms,
        kind,
        record,
    })
}
