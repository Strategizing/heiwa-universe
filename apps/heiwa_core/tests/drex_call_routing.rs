use heiwa_core::drex::{
    default_policy, plan_model_call, plan_route, CallRisk, CandidateRejectionReason, CostTruth,
    DrexIngress, ExecutionLocality, ModelCallCandidate, ModelCallIdentity, ModelCallRequest,
    ModelCallStage, PrivacyClass, SafetyClass,
};
use heiwa_protocol::ModelTier;

fn request() -> ModelCallRequest {
    ModelCallRequest {
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        work_id: None,
        call_id: "call-1".to_string(),
        intent: "code".to_string(),
        stage: ModelCallStage::Execution,
        raw_text: "edit routing".to_string(),
        privacy: PrivacyClass::Standard,
        risk: CallRisk::Low,
        safety: SafetyClass::Approved,
        required_capabilities: vec!["advanced_coding".to_string()],
        required_context_tokens: 8_192,
        minimum_quality_class: 3,
        minimum_success_rate: 0.90,
        maximum_marginal_cost_usd: None,
        preferred_provider: None,
        preferred_model: None,
        allowed_models: Vec::new(),
        excluded_models: Vec::new(),
    }
}

fn candidate(
    id: u64,
    model_id: &str,
    provider: &str,
    quality: u8,
    locality: ExecutionLocality,
    cost: Option<f64>,
    cost_truth: CostTruth,
) -> ModelCallCandidate {
    ModelCallCandidate {
        tier: ModelTier {
            id,
            model_id: model_id.to_string(),
            provider_model_id: model_id.to_string(),
            provider: provider.to_string(),
            rate_group: provider.to_string(),
            capability_class: quality,
            effort_knob: "default".to_string(),
            effort_level: quality,
            cost_per_turn: cost.unwrap_or(0.0),
            max_context_tokens: 128_000,
            vram_requirement_mb: 4_096,
            quantization_type: "none".to_string(),
            kv_cache_strategy: "standard".to_string(),
            strengths_json: "[\"advanced_coding\",\"tool_use\"]".to_string(),
            enabled: true,
            last_success_rate: 0.98,
            avg_latency_ms: 500,
            latency_p_95_ms: 750,
            updated_at: "2026-07-19T00:00:00Z".to_string(),
        },
        locality,
        connected: true,
        adapter_capable: true,
        quota_available: true,
        marginal_cost_usd: cost,
        cost_truth,
    }
}

fn reason(plan: &heiwa_core::drex::ModelCallPlan, id: u64) -> CandidateRejectionReason {
    plan.rejected
        .iter()
        .find(|rejection| rejection.candidate_id == id)
        .unwrap_or_else(|| panic!("missing rejection for {id}"))
        .reasons[0]
        .clone()
}

#[test]
fn quality_admits_before_marginal_cost_and_preserves_cost_truth() {
    let local_low = candidate(
        1,
        "local-low",
        "ollama",
        1,
        ExecutionLocality::OnDevice,
        Some(0.0),
        CostTruth::LocalZeroCost,
    );
    let subscription = candidate(
        2,
        "subscription",
        "openai",
        3,
        ExecutionLocality::Remote,
        Some(0.0),
        CostTruth::TargetOnly,
    );
    let direct = candidate(
        3,
        "direct",
        "openai",
        3,
        ExecutionLocality::Remote,
        Some(0.08),
        CostTruth::ExactProviderReport,
    );

    let plan = plan_model_call(
        &request(),
        &[local_low, subscription.clone(), direct],
        &default_policy(),
    )
    .expect("planner state is valid");

    assert_eq!(plan.selected, Some(subscription));
    assert_eq!(plan.selected_cost_truth, Some(CostTruth::TargetOnly));
    assert_eq!(
        reason(&plan, 1),
        CandidateRejectionReason::MinimumQualityClass
    );
    assert_eq!(plan.admitted_ids, vec![2, 3]);
}

