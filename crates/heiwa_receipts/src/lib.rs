//! Local SQLite store for **call receipts**.
//!
//! One row per cost-bearing model or tool call. Every operator view — by lane,
//! by agent, by model, by day — is a `SUM(...) GROUP BY ...` rollup over this
//! table. CAD is the storage base unit; presentation in other currencies is a
//! divide-at-read overlay handled at the UI layer.
//!
//! ## What a call receipt is not
//!
//! A [`CallReceipt`] is evidence that Heiwa **spent something**, never evidence
//! that anything **happened** outside Heiwa. A successful model call can produce
//! no effect at all, and an effect can occur even when the caller loses the
//! response and records an error. Proof that a file was written, a branch
//! published, a message sent, or a payment made is a separate noun — an Effect
//! Receipt — and it does not exist in this crate yet.
//!
//! Keeping the two apart is publication gate 1 of the Work Continuity design.
//! Until an Effect Receipt exists, no surface may present a call receipt as
//! proof of an external effect.
//!
//! See `docs/architecture/receipts.md` and
//! `docs/superpowers/specs/2026-08-27-heiwa-work-continuity-triple-design.md`.
//!
//! ## Status
//!
//! - Schema, insert, query, env/agent/model rollups: implemented.
//! - Rate-table loading + cost computation (actual + counterfactual): implemented.
//! - Tamper-evident SHA-256 hash chain (`prev_hash`/`entry_hash`) + `verify_chain`: implemented.
//! - `header()` returns the redacted subset a future export path may carry.
//!   No remote mirror is wired; the hosted authority plane was retired
//!   2026-07-15 and durable truth is local.
//! - Prompt bodies, WAL catch-up, CLI surface: **not implemented**.
//! - Effect receipts: **not implemented**. Named here so the gap is visible,
//!   not to imply a partial one exists.
//! - `id` is currently `uuid v4`; spec calls for ULID. Ordering is by `at`, not
//!   by id, so switching id type later requires no schema migration.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 3;
const INITIAL_SQL: &str = include_str!("../migrations/0001_initial.sql");
const MIGRATION_0002_SQL: &str = include_str!("../migrations/0002_hash_chain.sql");
const MIGRATION_0003_SQL: &str = include_str!("../migrations/0003_model_call_accounting.sql");

/// Genesis predecessor for the first receipt in a chain: SHA-256's width in
/// zero bytes, hex-encoded.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Error)]
pub enum ReceiptError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid env: {0}")]
    InvalidEnv(String),
    #[error("rate not found: env={env:?} provider={provider} model={model}")]
    RateNotFound {
        env: Env,
        provider: String,
        model: String,
    },
    #[error("invalid schema version: found {found}, expected {expected}")]
    SchemaVersion { found: i64, expected: i64 },
    #[error("cannot migrate broken schema-v2 receipt chain at seq {seq} ({id}): {reason:?}")]
    BrokenV2Chain {
        seq: i64,
        id: String,
        reason: ChainBreak,
    },
    #[error("store lock poisoned")]
    LockPoisoned,
}

pub type Result<T> = std::result::Result<T, ReceiptError>;

// ============================================================================
// Domain types
// ============================================================================

/// Where the call ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Env {
    /// Local model on the operator machine. Zero incremental cost.
    Local,
    /// Sub-backed via a provider CLI. Zero incremental cost.
    Oauth,
    /// Metered API call. Real money.
    Api,
}

impl Env {
    pub fn as_str(&self) -> &'static str {
        match self {
            Env::Local => "local",
            Env::Oauth => "oauth",
            Env::Api => "api",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "local" => Ok(Env::Local),
            "oauth" => Ok(Env::Oauth),
            "api" => Ok(Env::Api),
            other => Err(ReceiptError::InvalidEnv(other.to_string())),
        }
    }
}

/// Full local-side accounting row. One per cost-bearing model or tool call.
///
/// Records economics and execution telemetry — provider, model, tokens,
/// latency, attempts, cost. It carries no target, no idempotency key, and no
/// verification, because it makes no claim about the world outside this
/// process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CallReceipt {
    pub id: String,
    pub at: i64,
    pub env: Env,
    pub provider: String,
    pub model: String,
    pub agent: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub latency_ms: i64,
    pub actual_cost_cad: f64,
    pub counterfactual_cost_cad: f64,
    #[serde(default)]
    pub model_call_cost_usd: Option<f64>,
    #[serde(default)]
    pub model_call_cost_truth: Option<String>,
    #[serde(default)]
    pub model_call_attempts: Option<i64>,
    #[serde(default)]
    pub failed_attempt_cost_usd: Option<f64>,
    pub session_id: String,
    pub parent_id: Option<String>,
}

