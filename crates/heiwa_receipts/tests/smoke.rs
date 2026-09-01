//! End-to-end smoke test for `heiwa_receipts`.
//!
//! Mirrors the cost-attribution ledger demoed on heiwa.ltd:
//!   local · ollama · qwen3.5:9b · coding         92.4k tokens · 0.00 CAD
//!   oauth · claude-code · claude-sonnet-4-6 · strategy  47.1k · 0.00 CAD
//!   oauth · codex · gpt-5-codex · refactor       28.9k · 0.00 CAD
//!   api   · openrouter · claude-3.7-sonnet · trading    12.0k · 1.84 CAD
//!   local · ollama · qwen3.5:4b · summarise       4.2k · 0.00 CAD
//!
//! Total: 184.6k tokens · 1.84 CAD actual (counterfactual delta varies by rates).

use heiwa_receipts::{CallReceipt, ChainStatus, Costs, Env, RateTable, ReceiptStore, GENESIS_HASH};
use tempfile::TempDir;

const RATES_TOML: &str = r#"
synced_at = "2026-05-25T11:00:00Z"

[rates.local.ollama."qwen3.5:9b"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.local.ollama."qwen3.5:9b".counterfactual]
input_per_mtok_cad  = 0.27
output_per_mtok_cad = 0.81

[rates.local.ollama."qwen3.5:4b"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.local.ollama."qwen3.5:4b".counterfactual]
input_per_mtok_cad  = 0.14
output_per_mtok_cad = 0.42

[rates.oauth."claude-code"."claude-sonnet-4-6"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.oauth."claude-code"."claude-sonnet-4-6".counterfactual]
input_per_mtok_cad  = 4.05
output_per_mtok_cad = 20.25