#[test]
fn each_hard_gate_reports_its_own_exact_reason() {
    let mut baseline = request();
    baseline.minimum_quality_class = 1;
    baseline.minimum_success_rate = 0.0;
    baseline.required_context_tokens = 1;
    baseline.required_capabilities.clear();
    let valid = candidate(
        1,
        "valid",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );

    let cases: Vec<(u64, ModelCallCandidate, CandidateRejectionReason)> = vec![
        (
            2,
            {
                let mut value = valid.clone();
                value.tier.id = 2;
                value.tier.enabled = false;
                value
            },
            CandidateRejectionReason::DisabledModel,
        ),
        (
            3,
            {
                let mut value = valid.clone();
                value.tier.id = 3;
                value.connected = false;
                value
            },
            CandidateRejectionReason::Disconnected,
        ),
        (
            4,
            {
                let mut value = valid.clone();
                value.tier.id = 4;
                value.adapter_capable = false;
                value
            },
            CandidateRejectionReason::AdapterIncapable,
        ),
        (
            5,
            {
                let mut value = valid.clone();
                value.tier.id = 5;
                value.quota_available = false;
                value
            },
            CandidateRejectionReason::QuotaExhausted,
        ),
        (
            6,
            {
                let mut value = valid.clone();
                value.tier.id = 6;
                value.tier.max_context_tokens = 0;
                value
            },
            CandidateRejectionReason::InsufficientContext,
        ),
        (
            7,
            {
                let mut value = valid.clone();
                value.tier.id = 7;
                value.tier.strengths_json = "[]".to_string();
                value
            },
            CandidateRejectionReason::MissingRequiredCapability,
        ),
        (
            8,
            {
                let mut value = valid.clone();
                value.tier.id = 8;
                value.tier.capability_class = 0;
                value
            },
            CandidateRejectionReason::MinimumQualityClass,
        ),
        (
            9,
            {
                let mut value = valid.clone();
                value.tier.id = 9;
                value.tier.last_success_rate = 0.5;
                value
            },
            CandidateRejectionReason::MinimumSuccessRate,
        ),
    ];

    for (id, rejected, expected) in cases {
        let mut scoped = baseline.clone();
        if id == 7 {
            scoped.required_capabilities = vec!["tool_use".to_string()];
        }
        if id == 8 {
            scoped.minimum_quality_class = 3;
        }
        if id == 9 {
            scoped.minimum_success_rate = 0.9;
        }
        let plan = plan_model_call(&scoped, &[valid.clone(), rejected], &default_policy()).unwrap();
        assert_eq!(reason(&plan, id), expected, "candidate {id}");
    }
}

#[test]
fn explicit_gates_and_privacy_do_not_fail_open() {
    let valid = candidate(
        1,
        "allowed",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );

    let mut excluded = request();
    excluded.excluded_models = vec!["allowed".to_string()];
    let plan = plan_model_call(&excluded, std::slice::from_ref(&valid), &default_policy()).unwrap();
    assert_eq!(reason(&plan, 1), CandidateRejectionReason::ExcludedModel);

    let mut allowed = request();
    allowed.allowed_models = vec!["other".to_string()];
    let plan = plan_model_call(&allowed, std::slice::from_ref(&valid), &default_policy()).unwrap();
    assert_eq!(reason(&plan, 1), CandidateRejectionReason::NotAllowedModel);

    let mut budget = request();
    budget.maximum_marginal_cost_usd = Some(0.001);
    let plan = plan_model_call(&budget, std::slice::from_ref(&valid), &default_policy()).unwrap();
    assert_eq!(
        reason(&plan, 1),
        CandidateRejectionReason::MaximumMarginalCostUsd
    );

    let mut provider_pin = request();
    provider_pin.preferred_provider = Some("other".to_string());
    let plan = plan_model_call(
        &provider_pin,
        std::slice::from_ref(&valid),
        &default_policy(),
    )
    .unwrap();
    assert_eq!(
        reason(&plan, 1),
        CandidateRejectionReason::PreferredProviderMismatch
    );

    let mut model_pin = request();
    model_pin.preferred_model = Some("other".to_string());
    let plan =
        plan_model_call(&model_pin, std::slice::from_ref(&valid), &default_policy()).unwrap();
    assert_eq!(
        reason(&plan, 1),
        CandidateRejectionReason::PreferredModelMismatch
    );

    let mut local_only = request();
    local_only.privacy = PrivacyClass::LocalOnly;
    let plan =
        plan_model_call(&local_only, std::slice::from_ref(&valid), &default_policy()).unwrap();
    assert_eq!(
        reason(&plan, 1),
        CandidateRejectionReason::LocalOnlyOnDeviceRequired
    );

    let mut sovereign = request();
    sovereign.privacy = PrivacyClass::Sovereign;
    let litellm_remote = candidate(
        2,
        "litellm",
        "litellm",
        3,
        ExecutionLocality::Remote,
        Some(0.0),
        CostTruth::TargetOnly,
    );
    let plan = plan_model_call(&sovereign, &[litellm_remote], &default_policy()).unwrap();
    assert_eq!(
        reason(&plan, 2),
        CandidateRejectionReason::SovereignLocalityRequired
    );
}