impl CallReceipt {
    /// Build a new receipt with a fresh `id` and the given fields.
    /// `at` is unix seconds UTC; caller supplies it so this remains pure.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        at: i64,
        env: Env,
        provider: impl Into<String>,
        model: impl Into<String>,
        agent: impl Into<String>,
        tokens_in: i64,
        tokens_out: i64,
        latency_ms: i64,
        actual_cost_cad: f64,
        counterfactual_cost_cad: f64,
        session_id: impl Into<String>,
        parent_id: Option<String>,
    ) -> Self {
        CallReceipt {
            id: Uuid::new_v4().to_string(),
            at,
            env,
            provider: provider.into(),
            model: model.into(),
            agent: agent.into(),
            tokens_in,
            tokens_out,
            latency_ms,
            actual_cost_cad,
            counterfactual_cost_cad,
            model_call_cost_usd: None,
            model_call_cost_truth: None,
            model_call_attempts: None,
            failed_attempt_cost_usd: None,
            session_id: session_id.into(),
            parent_id,
        }
    }

    /// The redacted subset safe to carry across a sharing boundary.
    ///
    /// The row has no prompt body today, but the boundary helper exists so that
    /// future fields (prompt hash, completion summary) cannot leak by default.
    /// Nothing consumes this yet: the hosted mirror this once served was
    /// retired with the backend pivot, and any replacement export must clear
    /// the redaction policy before it ships.
    pub fn header(&self) -> ReceiptHeader {
        ReceiptHeader {
            id: self.id.clone(),
            at: self.at,
            env: self.env,
            provider: self.provider.clone(),
            model: self.model.clone(),
            agent: Some(self.agent.clone()),
            tokens_in: self.tokens_in,
            tokens_out: self.tokens_out,
            latency_ms: self.latency_ms,
            actual_cost_cad: self.actual_cost_cad,
            counterfactual_cost_cad: self.counterfactual_cost_cad,
            schema_version: SCHEMA_VERSION,
        }
    }
}

/// Former name of [`CallReceipt`].
///
/// Kept so the rename is not a breaking change mid-migration. The old name says
/// only "receipt", which is the ambiguity the Work Continuity design requires
/// removing before either noun can be published: a reader could not tell
/// whether it meant "we paid for a call" or "something happened out there".
#[deprecated(
    since = "0.1.0",
    note = "use CallReceipt; an Effect Receipt is a different noun"
)]
pub type Receipt = CallReceipt;

/// The exportable subset of a call receipt. Never contains prompt content or
/// completions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReceiptHeader {
    pub id: String,
    pub at: i64,
    pub env: Env,
    pub provider: String,
    pub model: String,
    /// Optional — operators may redact agent attribution before export.
    pub agent: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub latency_ms: i64,
    pub actual_cost_cad: f64,
    pub counterfactual_cost_cad: f64,
    pub schema_version: i64,
}

// ============================================================================
// Tamper-evident chain
// ============================================================================

/// Outcome of [`ReceiptStore::verify_chain`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainStatus {
    /// Every one of `len` rows recomputed to its stored `entry_hash`; `head` is
    /// the chain tip (`GENESIS_HASH` when the store is empty).
    Intact { len: u64, head: String },
    /// Verification failed at the row with chain position `seq` (`id`).
    Broken {
        seq: i64,
        id: String,
        reason: ChainBreak,
    },
}

/// How a chain link failed to verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainBreak {
    /// The row's stored `prev_hash` does not match the previous row's
    /// `entry_hash` — a row was inserted, removed, or reordered.
    PrevMismatch,
    /// The recomputed digest does not match the stored `entry_hash` — a field
    /// in this row was altered after it was written.
    HashMismatch,
    /// The row carries no chain columns (predates chaining or was written
    /// outside the store).
    MissingHash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiptHashVersion {
    /// Schema v2's deployed preimage used the original `chain.v1` domain.
    LegacyV2,
    /// Schema v3 adds executor USD accounting fields under a new domain.
    V3,
}

/// Deterministic schema-v3 SHA-256 digest binding receipt `r` to its
/// predecessor `prev_hash`. Pure, so it is trivially testable and identical on
/// every platform.
///
/// The preimage is canonical by construction: integers render as decimal, costs
/// as fixed 6-decimal, and every free-text field is length-prefixed
/// (`key=<len>:<value>`) so no value can borrow bytes from its neighbour. The
/// `heiwa.receipt.chain.v3` is the current domain separator. Schema-v2
/// migration verification uses its legacy preimage explicitly.
pub fn entry_hash(r: &CallReceipt, prev_hash: &str) -> String {
    entry_hash_for_version(r, prev_hash, ReceiptHashVersion::V3)
}

fn entry_hash_for_version(r: &CallReceipt, prev_hash: &str, version: ReceiptHashVersion) -> String {
    use std::fmt::Write as _;

    fn lp(buf: &mut String, key: &str, val: &str) {
        let _ = writeln!(buf, "{key}={}:{val}", val.len());
    }

    let mut p = String::with_capacity(256);
    p.push_str(match version {
        ReceiptHashVersion::LegacyV2 => "heiwa.receipt.chain.v1\n",
        ReceiptHashVersion::V3 => "heiwa.receipt.chain.v3\n",
    });
    lp(&mut p, "prev", prev_hash);
    lp(&mut p, "id", &r.id);
    let _ = writeln!(p, "at={}", r.at);
    let _ = writeln!(p, "env={}", r.env.as_str());
    lp(&mut p, "provider", &r.provider);
    lp(&mut p, "model", &r.model);
    lp(&mut p, "agent", &r.agent);
    let _ = writeln!(p, "tokens_in={}", r.tokens_in);
    let _ = writeln!(p, "tokens_out={}", r.tokens_out);
    let _ = writeln!(p, "latency_ms={}", r.latency_ms);
    let _ = writeln!(p, "actual_cost_cad={:.6}", r.actual_cost_cad);
    let _ = writeln!(
        p,
        "counterfactual_cost_cad={:.6}",
        r.counterfactual_cost_cad
    );
    if version == ReceiptHashVersion::V3 {
        let _ = writeln!(
            p,
            "model_call_cost_usd={}",
            r.model_call_cost_usd
                .map(|value| format!("{value:.6}"))
                .unwrap_or_default()
        );
        lp(
            &mut p,
            "model_call_cost_truth",
            r.model_call_cost_truth.as_deref().unwrap_or(""),
        );
        let _ = writeln!(
            p,
            "model_call_attempts={}",
            r.model_call_attempts
                .map(|value| value.to_string())
                .unwrap_or_default()
        );
        let _ = writeln!(
            p,
            "failed_attempt_cost_usd={}",
            r.failed_attempt_cost_usd
                .map(|value| format!("{value:.6}"))
                .unwrap_or_default()
        );
    }
    lp(&mut p, "session_id", &r.session_id);
    lp(&mut p, "parent_id", r.parent_id.as_deref().unwrap_or(""));

    let mut hasher = Sha256::new();
    hasher.update(p.as_bytes());
    let digest = hasher.finalize();

    let mut hex = String::with_capacity(64);
    for b in digest {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

// ============================================================================
// Rate table
// ============================================================================

/// Per-model pricing. `cad` suffix on every field — base unit is operator locale.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateEntry {
    pub input_per_mtok_cad: f64,
    pub output_per_mtok_cad: f64,
    /// Cost the *same tokens* would have incurred on the metered API lane.
    /// For `api` entries this equals the actual rate.
    #[serde(default)]
    pub counterfactual: Option<CounterfactualRate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CounterfactualRate {
    pub input_per_mtok_cad: f64,
    pub output_per_mtok_cad: f64,
    #[serde(default)]
    pub note: Option<String>,
}

/// Result of `RateTable::compute()` — two cost columns per call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Costs {
    pub actual_cad: f64,
    pub counterfactual_cad: f64,
}

/// Map of `env -> provider -> model -> RateEntry`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RateTable {
    /// When the rates were last refreshed from upstream (RFC 3339).
    #[serde(default)]
    pub synced_at: Option<String>,
    #[serde(default)]
    pub rates: RateMap,
}