[rates.oauth.codex."gpt-5-codex"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
[rates.oauth.codex."gpt-5-codex".counterfactual]
input_per_mtok_cad  = 2.75
output_per_mtok_cad = 11.00

[rates.api.openrouter."claude-3.7-sonnet"]
input_per_mtok_cad  = 4.05
output_per_mtok_cad = 20.25
"#;

/// Build the marketing-demo dataset as actual Receipts in the store.
fn seed(store: &ReceiptStore, rates: &RateTable, session: &str) {
    let entries = [
        (
            Env::Local,
            "ollama",
            "qwen3.5:9b",
            "coding",
            80_000_i64,
            12_400_i64,
        ),
        (
            Env::Oauth,
            "claude-code",
            "claude-sonnet-4-6",
            "strategy",
            40_000,
            7_100,
        ),
        (
            Env::Oauth,
            "codex",
            "gpt-5-codex",
            "refactor",
            24_000,
            4_900,
        ),
        (
            Env::Api,
            "openrouter",
            "claude-3.7-sonnet",
            "trading",
            9_000,
            3_000,
        ),
        (Env::Local, "ollama", "qwen3.5:4b", "summarise", 3_500, 700),
    ];

    for (i, (env, provider, model, agent, tin, tout)) in entries.iter().enumerate() {
        let Costs {
            actual_cad,
            counterfactual_cad,
        } = rates
            .compute(*env, provider, model, *tin, *tout)
            .expect("rate lookup");
        let r = CallReceipt::new(
            1_716_640_000 + i as i64 * 60, // 60s apart
            *env,
            *provider,
            *model,
            *agent,
            *tin,
            *tout,
            40 + (i as i64) * 5,
            actual_cad,
            counterfactual_cad,
            session,
            None,
        );
        store.insert(&r).expect("insert");
    }
}

#[test]
fn full_marketing_demo_roundtrip() {
    let dir = TempDir::new().unwrap();
    let store = ReceiptStore::open(dir.path().join("receipts.db")).unwrap();
    let rates = RateTable::from_toml_str(RATES_TOML).unwrap();

    seed(&store, &rates, "sess-2026-05-25");

    // Day totals — matches the hero fiducial readout
    let total = store.day_total(0).unwrap();
    assert_eq!(total.tokens, 184_600, "total tokens should be 184.6k");
    // API row: 9k input * 4.05/M + 3k output * 20.25/M = 0.03645 + 0.06075 = 0.0972 CAD
    // (Marketing demo shows 1.84 CAD because token counts are illustrative;
    //  the crate computes from real rates, not the demo string.)
    let api_actual = (9_000.0 / 1_000_000.0) * 4.05 + (3_000.0 / 1_000_000.0) * 20.25;
    assert!(
        (total.actual_cost_cad - api_actual).abs() < 1e-6,
        "actual_cost_cad should equal the API row's computed cost (others are 0): got {} expected {}",
        total.actual_cost_cad,
        api_actual
    );
    assert!(
        total.counterfactual_cost_cad > total.actual_cost_cad,
        "counterfactual must exceed actual when oauth/local lanes have counterfactual rates set"
    );

    // By env — 3 envs, ordered by token volume
    let by_env = store.rollup_by_env(0).unwrap();
    assert_eq!(by_env.len(), 3, "expected three env buckets");

    // Local has the most tokens in our seed
    assert_eq!(by_env[0].env, Env::Local);
    // Local actual must be zero (all oauth + local entries have zero actual rate)
    assert!((by_env[0].actual_cost_cad - 0.0).abs() < 1e-9);

    // By agent — 5 distinct agents
    let by_agent = store.rollup_by_agent(0).unwrap();
    assert_eq!(by_agent.len(), 5);

    // By model — 5 distinct provider+model pairs
    let by_model = store.rollup_by_model(0).unwrap();
    assert_eq!(by_model.len(), 5);

    // List — most recent first; we inserted 5
    let list = store.list(0, i64::MAX).unwrap();
    assert_eq!(list.len(), 5);
    assert!(list[0].at > list[4].at, "list should be DESC by at");

    // Get — round-trip
    let one = list[0].clone();
    let fetched = store.get(&one.id).unwrap().expect("receipt exists");
    assert_eq!(fetched, one);

    // Header — the exportable subset still has costs and token counts
    let header = one.header();
    assert_eq!(header.tokens_in, one.tokens_in);
    assert_eq!(header.tokens_out, one.tokens_out);
    assert_eq!(header.actual_cost_cad, one.actual_cost_cad);
    assert_eq!(header.schema_version, 3);

    // Evidence plane: the seeded ledger forms an intact tamper-evident chain.
    assert_ne!(store.head_hash().unwrap(), GENESIS_HASH);
    assert_eq!(
        store.verify_chain().unwrap(),
        ChainStatus::Intact {
            len: 5,
            head: store.head_hash().unwrap(),
        }
    );
}

#[test]
fn schema_version_is_three() {
    let store = ReceiptStore::open_in_memory().unwrap();
    assert_eq!(store.schema_version().unwrap(), 3);
}

#[test]
fn empty_store_returns_zeros_not_errors() {
    let store = ReceiptStore::open_in_memory().unwrap();
    let total = store.day_total(0).unwrap();
    assert_eq!(total.tokens, 0);
    assert_eq!(total.actual_cost_cad, 0.0);
    assert_eq!(total.counterfactual_cost_cad, 0.0);

    assert!(store.rollup_by_env(0).unwrap().is_empty());
    assert!(store.rollup_by_agent(0).unwrap().is_empty());
    assert!(store.rollup_by_model(0).unwrap().is_empty());
}

#[test]
fn the_former_receipt_name_still_resolves() {
    // The rename must not break a consumer mid-migration. When the alias is
    // finally removed, this test is the thing that fails first — which is the
    // point: the removal should be a decision, not a surprise.
    #[allow(deprecated)]
    let legacy: heiwa_receipts::Receipt = CallReceipt::new(
        1_700_000_000,
        Env::Api,
        "anthropic",
        "claude-opus-5",
        "reviewer",
        100,
        50,
        420,
        0.10,
        0.00,
        "session-alias",
        None,
    );
    assert_eq!(legacy.provider, "anthropic");

    // The two names denote one type, so a value built through either is
    // interchangeable.
    let current: CallReceipt = legacy.clone();
    assert_eq!(current, legacy);
}

#[test]
fn a_call_receipt_carries_no_external_effect_evidence() {
    // Guards publication gate 1 of the Work Continuity design. If someone adds
    // a target, idempotency key, or verification field to this row, the split
    // has been quietly collapsed and this test should be the thing that argues.
    let receipt = CallReceipt::new(
        1_700_000_000,
        Env::Api,
        "anthropic",
        "claude-opus-5",
        "reviewer",
        100,
        50,
        420,
        0.10,
        0.00,
        "session-effect",
        None,
    );
    let json = serde_json::to_value(&receipt).expect("serializes");
    let row = json.as_object().expect("object");
    for effect_field in [
        "effect_kind",
        "target_ref",
        "idempotency_key",
        "verification",
        "compensation",
        "external_refs",
    ] {
        assert!(
            !row.contains_key(effect_field),
            "`{effect_field}` belongs to an Effect Receipt, not to call accounting"
        );
    }
}