#[test]
fn safety_block_and_empty_or_rejected_input_return_no_route_evidence() {
    let mut blocked = request();
    blocked.safety = SafetyClass::Blocked;
    let plan = plan_model_call(&blocked, &[], &default_policy()).unwrap();
    assert_eq!(plan.selected, None);
    assert_eq!(plan.selection_reason, "safety_forbids_execution");
    assert!(!plan.decision.gate.authority_required.is_empty());
    assert_eq!(plan.thread_id, "thread-1");

    let plan = plan_model_call(&request(), &[], &default_policy()).unwrap();
    assert_eq!(plan.selected, None);
    assert_eq!(plan.selection_reason, "no_admitted_model_call_candidates");
    assert!(plan.rejected.is_empty());
}

#[test]
fn authority_gate_blocks_unapproved_high_risk_execution_before_selection() {
    let mut governed = request();
    governed.intent = "strategy".to_string();
    governed.risk = CallRisk::High;
    governed.safety = SafetyClass::Unapproved;
    let candidate = candidate(
        1,
        "candidate",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.0),
        CostTruth::TargetOnly,
    );

    let plan = plan_model_call(&governed, &[candidate], &default_policy()).unwrap();

    assert_eq!(plan.selected, None);
    assert_eq!(plan.selection_reason, "authority_approval_required");
    assert_eq!(
        reason(&plan, 1),
        CandidateRejectionReason::AuthorityApprovalRequired
    );
    assert!(plan.decision.gate.requires_approval);
}

#[test]
fn invalid_cost_truth_and_unknown_cost_never_win_known_zero() {
    let known = candidate(
        1,
        "known",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.0),
        CostTruth::TargetOnly,
    );
    let unknown = candidate(
        2,
        "unknown",
        "provider",
        3,
        ExecutionLocality::Remote,
        None,
        CostTruth::CannotConfirm,
    );
    let invalid_local = candidate(
        3,
        "invalid-local",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.0),
        CostTruth::LocalZeroCost,
    );
    let invalid_exact = candidate(
        4,
        "invalid-exact",
        "provider",
        3,
        ExecutionLocality::Remote,
        None,
        CostTruth::ExactProviderReport,
    );
    let invalid_target = candidate(
        5,
        "invalid-target",
        "provider",
        3,
        ExecutionLocality::Remote,
        None,
        CostTruth::TargetOnly,
    );
    let plan = plan_model_call(
        &request(),
        &[
            unknown,
            invalid_local,
            invalid_exact,
            invalid_target,
            known.clone(),
        ],
        &default_policy(),
    )
    .unwrap();
    assert_eq!(plan.selected, Some(known));
    assert_eq!(reason(&plan, 3), CandidateRejectionReason::InvalidCostTruth);
    assert_eq!(reason(&plan, 4), CandidateRejectionReason::InvalidCostTruth);
    assert_eq!(reason(&plan, 5), CandidateRejectionReason::InvalidCostTruth);
}

#[test]
fn non_finite_or_negative_metrics_have_exact_rejection_reasons() {
    let valid = candidate(
        1,
        "valid",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );
    let nan_cost = candidate(
        2,
        "nan-cost",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(f64::NAN),
        CostTruth::ProxyEstimate,
    );
    let negative_cost = candidate(
        3,
        "negative-cost",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(-0.01),
        CostTruth::ProxyEstimate,
    );
    let mut nan_success = candidate(
        4,
        "nan-success",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );
    nan_success.tier.last_success_rate = f64::NAN;
    let mut negative_success = candidate(
        5,
        "negative-success",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );
    negative_success.tier.last_success_rate = -0.1;
    let plan = plan_model_call(
        &request(),
        &[
            valid,
            nan_cost,
            negative_cost,
            nan_success,
            negative_success,
        ],
        &default_policy(),
    )
    .unwrap();
    assert_eq!(
        reason(&plan, 2),
        CandidateRejectionReason::InvalidMarginalCostUsd
    );
    assert_eq!(
        reason(&plan, 3),
        CandidateRejectionReason::InvalidMarginalCostUsd
    );
    assert_eq!(
        reason(&plan, 4),
        CandidateRejectionReason::InvalidSuccessRate
    );
    assert_eq!(
        reason(&plan, 5),
        CandidateRejectionReason::InvalidSuccessRate
    );
}