type RateMap = HashMap<String, HashMap<String, HashMap<String, RateEntry>>>;

impl RateTable {
    pub fn from_toml_str(s: &str) -> Result<Self> {
        Ok(toml::from_str(s)?)
    }

    pub fn from_path(p: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read_to_string(p)?;
        Self::from_toml_str(&bytes)
    }

    /// Compute (actual, counterfactual) cost for the given dimensions.
    /// Missing rate entries yield `RateNotFound` — callers decide whether to
    /// fall back to zero or surface the gap.
    pub fn compute(
        &self,
        env: Env,
        provider: &str,
        model: &str,
        tokens_in: i64,
        tokens_out: i64,
    ) -> Result<Costs> {
        let entry = self
            .rates
            .get(env.as_str())
            .and_then(|provs| provs.get(provider))
            .and_then(|mods| mods.get(model))
            .ok_or_else(|| ReceiptError::RateNotFound {
                env,
                provider: provider.to_string(),
                model: model.to_string(),
            })?;

        let m_in = tokens_in as f64 / 1_000_000.0;
        let m_out = tokens_out as f64 / 1_000_000.0;

        let actual = m_in * entry.input_per_mtok_cad + m_out * entry.output_per_mtok_cad;

        let counterfactual = match &entry.counterfactual {
            Some(cf) => m_in * cf.input_per_mtok_cad + m_out * cf.output_per_mtok_cad,
            None => actual, // No counterfactual specified — savings = 0.
        };

        Ok(Costs {
            actual_cad: actual,
            counterfactual_cad: counterfactual,
        })
    }
}

