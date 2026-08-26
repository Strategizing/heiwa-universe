use anyhow::Result;
use heiwa_protocol::ModelTier;

use super::call::{
    plan_model_call, CallRisk, CandidateRejection, CandidateRejectionReason, CostTruth,
    ExecutionLocality, ModelCallCandidate, ModelCallIdentity, ModelCallRequest, ModelCallStage,
    PrivacyClass, SafetyClass,
};
use super::policy::{DrexDecision, DrexPolicy, ExecutionMode, ResolutionTier};
use super::scorer::evaluate_drex;
use super::vector::DrexVector;

use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct DrexIngress {
    pub intent: String,
    pub risk: String,
    pub raw_text: String,
    pub privacy: String,
    pub runtime: String,
    pub available_vram_mb: u32,
    pub required_context_tokens: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoutePlan {
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
    pub stage: ModelCallStage,
    pub decision: DrexDecision,
    pub execution_mode: ExecutionMode,
    pub runtime_hint: String,
    pub selected_model: Option<ModelTier>,
    pub routing_metadata: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreflightDecision {
    pub execution_mode: ExecutionMode,
    pub response_text: Option<String>,
    pub reason: String,
}

pub fn preflight_execution(
    ingress: &DrexIngress,
    model_tiers: &[ModelTier],
    policy: &DrexPolicy,
) -> PreflightDecision {
    let trimmed = ingress.raw_text.trim();
    if trimmed.is_empty() {
        return PreflightDecision {
            execution_mode: ExecutionMode::Clarify,
            response_text: Some(
                "Tell me the outcome you want, or use /status, /providers, or /models.".to_string(),
            ),
            reason: "empty_input".to_string(),
        };
    }

    let lowercase = trimmed.to_ascii_lowercase();
    if is_greeting(&lowercase) {
        return PreflightDecision {
            execution_mode: ExecutionMode::Deterministic,
            response_text: Some(
                "Ready. Tell me what you want to do, or use /status, /providers, or /models."
                    .to_string(),
            ),
            reason: "greeting".to_string(),
        };
    }

    if is_underspecified(&lowercase) {
        return PreflightDecision {
            execution_mode: ExecutionMode::Clarify,
            response_text: Some(
                "Tell me the outcome you want, the repo or file target, or the command context."
                    .to_string(),
            ),
            reason: "underspecified".to_string(),
        };
    }

    let vector = build_drex_vector(
        &ingress.intent,
        &ingress.risk,
        &ingress.raw_text,
        &ingress.privacy,
        &ingress.runtime,
    );
    let runtime_hint = runtime_hint(ingress, &vector);
    let runtime_fit = if is_local_runtime(&runtime_hint) {
        1.0
    } else {
        0.8
    };
    let decision = evaluate_drex(&vector, policy, 0.95, runtime_fit, 0.65);
    let local_candidates: Vec<&ModelTier> = model_tiers
        .iter()
        .filter(|tier| tier.enabled && is_local_provider(&tier.provider))
        .collect();
    let remote_candidates: Vec<&ModelTier> = model_tiers
        .iter()
        .filter(|tier| tier.enabled && !is_local_provider(&tier.provider))
        .collect();

    let execution_mode = if should_prefer_local(ingress, &decision, &local_candidates)
        || remote_candidates.is_empty()
    {
        ExecutionMode::LocalModel
    } else {
        ExecutionMode::RemoteModel
    };

    PreflightDecision {
        execution_mode,
        response_text: None,
        reason: match execution_mode {
            ExecutionMode::LocalModel => "local_first".to_string(),
            ExecutionMode::RemoteModel => "remote_escalation".to_string(),
            ExecutionMode::Deterministic => "deterministic".to_string(),
            ExecutionMode::Clarify => "clarify".to_string(),
        },
    }
}

pub fn plan_route(
    ingress: &DrexIngress,
    model_tiers: &[ModelTier],
    policy: &DrexPolicy,
) -> Result<RoutePlan> {
    plan_route_for_call(
        ingress,
        model_tiers,
        policy,
        ModelCallIdentity::legacy_uuid(),
    )
}

pub fn plan_route_for_call(
    ingress: &DrexIngress,
    model_tiers: &[ModelTier],
    policy: &DrexPolicy,
    identity: ModelCallIdentity,
) -> Result<RoutePlan> {
    let privacy = match PrivacyClass::parse(&ingress.privacy) {
        Ok(privacy) => privacy,
        Err(reason) => {
            return Ok(legacy_no_route(
                ingress,
                model_tiers,
                policy,
                &identity,
                reason,
            ));
        }
    };
    let risk = match CallRisk::parse(&ingress.risk) {
        Ok(risk) => risk,
        Err(reason) => {
            return Ok(legacy_no_route(
                ingress,
                model_tiers,
                policy,
                &identity,
                reason,
            ));
        }
    };
    let vector = build_drex_vector(
        &ingress.intent,
        risk.as_str(),
        &ingress.raw_text,
        privacy.as_str(),
        &ingress.runtime,
    );
    let initial_runtime_hint = runtime_hint(ingress, &vector);
    let mut required_capabilities = required_model_capabilities(ingress);
    if ingress.intent == "code" && privacy == PrivacyClass::Sovereign {
        required_capabilities.push("advanced_coding");
    }
    let request_privacy = if is_local_runtime(&ingress.runtime) {
        PrivacyClass::LocalOnly
    } else {
        privacy.clone()
    };
    let minimum_quality_class = minimum_quality_class(ingress, privacy);
    let request = ModelCallRequest {
        thread_id: identity.thread_id.clone(),
        turn_id: identity.turn_id.clone(),
        work_id: None,
        call_id: identity.call_id.clone(),
        intent: ingress.intent.clone(),
        stage: ModelCallStage::LegacyRoute,
        raw_text: ingress.raw_text.clone(),
        privacy: request_privacy,
        risk,
        safety: SafetyClass::Unapproved,
        required_capabilities: required_capabilities
            .iter()
            .map(|capability| (*capability).to_string())
            .collect(),
        required_context_tokens: ingress.required_context_tokens,
        minimum_quality_class,
        minimum_success_rate: 0.0,
        maximum_marginal_cost_usd: None,
        preferred_provider: None,
        preferred_model: None,
        allowed_models: Vec::new(),
        excluded_models: Vec::new(),
    };
    let candidates: Vec<ModelCallCandidate> = model_tiers
        .iter()
        .cloned()
        .map(|mut tier| {
            if tier.id == 0 {
                tier.id = legacy_model_id(&tier);
            }
            let locality = legacy_locality(ingress, &tier);
            ModelCallCandidate {
                connected: tier.enabled,
                locality: locality.clone(),
                adapter_capable: locality != ExecutionLocality::OnDevice
                    || tier.vram_requirement_mb == 0
                    || tier.vram_requirement_mb <= ingress.available_vram_mb,
                quota_available: true,
                marginal_cost_usd: legacy_marginal_cost(ingress, &tier),
                cost_truth: cost_truth_for_tier(ingress, &tier),
                tier,
            }
        })
        .collect();
    let call_plan = plan_model_call(&request, &candidates, policy)?;
    let selected_model = call_plan.selected.map(|candidate| candidate.tier);

    let (execution_mode, runtime_hint, routing_metadata) = if let Some(ref tier) = selected_model {
        let execution_mode = execution_mode_for_tier(tier);
        let runtime_hint = if matches!(execution_mode, ExecutionMode::LocalModel) {
            "local".to_string()
        } else {
            initial_runtime_hint.clone()
        };
        (
            execution_mode,
            runtime_hint,
            json!({
                "reason": call_plan.selection_reason,
                "mode": execution_mode_label(execution_mode),
                "model_id": tier.model_id,
                "provider": tier.provider,
                "required_capabilities": required_capabilities,
                "cost_truth": call_plan.selected_cost_truth,
                "admitted_ids": call_plan.admitted_ids,
                "rejected": call_plan.rejected,
                "policy_version": call_plan.policy_version,
                "thread_id": call_plan.thread_id,
                "turn_id": call_plan.turn_id,
                "call_id": call_plan.call_id,
                "stage": call_plan.stage,
            })
            .to_string(),
        )
    } else {
        (
            ExecutionMode::Clarify,
            initial_runtime_hint.clone(),
            json!({
                "reason": call_plan.selection_reason,
                "mode": execution_mode_label(ExecutionMode::Clarify),
                "admitted_ids": call_plan.admitted_ids,
                "rejected": call_plan.rejected,
                "policy_version": call_plan.policy_version,
                "legacy_locality_policy": if is_local_runtime(&ingress.runtime) {
                    "local_runtime_locality_required"
                } else {
                    "none"
                },
                "thread_id": call_plan.thread_id,
                "turn_id": call_plan.turn_id,
                "call_id": call_plan.call_id,
                "stage": call_plan.stage,
            })
            .to_string(),
        )
    };

    Ok(RoutePlan {
        thread_id: call_plan.thread_id,
        turn_id: call_plan.turn_id,
        call_id: call_plan.call_id,
        stage: call_plan.stage,
        decision: call_plan.decision,
        execution_mode,
        runtime_hint,
        selected_model,
        routing_metadata,
    })
}

fn minimum_quality_class(ingress: &DrexIngress, privacy: PrivacyClass) -> u8 {
    match ingress.risk.as_str() {
        "critical" | "high" => 3,
        "medium" => 2,
        _ if ingress.intent == "code" && privacy != PrivacyClass::Sovereign => 3,
        _ => 1,
    }
}

fn legacy_locality(_ingress: &DrexIngress, tier: &ModelTier) -> ExecutionLocality {
    // An Ollama/local detected inventory is direct on-device proof. Generic
    // LiteLLM/vLLM provider names remain unverified proxy/endpoint labels.
    if matches!(tier.provider.as_str(), "ollama" | "local")
        && matches!(tier.rate_group.as_str(), "local" | "local_ollama")
    {
        ExecutionLocality::OnDevice
    } else {
        ExecutionLocality::Unverified
    }
}

fn legacy_marginal_cost(ingress: &DrexIngress, tier: &ModelTier) -> Option<f64> {
    if tier.cost_per_turn == 0.0 && legacy_locality(ingress, tier) != ExecutionLocality::OnDevice {
        None
    } else {
        Some(tier.cost_per_turn)
    }
}

fn cost_truth_for_tier(ingress: &DrexIngress, tier: &ModelTier) -> CostTruth {
    if tier.cost_per_turn == 0.0 && legacy_locality(ingress, tier) == ExecutionLocality::OnDevice {
        CostTruth::LocalZeroCost
    } else if tier.cost_per_turn == 0.0 {
        CostTruth::CannotConfirm
    } else {
        CostTruth::ProxyEstimate
    }
}

fn legacy_model_id(tier: &ModelTier) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in format!(
        "{}|{}|{}|{}",
        tier.provider, tier.model_id, tier.provider_model_id, tier.rate_group
    )
    .bytes()
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn legacy_no_route(
    ingress: &DrexIngress,
    model_tiers: &[ModelTier],
    policy: &DrexPolicy,
    identity: &ModelCallIdentity,
    reason: &str,
) -> RoutePlan {
    let vector = build_drex_vector(
        &ingress.intent,
        "low",
        &ingress.raw_text,
        "standard",
        &ingress.runtime,
    );
    let decision = evaluate_drex(&vector, policy, 0.80, 0.80, 0.65);
    let rejected: Vec<CandidateRejection> = model_tiers
        .iter()
        .map(|tier| CandidateRejection {
            candidate_id: tier.id,
            reasons: vec![CandidateRejectionReason::InvalidRequestPolicy],
        })
        .collect();
    RoutePlan {
        thread_id: identity.thread_id.clone(),
        turn_id: identity.turn_id.clone(),
        call_id: identity.call_id.clone(),
        stage: ModelCallStage::LegacyRoute,
        decision,
        execution_mode: ExecutionMode::Clarify,
        runtime_hint: ingress.runtime.clone(),
        selected_model: None,
        routing_metadata: json!({
            "reason": reason,
            "mode": execution_mode_label(ExecutionMode::Clarify),
            "rejected": rejected,
            "policy_version": super::policy::DEFAULT_POLICY_VERSION,
            "thread_id": identity.thread_id,
            "turn_id": identity.turn_id,
            "call_id": identity.call_id,
            "stage": ModelCallStage::LegacyRoute,
        })
        .to_string(),
    }
}

pub(crate) fn build_drex_vector(
    intent: &str,
    risk: &str,
    raw_text: &str,
    privacy: &str,
    runtime: &str,
) -> DrexVector {
    let mut vector = match intent {
        "build" | "files" => DrexVector {
            scope: 0.35,
            abstraction: 0.35,
            context_span: 0.55,
            execution_proximity: 0.90,
            blast_radius: 0.55,
            coordination_load: 0.30,
            latency_pressure: 0.75,
        },
        "audit" | "status_check" => DrexVector {
            scope: 0.45,
            abstraction: 0.40,
            context_span: 0.60,
            execution_proximity: 0.30,
            blast_radius: 0.45,
            coordination_load: 0.25,
            latency_pressure: 0.60,
        },
        "research" => DrexVector {
            scope: 0.70,
            abstraction: 0.80,
            context_span: 0.85,
            execution_proximity: 0.20,
            blast_radius: 0.40,
            coordination_load: 0.65,
            latency_pressure: 0.30,
        },
        "strategy" => DrexVector {
            scope: 0.90,
            abstraction: 0.95,
            context_span: 0.80,
            execution_proximity: 0.10,
            blast_radius: 0.75,
            coordination_load: 0.85,
            latency_pressure: 0.20,
        },
        "deploy" | "operate" | "automate" => DrexVector {
            scope: 0.75,
            abstraction: 0.60,
            context_span: 0.65,
            execution_proximity: 0.65,
            blast_radius: 0.90,
            coordination_load: 0.70,
            latency_pressure: 0.70,
        },
        _ => DrexVector {
            scope: 0.50,
            abstraction: 0.50,
            context_span: 0.50,
            execution_proximity: 0.50,
            blast_radius: 0.50,
            coordination_load: 0.50,
            latency_pressure: 0.50,
        },
    };

    if matches!(risk, "high" | "critical") {
        vector.scope = clamp(vector.scope + 0.15);
        vector.blast_radius = clamp(vector.blast_radius + 0.15);
    }

    if privacy == "sovereign" {
        vector.scope = clamp(vector.scope - 0.05);
        vector.execution_proximity = clamp(vector.execution_proximity + 0.10);
    }

    if matches!(runtime, "boost" | "macbook" | "local") {
        vector.execution_proximity = clamp(vector.execution_proximity + 0.10);
    }

    if raw_text.len() > 500 {
        vector.context_span = clamp(vector.context_span + 0.10);
    }

    let lowercase = raw_text.to_ascii_lowercase();
    if ["patch", "edit", "write", "run", "pytest", "bash", "shell"]
        .iter()
        .any(|needle| lowercase.contains(needle))
    {
        vector.execution_proximity = clamp(vector.execution_proximity + 0.10);
    }

    if [
        "portfolio",
        "enterprise",
        "roadmap",
        "priority",
        "governance",
    ]
    .iter()
    .any(|needle| lowercase.contains(needle))
    {
        vector.scope = clamp(vector.scope + 0.10);
        vector.abstraction = clamp(vector.abstraction + 0.10);
    }

    vector
}

fn runtime_hint(ingress: &DrexIngress, vector: &DrexVector) -> String {
    if ingress.privacy == "sovereign" && is_local_runtime(&ingress.runtime) {
        return ingress.runtime.clone();
    }

    if vector.execution_proximity >= 0.80 && is_local_runtime(&ingress.runtime) {
        return ingress.runtime.clone();
    }

    "cloud".to_string()
}

fn required_model_capabilities(ingress: &DrexIngress) -> Vec<&'static str> {
    let lowercase = ingress.raw_text.to_ascii_lowercase();
    let phrase_needles = [
        "mcp server",
        "mcp tool",
        "tool use",
        "use github",
        "github",
        "pull request",
        "open an issue",
        "create an issue",
        "notion",
        "slack",
        "gmail",
        "send email",
        "calendar",
        "google drive",
        "google sheet",
        "browser",
        "webhook",
        "jira",
        "linear",
    ];
    let word_needles = ["mcp"];
    let needs_external_tool = phrase_needles
        .iter()
        .any(|needle| lowercase.contains(needle))
        || word_needles
            .iter()
            .any(|needle| contains_word(&lowercase, needle));

    if needs_external_tool {
        vec!["tool_use"]
    } else {
        Vec::new()
    }
}

pub(crate) fn tier_has_strength(tier: &ModelTier, capability: &str) -> bool {
    serde_json::from_str::<Vec<String>>(&tier.strengths_json)
        .map(|strengths| strengths.iter().any(|strength| strength == capability))
        .unwrap_or(false)
}

fn contains_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|token| token == word)
}