#[test]
fn duplicate_ids_reject_independently_of_input_order() {
    let first = candidate(
        7,
        "first",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );
    let second = candidate(
        7,
        "second",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.02),
        CostTruth::ProxyEstimate,
    );
    let forward = plan_model_call(
        &request(),
        &[first.clone(), second.clone()],
        &default_policy(),
    )
    .unwrap();
    let reverse = plan_model_call(&request(), &[second, first], &default_policy()).unwrap();
    assert_eq!(forward.selected, None);
    assert_eq!(forward.rejected, reverse.rejected);
    assert_eq!(
        reason(&forward, 7),
        CandidateRejectionReason::DuplicateCandidateId
    );
}

#[test]
fn full_tie_order_is_cost_quality_latency_success_then_id() {
    let mut costly = candidate(
        5,
        "costly",
        "p",
        4,
        ExecutionLocality::Remote,
        Some(0.02),
        CostTruth::ProxyEstimate,
    );
    let mut low_quality = candidate(
        4,
        "low-quality",
        "p",
        3,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );
    let mut slow = candidate(
        3,
        "slow",
        "p",
        4,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );
    let mut low_success = candidate(
        2,
        "low-success",
        "p",
        4,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );
    let id_tiebreak = candidate(
        1,
        "id",
        "p",
        4,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    );
    costly.tier.latency_p_95_ms = 1;
    low_quality.tier.latency_p_95_ms = 1;
    slow.tier.latency_p_95_ms = 900;
    low_success.tier.last_success_rate = 0.97;
    let plan = plan_model_call(
        &request(),
        &[costly, low_quality, slow, low_success, id_tiebreak],
        &default_policy(),
    )
    .unwrap();
    assert_eq!(plan.admitted_ids, vec![1, 2, 3, 4, 5]);
}

#[test]
fn legacy_wrapper_rejects_privacy_typo_and_remote_local_runtime() {
    let mut tier = candidate(
        1,
        "remote",
        "remote",
        3,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    )
    .tier;
    tier.vram_requirement_mb = 0;
    let typo = DrexIngress {
        intent: "code".to_string(),
        risk: "low".to_string(),
        raw_text: "edit".to_string(),
        privacy: "sovreign".to_string(),
        runtime: "local".to_string(),
        available_vram_mb: 8_192,
        required_context_tokens: 1,
    };
    let route = plan_route(&typo, std::slice::from_ref(&tier), &default_policy()).unwrap();
    assert_eq!(route.selected_model, None);
    assert!(route.routing_metadata.contains("invalid_privacy_class"));

    let local = DrexIngress {
        privacy: "standard".to_string(),
        ..typo
    };
    let route = plan_route(&local, &[tier], &default_policy()).unwrap();
    assert_eq!(route.selected_model, None);
    assert!(route
        .routing_metadata
        .contains("local_runtime_locality_required"));
    assert_eq!(
        PrivacyClass::parse("sovreign"),
        Err("invalid_privacy_class")
    );
}

#[test]
fn legacy_sovereign_code_keeps_advanced_coding_or_quality_floor() {
    let mut advanced = candidate(
        1,
        "advanced",
        "ollama",
        1,
        ExecutionLocality::OnDevice,
        Some(0.0),
        CostTruth::LocalZeroCost,
    )
    .tier;
    advanced.rate_group = "local_ollama".to_string();
    let mut high_quality = candidate(
        2,
        "high-quality",
        "ollama",
        3,
        ExecutionLocality::OnDevice,
        Some(0.0),
        CostTruth::LocalZeroCost,
    )
    .tier;
    high_quality.rate_group = "local_ollama".to_string();
    high_quality.strengths_json = "[]".to_string();
    let ingress = DrexIngress {
        intent: "code".to_string(),
        risk: "low".to_string(),
        raw_text: "edit source".to_string(),
        privacy: "sovereign".to_string(),
        runtime: "local".to_string(),
        available_vram_mb: 8_192,
        required_context_tokens: 1,
    };

    let route = plan_route(&ingress, &[advanced, high_quality], &default_policy()).unwrap();

    assert_eq!(
        route.selected_model.expect("selected").model_id,
        "high-quality"
    );
    assert!(route.routing_metadata.contains("\"admitted_ids\":[2,1]"));
}