// ============================================================================
// Rollups — what the operator pages will read
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnvRollup {
    pub env: Env,
    pub calls: i64,
    pub tokens: i64,
    pub actual_cost_cad: f64,
    pub counterfactual_cost_cad: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRollup {
    pub agent: String,
    pub calls: i64,
    pub tokens: i64,
    pub actual_cost_cad: f64,
    pub counterfactual_cost_cad: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelRollup {
    pub provider: String,
    pub model: String,
    pub calls: i64,
    pub tokens: i64,
    pub actual_cost_cad: f64,
    pub counterfactual_cost_cad: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct DayTotal {
    pub tokens: i64,
    pub actual_cost_cad: f64,
    pub counterfactual_cost_cad: f64,
}

// ============================================================================
// Store
// ============================================================================

pub struct ReceiptStore {
    conn: Mutex<Connection>,
}

impl ReceiptStore {
    /// Open or create the store at `path`. Runs the v1 migration idempotently.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        Self::initialise(&conn)?;
        Ok(ReceiptStore {
            conn: Mutex::new(conn),
        })
    }

    /// In-memory store for tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::initialise(&conn)?;
        Ok(ReceiptStore {
            conn: Mutex::new(conn),
        })
    }

    fn initialise(conn: &Connection) -> Result<()> {
        conn.execute_batch(INITIAL_SQL)?;
        let mut found = read_schema_version(conn)?;
        if found < 2 {
            migrate_v2_hash_chain(conn)?;
            found = read_schema_version(conn)?;
        }
        if found < 3 {
            migrate_v3_model_call_accounting(conn)?;
            found = read_schema_version(conn)?;
        }
        if found != SCHEMA_VERSION {
            return Err(ReceiptError::SchemaVersion {
                found,
                expected: SCHEMA_VERSION,
            });
        }
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| ReceiptError::LockPoisoned)
    }

    /// Insert a single receipt, extending the tamper-evident chain.
    ///
    /// The store is the sole writer, so reading the current tip and appending
    /// the next link happen under one lock and one transaction — `seq` and
    /// `prev_hash` cannot race even under concurrent callers.
    pub fn insert(&self, r: &CallReceipt) -> Result<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;

        let (last_seq, last_hash) = tx
            .query_row(
                "SELECT seq, entry_hash FROM receipts ORDER BY seq DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .unwrap_or((0, GENESIS_HASH.to_string()));

        let seq = last_seq + 1;
        let entry = entry_hash(r, &last_hash);

        tx.execute(
            "INSERT INTO receipts (
                id, at, env, provider, model, agent,
                tokens_in, tokens_out, latency_ms,
                actual_cost_cad, counterfactual_cost_cad,
                model_call_cost_usd, model_call_cost_truth,
                model_call_attempts, failed_attempt_cost_usd,
                session_id, parent_id,
                seq, prev_hash, entry_hash
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9,
                ?10, ?11,
                ?12, ?13, ?14, ?15,
                ?16, ?17,
                ?18, ?19, ?20
            )",
            params![
                r.id,
                r.at,
                r.env.as_str(),
                r.provider,
                r.model,
                r.agent,
                r.tokens_in,
                r.tokens_out,
                r.latency_ms,
                r.actual_cost_cad,
                r.counterfactual_cost_cad,
                r.model_call_cost_usd,
                r.model_call_cost_truth,
                r.model_call_attempts,
                r.failed_attempt_cost_usd,
                r.session_id,
                r.parent_id,
                seq,
                last_hash,
                entry,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<CallReceipt>> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT * FROM receipts WHERE id = ?1",
                params![id],
                row_to_receipt,
            )
            .optional()?;
        Ok(row)
    }

    /// List receipts in `[since_unix, until_unix)`, most recent first.
    pub fn list(&self, since_unix: i64, until_unix: i64) -> Result<Vec<CallReceipt>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, at, env, provider, model, agent,
                    tokens_in, tokens_out, latency_ms,
                    actual_cost_cad, counterfactual_cost_cad,
                    model_call_cost_usd, model_call_cost_truth,
                    model_call_attempts, failed_attempt_cost_usd,
                    session_id, parent_id
             FROM receipts
             WHERE at >= ?1 AND at < ?2
             ORDER BY at DESC",
        )?;
        let rows = stmt
            .query_map(params![since_unix, until_unix], row_to_receipt)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn rollup_by_env(&self, since_unix: i64) -> Result<Vec<EnvRollup>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT env,
                    COUNT(*)                              AS calls,
                    SUM(tokens_in + tokens_out)           AS tokens,
                    COALESCE(SUM(actual_cost_cad),0)      AS actual,
                    COALESCE(SUM(counterfactual_cost_cad),0) AS counterfactual
             FROM receipts
             WHERE at >= ?1
             GROUP BY env
             ORDER BY tokens DESC",
        )?;
        let rows = stmt
            .query_map(params![since_unix], |row| {
                let env: String = row.get(0)?;
                Ok((
                    env,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut out = Vec::with_capacity(rows.len());
        for (env, calls, tokens, actual, counterfactual) in rows {
            out.push(EnvRollup {
                env: Env::parse(&env)?,
                calls,
                tokens,
                actual_cost_cad: actual,
                counterfactual_cost_cad: counterfactual,
            });
        }
        Ok(out)
    }

    pub fn rollup_by_agent(&self, since_unix: i64) -> Result<Vec<AgentRollup>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT agent,
                    COUNT(*),
                    SUM(tokens_in + tokens_out),
                    COALESCE(SUM(actual_cost_cad),0),
                    COALESCE(SUM(counterfactual_cost_cad),0)
             FROM receipts
             WHERE at >= ?1
             GROUP BY agent
             ORDER BY 3 DESC",
        )?;
        let rows = stmt
            .query_map(params![since_unix], |row| {
                Ok(AgentRollup {
                    agent: row.get(0)?,
                    calls: row.get(1)?,
                    tokens: row.get(2)?,
                    actual_cost_cad: row.get(3)?,
                    counterfactual_cost_cad: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn rollup_by_model(&self, since_unix: i64) -> Result<Vec<ModelRollup>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT provider, model,
                    COUNT(*),
                    SUM(tokens_in + tokens_out),
                    COALESCE(SUM(actual_cost_cad),0),
                    COALESCE(SUM(counterfactual_cost_cad),0)
             FROM receipts
             WHERE at >= ?1
             GROUP BY provider, model
             ORDER BY 4 DESC",
        )?;
        let rows = stmt
            .query_map(params![since_unix], |row| {
                Ok(ModelRollup {
                    provider: row.get(0)?,
                    model: row.get(1)?,
                    calls: row.get(2)?,
                    tokens: row.get(3)?,
                    actual_cost_cad: row.get(4)?,
                    counterfactual_cost_cad: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Aggregate totals since `since_unix` — the numbers the hero readout shows.
    pub fn day_total(&self, since_unix: i64) -> Result<DayTotal> {
        let conn = self.lock()?;
        let row = conn.query_row(
            "SELECT COALESCE(SUM(tokens_in + tokens_out),0),
                    COALESCE(SUM(actual_cost_cad),0),
                    COALESCE(SUM(counterfactual_cost_cad),0)
             FROM receipts
             WHERE at >= ?1",
            params![since_unix],
            |row| {
                Ok(DayTotal {
                    tokens: row.get(0)?,
                    actual_cost_cad: row.get(1)?,
                    counterfactual_cost_cad: row.get(2)?,
                })
            },
        )?;
        Ok(row)
    }

    pub fn schema_version(&self) -> Result<i64> {
        let conn = self.lock()?;
        let v: i64 = conn.query_row(
            "SELECT CAST(value AS INTEGER) FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )?;
        Ok(v)
    }

    /// The current chain tip — `entry_hash` of the newest receipt, or
    /// [`GENESIS_HASH`] when the store holds no receipts.
    pub fn head_hash(&self) -> Result<String> {
        let conn = self.lock()?;
        let head = conn
            .query_row(
                "SELECT entry_hash FROM receipts ORDER BY seq DESC LIMIT 1",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .unwrap_or_else(|| GENESIS_HASH.to_string());
        Ok(head)
    }

    /// Walk the ledger oldest-first and recompute every link. Returns
    /// [`ChainStatus::Intact`] with the verified length and tip, or
    /// [`ChainStatus::Broken`] at the first row that fails — which is exactly
    /// where the audit trail was tampered with.
    pub fn verify_chain(&self) -> Result<ChainStatus> {
        let conn = self.lock()?;
        verify_chain_with_version(&conn, ReceiptHashVersion::V3)
    }
}

fn verify_chain_with_version(
    conn: &Connection,
    version: ReceiptHashVersion,
) -> Result<ChainStatus> {
    let sql = match version {
        ReceiptHashVersion::LegacyV2 => {
            "SELECT id, at, env, provider, model, agent,
                    tokens_in, tokens_out, latency_ms,
                    actual_cost_cad, counterfactual_cost_cad,
                    session_id, parent_id,
                    seq, prev_hash, entry_hash
             FROM receipts
             ORDER BY seq ASC"
        }
        ReceiptHashVersion::V3 => {
            "SELECT id, at, env, provider, model, agent,
                    tokens_in, tokens_out, latency_ms,
                    actual_cost_cad, counterfactual_cost_cad,
                    model_call_cost_usd, model_call_cost_truth,
                    model_call_attempts, failed_attempt_cost_usd,
                    session_id, parent_id,
                    seq, prev_hash, entry_hash
             FROM receipts
             ORDER BY seq ASC"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;

    let mut prev = GENESIS_HASH.to_string();
    let mut len: u64 = 0;

    while let Some(row) = rows.next()? {
        let receipt = row_to_receipt(row)?;
        let seq: Option<i64> = row.get("seq")?;
        let stored_prev: Option<String> = row.get("prev_hash")?;
        let stored_entry: Option<String> = row.get("entry_hash")?;

        let (seq, stored_prev, stored_entry) = match (seq, stored_prev, stored_entry) {
            (Some(s), Some(p), Some(e)) => (s, p, e),
            _ => {
                return Ok(ChainStatus::Broken {
                    seq: seq.unwrap_or(-1),
                    id: receipt.id,
                    reason: ChainBreak::MissingHash,
                })
            }
        };

        if stored_prev != prev {
            return Ok(ChainStatus::Broken {
                seq,
                id: receipt.id,
                reason: ChainBreak::PrevMismatch,
            });
        }
        if entry_hash_for_version(&receipt, &prev, version) != stored_entry {
            return Ok(ChainStatus::Broken {
                seq,
                id: receipt.id,
                reason: ChainBreak::HashMismatch,
            });
        }

        prev = stored_entry;
        len += 1;
    }

    Ok(ChainStatus::Intact { len, head: prev })
}

fn read_schema_version(conn: &Connection) -> Result<i64> {
    let v: i64 = conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(1);
    Ok(v)
}

/// v1 → v2: add the chain columns, then backfill a hash chain over any rows that
/// existed before chaining (oldest first by `at`, ties broken by `id` for a
/// deterministic order). Called once, guarded by the schema-version check in
/// [`ReceiptStore::initialise`].
fn migrate_v2_hash_chain(conn: &Connection) -> Result<()> {
    conn.execute_batch(MIGRATION_0002_SQL)?;

    let rows: Vec<CallReceipt> = {
        let mut stmt = conn.prepare(
            "SELECT id, at, env, provider, model, agent,
                    tokens_in, tokens_out, latency_ms,
                    actual_cost_cad, counterfactual_cost_cad,
                    session_id, parent_id
             FROM receipts
             ORDER BY at ASC, id ASC",
        )?;
        let collected = stmt
            .query_map([], row_to_receipt)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        collected
    };

    let mut prev = GENESIS_HASH.to_string();
    for (i, r) in rows.iter().enumerate() {
        let seq = i as i64 + 1;
        let entry = entry_hash_for_version(r, &prev, ReceiptHashVersion::LegacyV2);
        conn.execute(
            "UPDATE receipts SET seq = ?1, prev_hash = ?2, entry_hash = ?3 WHERE id = ?4",
            params![seq, prev, entry, r.id],
        )?;
        prev = entry;
    }

    conn.execute(
        "UPDATE schema_meta SET value = '2' WHERE key = 'schema_version'",
        [],
    )?;
    Ok(())
}

/// v2 → v3: add executor USD accounting columns and rebuild the chain so new
/// nullable fields are covered by tamper evidence without relabeling CAD.
fn migrate_v3_model_call_accounting(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    match verify_chain_with_version(&tx, ReceiptHashVersion::LegacyV2)? {
        ChainStatus::Intact { .. } => {}
        ChainStatus::Broken { seq, id, reason } => {
            return Err(ReceiptError::BrokenV2Chain { seq, id, reason });
        }
    }

    tx.execute_batch(MIGRATION_0003_SQL)?;

    let rows: Vec<CallReceipt> = {
        let mut stmt = tx.prepare("SELECT * FROM receipts ORDER BY seq ASC")?;
        let collected = stmt
            .query_map([], row_to_receipt)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        collected
    };

    let mut prev = GENESIS_HASH.to_string();
    for (i, receipt) in rows.iter().enumerate() {
        let seq = i as i64 + 1;
        let entry = entry_hash_for_version(receipt, &prev, ReceiptHashVersion::V3);
        tx.execute(
            "UPDATE receipts SET seq = ?1, prev_hash = ?2, entry_hash = ?3 WHERE id = ?4",
            params![seq, prev, entry, receipt.id],
        )?;
        prev = entry;
    }

    tx.execute(
        "UPDATE schema_meta SET value = '3' WHERE key = 'schema_version'",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

fn optional_column<T: rusqlite::types::FromSql>(
    row: &rusqlite::Row<'_>,
    name: &str,
) -> rusqlite::Result<Option<T>> {
    match row.get::<_, Option<T>>(name) {
        Ok(value) => Ok(value),
        Err(rusqlite::Error::InvalidColumnName(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

fn row_to_receipt(row: &rusqlite::Row<'_>) -> rusqlite::Result<CallReceipt> {
    let env: String = row.get("env")?;
    Ok(CallReceipt {
        id: row.get("id")?,
        at: row.get("at")?,
        env: match env.as_str() {
            "local" => Env::Local,
            "oauth" => Env::Oauth,
            "api" => Env::Api,
            _ => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, env)),
                ))
            }
        },
        provider: row.get("provider")?,
        model: row.get("model")?,
        agent: row.get("agent")?,
        tokens_in: row.get("tokens_in")?,
        tokens_out: row.get("tokens_out")?,
        latency_ms: row.get("latency_ms")?,
        actual_cost_cad: row.get("actual_cost_cad")?,
        counterfactual_cost_cad: row.get("counterfactual_cost_cad")?,
        model_call_cost_usd: optional_column(row, "model_call_cost_usd")?,
        model_call_cost_truth: optional_column(row, "model_call_cost_truth")?,
        model_call_attempts: optional_column(row, "model_call_attempts")?,
        failed_attempt_cost_usd: optional_column(row, "failed_attempt_cost_usd")?,
        session_id: row.get("session_id")?,
        parent_id: row.get("parent_id")?,
    })
}

// ============================================================================
// Unit tests — pure logic only. End-to-end smoke lives in `tests/`.
// ============================================================================

// ============================================================================
// Runtime helpers — what callers at the shell/runtime boundary use
// ============================================================================

/// Convenience module so callers don't reinvent the rates loader + env mapping.
pub mod runtime {
    use super::*;

    /// Built-in fallback rates matching the marketing-demo conventions on
    /// heiwa.ltd. Operators override by writing `~/.heiwa/rates.toml`.
    pub fn default_rates() -> RateTable {
        const DEFAULT_TOML: &str = r#"
synced_at = "2026-05-25T00:00:00Z"

[rates.local.ollama."qwen3.5:9b"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.local.ollama."qwen3.5:9b".counterfactual]
input_per_mtok_cad  = 0.27
output_per_mtok_cad = 0.81
note = "Mistral 7B tier as fairness proxy"

[rates.local.ollama."qwen3.5:4b"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.local.ollama."qwen3.5:4b".counterfactual]
input_per_mtok_cad  = 0.14
output_per_mtok_cad = 0.42

[rates.local.ollama."gemma4"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.local.ollama."gemma4".counterfactual]
input_per_mtok_cad  = 0.27
output_per_mtok_cad = 0.81

[rates.oauth."claude-code"."claude-sonnet-4-6"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.oauth."claude-code"."claude-sonnet-4-6".counterfactual]
input_per_mtok_cad  = 4.05
output_per_mtok_cad = 20.25

[rates.oauth."claude-code"."claude-opus-4-7"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.oauth."claude-code"."claude-opus-4-7".counterfactual]
input_per_mtok_cad  = 20.25
output_per_mtok_cad = 101.25

[rates.oauth.codex."gpt-5-codex"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.oauth.codex."gpt-5-codex".counterfactual]
input_per_mtok_cad  = 2.75
output_per_mtok_cad = 11.00

[rates.oauth.gemini."gemini-3.1-pro"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.oauth.gemini."gemini-3.1-pro".counterfactual]
input_per_mtok_cad  = 1.69
output_per_mtok_cad = 6.75

[rates.api.openrouter."claude-3.7-sonnet"]
input_per_mtok_cad  = 4.05
output_per_mtok_cad = 20.25
"#;
        RateTable::from_toml_str(DEFAULT_TOML).expect("default rates parse")
    }

    /// Read `~/.heiwa/rates.toml` if present; otherwise return defaults.
    /// Parse failures silently fall back too — corrupt rate files should not
    /// stop the runtime from writing receipts.
    pub fn load_rates_or_default(heiwa_home: &std::path::Path) -> RateTable {
        let path = heiwa_home.join("rates.toml");
        if !path.exists() {
            return default_rates();
        }
        match RateTable::from_path(&path) {
            Ok(t) => t,
            Err(_) => default_rates(),
        }
    }

    /// Convention map: provider id -> environment lane.
    /// New providers default to `Api` (metered) so cost is never silently
    /// underreported.
    pub fn env_for_provider(provider: &str) -> Env {
        match provider {
            "ollama" | "local" => Env::Local,
            "claude-code" | "claude_code" | "codex-cli" | "codex_cli" | "codex" | "gemini-cli"
            | "gemini_cli" | "gemini" | "antigravity" => Env::Oauth,
            _ => Env::Api,
        }
    }

    /// Rough token estimator for adapters that don't report counts
    /// (the Ollama CLI subprocess being the main case today). ~3.7 chars/token
    /// is a common English approximation. **Best-effort.** Real implementation
    /// should call Ollama's HTTP API which reports `prompt_eval_count` and
    /// `eval_count` exactly.
    pub fn estimate_tokens(text: &str) -> i64 {
        let chars = text.chars().count() as f64;
        if chars == 0.0 {
            0
        } else {
            (chars / 3.7).ceil() as i64
        }
    }

    /// Compute costs with graceful zero-fallback when the rate entry is missing.
    /// Returns `(costs, found)` so callers can log unknown-rate cases without
    /// dropping the receipt.
    pub fn compute_or_zero(
        rates: &RateTable,
        env: Env,
        provider: &str,
        model: &str,
        tokens_in: i64,
        tokens_out: i64,
    ) -> (Costs, bool) {
        match rates.compute(env, provider, model, tokens_in, tokens_out) {
            Ok(c) => (c, true),
            Err(_) => (
                Costs {
                    actual_cad: 0.0,
                    counterfactual_cad: 0.0,
                },
                false,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_roundtrip() {
        for e in [Env::Local, Env::Oauth, Env::Api] {
            assert_eq!(Env::parse(e.as_str()).unwrap(), e);
        }
        assert!(matches!(
            Env::parse("invalid"),
            Err(ReceiptError::InvalidEnv(_))
        ));
    }

    #[test]
    fn cost_compute_oauth_zero_with_counterfactual() {
        let rates = r#"
            [rates.oauth.claude-code."claude-sonnet-4-6"]
            input_per_mtok_cad  = 0.0
            output_per_mtok_cad = 0.0

            [rates.oauth.claude-code."claude-sonnet-4-6".counterfactual]
            input_per_mtok_cad  = 4.05
            output_per_mtok_cad = 20.25
        "#;
        let table = RateTable::from_toml_str(rates).unwrap();
        let costs = table
            .compute(
                Env::Oauth,
                "claude-code",
                "claude-sonnet-4-6",
                1_000_000,
                200_000,
            )
            .unwrap();
        // 1M input @ 0 + 200k output @ 0 = 0 actual
        assert!((costs.actual_cad - 0.0).abs() < 1e-9);
        // Counterfactual: 1M * 4.05 + 0.2M * 20.25 = 4.05 + 4.05 = 8.10
        assert!((costs.counterfactual_cad - 8.10).abs() < 1e-9);
    }

    #[test]
    fn cost_compute_api_counterfactual_equals_actual_when_unset() {
        let rates = r#"
            [rates.api.openrouter."claude-3.7-sonnet"]
            input_per_mtok_cad  = 4.05
            output_per_mtok_cad = 20.25
        "#;
        let table = RateTable::from_toml_str(rates).unwrap();
        let costs = table
            .compute(Env::Api, "openrouter", "claude-3.7-sonnet", 1_000_000, 0)
            .unwrap();
        assert!((costs.actual_cad - 4.05).abs() < 1e-9);
        // No counterfactual entry → equals actual; savings = 0.
        assert!((costs.counterfactual_cad - 4.05).abs() < 1e-9);
    }

    #[test]
    fn cost_missing_rate_surfaces_error() {
        let table = RateTable::default();
        let err = table
            .compute(Env::Api, "nowhere", "no-model", 100, 100)
            .unwrap_err();
        assert!(matches!(err, ReceiptError::RateNotFound { .. }));
    }

    // ---- tamper-evident hash chain ----

    fn chain_sample(i: i64) -> CallReceipt {
        CallReceipt::new(
            1_716_640_000 + i,
            Env::Local,
            "ollama",
            "qwen3.5:9b",
            "coding",
            100 + i,
            10 + i,
            42,
            0.0,
            0.0,
            "sess-chain",
            None,
        )
    }

    #[test]
    fn entry_hash_is_deterministic_prev_sensitive_and_hex256() {
        let r = chain_sample(7);
        let h = entry_hash(&r, GENESIS_HASH);
        assert_eq!(
            h,
            entry_hash(&r, GENESIS_HASH),
            "pure: same inputs, same digest"
        );
        assert_eq!(h.len(), 64, "sha-256 is 32 bytes = 64 hex chars");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(
            h,
            entry_hash(&r, &"11".repeat(32)),
            "a different predecessor must change the digest"
        );
    }

    #[test]
    fn empty_store_head_is_genesis_and_verifies() {
        let store = ReceiptStore::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(store.head_hash().unwrap(), GENESIS_HASH);
        assert_eq!(
            store.verify_chain().unwrap(),
            ChainStatus::Intact {
                len: 0,
                head: GENESIS_HASH.to_string()
            }
        );
    }

    #[test]
    fn inserts_build_a_verifiable_chain() {
        let store = ReceiptStore::open_in_memory().unwrap();
        for i in 0..5 {
            store.insert(&chain_sample(i)).unwrap();
        }
        match store.verify_chain().unwrap() {
            ChainStatus::Intact { len, head } => {
                assert_eq!(len, 5);
                assert_ne!(head, GENESIS_HASH);
                assert_eq!(head, store.head_hash().unwrap());
            }
            other => panic!("expected intact chain, got {other:?}"),
        }
    }

    #[test]
    fn populated_v3_model_call_fields_round_trip_and_verify() {
        let store = ReceiptStore::open_in_memory().unwrap();
        let mut receipt = chain_sample(1);
        receipt.model_call_cost_usd = Some(0.031_25);
        receipt.model_call_cost_truth = Some("proxy_estimate".to_string());
        receipt.model_call_attempts = Some(2);
        receipt.failed_attempt_cost_usd = Some(0.01);

        store.insert(&receipt).unwrap();

        assert_eq!(store.get(&receipt.id).unwrap(), Some(receipt));
        assert!(matches!(
            store.verify_chain().unwrap(),
            ChainStatus::Intact { len: 1, .. }
        ));
    }

    fn seed_v2_database(path: &Path) {
        let raw = rusqlite::Connection::open(path).unwrap();
        raw.execute_batch(INITIAL_SQL).unwrap();
        for i in 0..2 {
            raw.execute(
                "INSERT INTO receipts
                   (id, at, env, provider, model, agent, tokens_in, tokens_out,
                    latency_ms, actual_cost_cad, counterfactual_cost_cad,
                    session_id, parent_id)
                 VALUES (?1, ?2, 'local', 'ollama', 'qwen3.5:9b', 'coding',
                         ?3, ?4, 42, 0.0, 0.0, 'sess-v2', NULL)",
                rusqlite::params![format!("v2-id-{i}"), 1_716_640_000_i64 + i, 100 + i, 10 + i],
            )
            .unwrap();
        }
        migrate_v2_hash_chain(&raw).unwrap();
        assert_eq!(read_schema_version(&raw).unwrap(), 2);
    }

    fn column_exists(conn: &rusqlite::Connection, name: &str) -> bool {
        let mut stmt = conn.prepare("PRAGMA table_info(receipts)").unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .any(|column| column == name)
    }

    #[test]
    fn tampered_v2_chain_is_rejected_without_schema_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tampered-v2.db");
        seed_v2_database(&path);

        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute("UPDATE receipts SET agent = 'forged' WHERE seq = 1", [])
            .unwrap();
        drop(raw);

        match ReceiptStore::open(&path) {
            Err(ReceiptError::BrokenV2Chain { seq, reason, .. }) => {
                assert_eq!(seq, 1);
                assert_eq!(reason, ChainBreak::HashMismatch);
            }
            Err(other) => panic!("expected broken-v2-chain error, got {other}"),
            Ok(_) => panic!("tampered v2 chain must not migrate"),
        }

        let raw = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(read_schema_version(&raw).unwrap(), 2);
        for column in [
            "model_call_cost_usd",
            "model_call_cost_truth",
            "model_call_attempts",
            "failed_attempt_cost_usd",
        ] {
            assert!(!column_exists(&raw, column));
        }
    }

    #[test]
    fn failed_v3_rehash_rolls_back_alters_and_reopens_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failed-v3.db");
        seed_v2_database(&path);

        {
            let raw = rusqlite::Connection::open(&path).unwrap();
            raw.execute_batch(
                "CREATE TEMP TRIGGER fail_v3_rehash
                 BEFORE UPDATE OF entry_hash ON receipts
                 BEGIN
                   SELECT RAISE(ABORT, 'injected v3 rehash failure');
                 END;",
            )
            .unwrap();

            assert!(migrate_v3_model_call_accounting(&raw).is_err());
            assert_eq!(read_schema_version(&raw).unwrap(), 2);
            for column in [
                "model_call_cost_usd",
                "model_call_cost_truth",
                "model_call_attempts",
                "failed_attempt_cost_usd",
            ] {
                assert!(!column_exists(&raw, column));
            }
        }

        let store = ReceiptStore::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 3);
        assert!(matches!(
            store.verify_chain().unwrap(),
            ChainStatus::Intact { len: 2, .. }
        ));
    }

    #[test]
    fn tampering_a_field_is_detected_at_its_seq() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipts.db");
        {
            let store = ReceiptStore::open(&path).unwrap();
            for i in 0..4 {
                store.insert(&chain_sample(i)).unwrap();
            }
        }

        // Forge a cost-bearing field directly in the DB, behind the store's back.
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute("UPDATE receipts SET agent = 'forged' WHERE seq = 2", [])
            .unwrap();
        drop(raw);

        let store = ReceiptStore::open(&path).unwrap();
        match store.verify_chain().unwrap() {
            ChainStatus::Broken { seq, reason, .. } => {
                assert_eq!(seq, 2);
                assert_eq!(reason, ChainBreak::HashMismatch);
            }
            other => panic!("expected a broken chain at seq 2, got {other:?}"),
        }
    }

    #[test]
    fn v1_database_is_backfilled_into_a_verifiable_chain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.db");

        // Forge a pre-chain (v1) database: schema + rows, schema_version = 1,
        // no chain columns.
        {
            let raw = rusqlite::Connection::open(&path).unwrap();
            raw.execute_batch(INITIAL_SQL).unwrap();
            for i in 0..3 {
                raw.execute(
                    "INSERT INTO receipts
                       (id, at, env, provider, model, agent, tokens_in, tokens_out,
                        latency_ms, actual_cost_cad, counterfactual_cost_cad,
                        session_id, parent_id)
                     VALUES (?1, ?2, 'local', 'ollama', 'qwen3.5:9b', 'coding',
                             ?3, ?4, 42, 0.0, 0.0, 'sess-v1', NULL)",
                    rusqlite::params![format!("id-{i}"), 1_716_640_000_i64 + i, 100 + i, 10 + i],
                )
                .unwrap();
            }
            let v: i64 = raw
                .query_row(
                    "SELECT CAST(value AS INTEGER) FROM schema_meta WHERE key = 'schema_version'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(v, 1, "fixture must start at schema v1");
        }

        // Opening through the store runs the v1 → v2 migration + backfill.
        let store = ReceiptStore::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 3);
        match store.verify_chain().unwrap() {
            ChainStatus::Intact { len, head } => {
                assert_eq!(len, 3, "all pre-existing rows should be chained");
                assert_ne!(head, GENESIS_HASH);
            }
            other => panic!("backfilled chain should verify, got {other:?}"),
        }
    }
}