pub(crate) fn is_local_provider(provider: &str) -> bool {
    matches!(provider, "ollama" | "local" | "vllm" | "litellm")
}

fn is_local_runtime(runtime: &str) -> bool {
    matches!(runtime, "macbook" | "boost" | "local")
}

fn should_prefer_local(
    ingress: &DrexIngress,
    decision: &DrexDecision,
    local_candidates: &[&ModelTier],
) -> bool {
    if local_candidates.is_empty() {
        return false;
    }

    if matches!(ingress.intent.as_str(), "chat" | "status_check") {
        return true;
    }

    if ingress.privacy == "sovereign" {
        return true;
    }

    if ingress.intent == "code" {
        let strongest_local = local_candidates
            .iter()
            .map(|tier| tier.capability_class)
            .max()
            .unwrap_or(0);
        return strongest_local >= 3
            && ingress.required_context_tokens <= 32_768
            && !matches!(ingress.risk.as_str(), "high" | "critical");
    }

    decision.active_tier == ResolutionTier::Micro
        && ingress.required_context_tokens <= 32_768
        && !matches!(ingress.risk.as_str(), "high" | "critical")
}

fn execution_mode_for_tier(tier: &ModelTier) -> ExecutionMode {
    if matches!(tier.provider.as_str(), "ollama" | "local")
        && matches!(tier.rate_group.as_str(), "local" | "local_ollama")
    {
        ExecutionMode::LocalModel
    } else {
        ExecutionMode::RemoteModel
    }
}

fn execution_mode_label(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Deterministic => "deterministic",
        ExecutionMode::LocalModel => "local_model",
        ExecutionMode::RemoteModel => "remote_model",
        ExecutionMode::Clarify => "clarify",
    }
}

fn clamp(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn is_greeting(lowercase: &str) -> bool {
    matches!(
        lowercase.trim(),
        "hi" | "hello"
            | "hey"
            | "yo"
            | "sup"
            | "gm"
            | "good morning"
            | "good afternoon"
            | "good evening"
    )
}

fn is_underspecified(lowercase: &str) -> bool {
    let trimmed = lowercase.trim();
    let token_count = trimmed.split_whitespace().count();
    token_count <= 2
        && matches!(
            trimmed,
            "help" | "what" | "why" | "huh" | "okay" | "ok" | "sure" | "go" | "start" | "continue"
        )
}