#[test]
fn execution_stages_fail_closed_and_unknown_stage_is_invalid() {
    for stage in [
        ModelCallStage::Compression,
        ModelCallStage::Drafting,
        ModelCallStage::Execution,
        ModelCallStage::LoopIteration,
    ] {
        let mut blocked = request();
        blocked.stage = stage.clone();
        blocked.safety = SafetyClass::Blocked;
        let blocked_candidate = candidate(
            1,
            "candidate",
            "provider",
            3,
            ExecutionLocality::Remote,
            Some(0.0),
            CostTruth::TargetOnly,
        );
        let plan = plan_model_call(&blocked, &[blocked_candidate], &default_policy()).unwrap();
        assert_eq!(
            reason(&plan, 1),
            CandidateRejectionReason::SafetyForbidsExecution
        );

        let mut unapproved = request();
        unapproved.stage = stage;
        unapproved.intent = "strategy".to_string();
        unapproved.risk = CallRisk::High;
        unapproved.safety = SafetyClass::Unapproved;
        let authority_candidate = candidate(
            2,
            "candidate",
            "provider",
            3,
            ExecutionLocality::Remote,
            Some(0.0),
            CostTruth::TargetOnly,
        );
        let plan = plan_model_call(&unapproved, &[authority_candidate], &default_policy()).unwrap();
        assert_eq!(
            reason(&plan, 2),
            CandidateRejectionReason::AuthorityApprovalRequired
        );
    }
    assert_eq!(
        ModelCallStage::parse("wire_unknown"),
        Err("invalid_model_call_stage")
    );
    assert!(serde_json::from_str::<ModelCallStage>("\"wire_unknown\"").is_err());
    assert_eq!(
        ModelCallStage::parse("compression"),
        Ok(ModelCallStage::Compression)
    );
    assert_eq!(
        ModelCallStage::parse("drafting"),
        Ok(ModelCallStage::Drafting)
    );
}

#[test]
fn local_legacy_vram_gate_applies_even_when_runtime_is_any() {
    let mut tier = candidate(
        1,
        "local",
        "ollama",
        3,
        ExecutionLocality::OnDevice,
        Some(0.0),
        CostTruth::LocalZeroCost,
    )
    .tier;
    tier.rate_group = "local_ollama".to_string();
    tier.vram_requirement_mb = 32_768;
    let ingress = DrexIngress {
        intent: "code".to_string(),
        risk: "low".to_string(),
        raw_text: "edit".to_string(),
        privacy: "sovereign".to_string(),
        runtime: "any".to_string(),
        available_vram_mb: 8_192,
        required_context_tokens: 1,
    };
    let rejected = plan_route(&ingress, std::slice::from_ref(&tier), &default_policy()).unwrap();
    assert_eq!(rejected.selected_model, None);
    assert!(rejected.routing_metadata.contains("adapter_incapable"));

    tier.vram_requirement_mb = 4_096;
    let accepted = plan_route(&ingress, &[tier], &default_policy()).unwrap();
    assert_eq!(accepted.selected_model.expect("selected").model_id, "local");
}

#[test]
fn legacy_call_identity_is_unique_or_round_trips_when_supplied() {
    let tier = candidate(
        1,
        "remote",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.01),
        CostTruth::ProxyEstimate,
    )
    .tier;
    let ingress = DrexIngress {
        intent: "code".to_string(),
        risk: "low".to_string(),
        raw_text: "edit".to_string(),
        privacy: "standard".to_string(),
        runtime: "any".to_string(),
        available_vram_mb: 0,
        required_context_tokens: 1,
    };
    let first = plan_route(&ingress, std::slice::from_ref(&tier), &default_policy()).unwrap();
    let second = plan_route(&ingress, std::slice::from_ref(&tier), &default_policy()).unwrap();
    assert_ne!(first.call_id, second.call_id);
    assert_eq!(first.stage, ModelCallStage::LegacyRoute);

    let identity = ModelCallIdentity {
        thread_id: "thread-x".to_string(),
        turn_id: "turn-y".to_string(),
        call_id: "call-z".to_string(),
    };
    let routed = heiwa_core::drex::plan_route_for_call(
        &ingress,
        &[tier],
        &default_policy(),
        identity.clone(),
    )
    .unwrap();
    assert_eq!(routed.thread_id, identity.thread_id);
    assert_eq!(routed.turn_id, identity.turn_id);
    assert_eq!(routed.call_id, identity.call_id);
    assert!(routed.routing_metadata.contains("call-z"));
}

#[test]
fn empty_direct_identity_returns_invalid_policy_plan() {
    let mut request = request();
    request.thread_id.clear();
    let candidate = candidate(
        1,
        "candidate",
        "provider",
        3,
        ExecutionLocality::Remote,
        Some(0.0),
        CostTruth::TargetOnly,
    );
    let plan = plan_model_call(&request, &[candidate], &default_policy()).unwrap();
    assert_eq!(plan.selected, None);
    assert_eq!(
        reason(&plan, 1),
        CandidateRejectionReason::InvalidRequestPolicy
    );
}
