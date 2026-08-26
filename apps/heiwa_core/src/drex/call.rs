use std::cmp::Ordering;
use std::collections::BTreeMap;

use anyhow::Result;
use heiwa_protocol::ModelTier;

use super::policy::{DrexDecision, DrexPolicy, DEFAULT_POLICY_VERSION};
use super::router::{build_drex_vector, tier_has_strength};
use super::scorer::evaluate_drex;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyClass {
    Standard,
    LocalOnly,
    Sovereign,
}

impl PrivacyClass {
    pub fn parse(value: &str) -> std::result::Result<Self, &'static str> {
        match value {
            "standard" => Ok(Self::Standard),
            "local_only" => Ok(Self::LocalOnly),
            "sovereign" => Ok(Self::Sovereign),
            _ => Err("invalid_privacy_class"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::LocalOnly => "local_only",
            Self::Sovereign => "sovereign",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLocality {
    OnDevice,
    SovereignEndpoint,
    Remote,
    Unverified,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallRisk {
    Low,
    Medium,
    High,
    Critical,
}

impl CallRisk {
    pub fn parse(value: &str) -> std::result::Result<Self, &'static str> {
        match value {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "critical" => Ok(Self::Critical),
            _ => Err("invalid_risk_class"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SafetyClass {
    Approved,
    Unapproved,
    Blocked,
}

impl SafetyClass {
    /// Explicit policy outcome for inference that has been classified as
    /// low-risk. Higher-risk work remains unapproved until its own approval
    /// flow records a decision.
    pub fn low_risk_auto_approval(risk: &CallRisk) -> Self {
        if risk == &CallRisk::Low {
            Self::Approved
        } else {
            Self::Unapproved
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelCallStage {
    Classification,
    Planning,
    Compression,
    Drafting,
    Execution,
    Review,
    LoopIteration,
    LegacyRoute,
}

impl ModelCallStage {
    pub fn parse(value: &str) -> std::result::Result<Self, &'static str> {
        match value {
            "classification" => Ok(Self::Classification),
            "planning" => Ok(Self::Planning),
            "compression" => Ok(Self::Compression),
            "drafting" => Ok(Self::Drafting),
            "execution" => Ok(Self::Execution),
            "review" => Ok(Self::Review),
            "loop_iteration" => Ok(Self::LoopIteration),
            "legacy_route" => Ok(Self::LegacyRoute),
            _ => Err("invalid_model_call_stage"),
        }
    }

    pub fn is_execution_bearing(&self) -> bool {
        matches!(
            self,
            Self::Compression | Self::Drafting | Self::Execution | Self::LoopIteration
        )
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ModelCallIdentity {
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
}

impl ModelCallIdentity {
    pub fn legacy_uuid() -> Self {
        let id = uuid::Uuid::new_v4();
        Self {
            thread_id: format!("thread-{id}"),
            turn_id: format!("turn-{id}"),
            call_id: format!("call-{id}"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CostTruth {
    LocalZeroCost,
    TargetOnly,
    ProxyEstimate,
    ExactProviderReport,
    CannotConfirm,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelCallRequest {
    pub thread_id: String,
    pub turn_id: String,
    /// Durable Work scope for operator-owned calls; absent for system and
    /// legacy calls that are intentionally not attached to a Work.
    pub work_id: Option<String>,
    pub call_id: String,
    pub intent: String,
    pub stage: ModelCallStage,
    pub raw_text: String,
    pub privacy: PrivacyClass,
    pub risk: CallRisk,
    pub safety: SafetyClass,
    pub required_capabilities: Vec<String>,
    pub required_context_tokens: u32,
    pub minimum_quality_class: u8,
    pub minimum_success_rate: f64,
    pub maximum_marginal_cost_usd: Option<f64>,
    pub preferred_provider: Option<String>,
    pub preferred_model: Option<String>,
    pub allowed_models: Vec<String>,
    pub excluded_models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelCallCandidate {
    pub tier: ModelTier,
    pub locality: ExecutionLocality,
    pub connected: bool,
    pub adapter_capable: bool,
    pub quota_available: bool,
    pub marginal_cost_usd: Option<f64>,
    pub cost_truth: CostTruth,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CandidateRejectionReason {
    DuplicateCandidateId,
    DisabledModel,
    InvalidMarginalCostUsd,
    InvalidCostTruth,
    InvalidSuccessRate,
    Disconnected,
    AdapterIncapable,
    QuotaExhausted,
    ExcludedModel,
    NotAllowedModel,
    PreferredProviderMismatch,
    PreferredModelMismatch,
    LocalOnlyOnDeviceRequired,
    SovereignLocalityRequired,
    InsufficientContext,
    MissingRequiredCapability,
    MinimumQualityClass,
    MinimumSuccessRate,
    MaximumMarginalCostUsd,
    UnknownMarginalCostUsd,
    SafetyForbidsExecution,
    AuthorityApprovalRequired,
    InvalidRequestPolicy,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CandidateRejection {
    pub candidate_id: u64,
    pub reasons: Vec<CandidateRejectionReason>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelCallPlan {
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
    pub stage: ModelCallStage,
    pub selected: Option<ModelCallCandidate>,
    pub selected_id: Option<u64>,
    pub selected_cost_truth: Option<CostTruth>,
    pub admitted_ids: Vec<u64>,
    pub rejected: Vec<CandidateRejection>,
    pub policy_version: String,
    pub selection_reason: String,
    pub decision: DrexDecision,
}

pub fn plan_model_call(
    request: &ModelCallRequest,
    candidates: &[ModelCallCandidate],
    policy: &DrexPolicy,
) -> Result<ModelCallPlan> {
    let decision = call_decision(request, policy);
    if !valid_request(request) {
        return Ok(no_selection_plan(
            request,
            decision,
            candidates,
            CandidateRejectionReason::InvalidRequestPolicy,
            "invalid_request_policy",
        ));
    }
    if request.safety == SafetyClass::Blocked {
        return Ok(no_selection_plan(
            request,
            decision,
            candidates,
            CandidateRejectionReason::SafetyForbidsExecution,
            "safety_forbids_execution",
        ));
    }
    if request.stage.is_execution_bearing()
        && decision.gate.requires_approval
        && request.safety != SafetyClass::Approved
    {
        return Ok(no_selection_plan(
            request,
            decision,
            candidates,
            CandidateRejectionReason::AuthorityApprovalRequired,
            "authority_approval_required",
        ));
    }

    let duplicate_ids = duplicate_ids(candidates);
    let mut admitted = Vec::new();
    let mut rejected = Vec::new();
    for candidate in candidates {
        if duplicate_ids.contains_key(&candidate.tier.id) {
            rejected.push(rejection(
                candidate.tier.id,
                CandidateRejectionReason::DuplicateCandidateId,
            ));
        } else if let Some(reason) = rejection_reason(request, candidate) {
            rejected.push(rejection(candidate.tier.id, reason));
        } else {
            admitted.push(candidate.clone());
        }
    }

    admitted.sort_by(compare_candidates);
    let admitted_ids = admitted.iter().map(|candidate| candidate.tier.id).collect();
    sort_rejections(&mut rejected);
    let selected = admitted.into_iter().next();
    let (selected_id, selected_cost_truth) = selected
        .as_ref()
        .map(|candidate| (Some(candidate.tier.id), Some(candidate.cost_truth.clone())))
        .unwrap_or((None, None));

    Ok(ModelCallPlan {
        thread_id: request.thread_id.clone(),
        turn_id: request.turn_id.clone(),
        call_id: request.call_id.clone(),
        stage: request.stage.clone(),
        selected,
        selected_id,
        selected_cost_truth,
        admitted_ids,
        rejected,
        policy_version: DEFAULT_POLICY_VERSION.to_string(),
        selection_reason: if selected_id.is_some() {
            "lowest_known_marginal_cost_then_quality_latency_success".to_string()
        } else {
            "no_admitted_model_call_candidates".to_string()
        },
        decision,
    })
}

pub fn compare_candidates(left: &ModelCallCandidate, right: &ModelCallCandidate) -> Ordering {
    match (left.marginal_cost_usd, right.marginal_cost_usd) {
        (Some(left_cost), Some(right_cost)) => {
            let ordering = left_cost.total_cmp(&right_cost);
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        (Some(_), None) => return Ordering::Less,
        (None, Some(_)) => return Ordering::Greater,
        (None, None) => {}
    }
    let quality = right.tier.capability_class.cmp(&left.tier.capability_class);
    if quality != Ordering::Equal {
        return quality;
    }
    let latency = left.tier.latency_p_95_ms.cmp(&right.tier.latency_p_95_ms);
    if latency != Ordering::Equal {
        return latency;
    }
    let success = right
        .tier
        .last_success_rate
        .total_cmp(&left.tier.last_success_rate);
    if success != Ordering::Equal {
        return success;
    }
    left.tier.id.cmp(&right.tier.id)
}

fn call_decision(request: &ModelCallRequest, policy: &DrexPolicy) -> DrexDecision {
    let vector = build_drex_vector(
        &request.intent,
        request.risk.as_str(),
        &request.raw_text,
        request.privacy.as_str(),
        if matches!(request.privacy, PrivacyClass::Standard) {
            "any"
        } else {
            "local"
        },
    );
    let runtime_fit = if request.stage.is_execution_bearing() {
        1.0
    } else {
        0.8
    };
    let observability = if request.safety == SafetyClass::Approved {
        0.95
    } else {
        0.80
    };
    evaluate_drex(&vector, policy, observability, runtime_fit, 0.65)
}

fn no_selection_plan(
    request: &ModelCallRequest,
    decision: DrexDecision,
    candidates: &[ModelCallCandidate],
    reason: CandidateRejectionReason,
    selection_reason: &str,
) -> ModelCallPlan {
    let mut rejected: Vec<_> = candidates
        .iter()
        .map(|candidate| rejection(candidate.tier.id, reason.clone()))
        .collect();
    sort_rejections(&mut rejected);
    ModelCallPlan {
        thread_id: request.thread_id.clone(),
        turn_id: request.turn_id.clone(),
        call_id: request.call_id.clone(),
        stage: request.stage.clone(),
        selected: None,
        selected_id: None,
        selected_cost_truth: None,
        admitted_ids: Vec::new(),
        rejected,
        policy_version: DEFAULT_POLICY_VERSION.to_string(),
        selection_reason: selection_reason.to_string(),
        decision,
    }
}

fn rejection(candidate_id: u64, reason: CandidateRejectionReason) -> CandidateRejection {
    CandidateRejection {
        candidate_id,
        reasons: vec![reason],
    }
}

fn sort_rejections(rejected: &mut [CandidateRejection]) {
    rejected.sort_by(|left, right| {
        left.candidate_id
            .cmp(&right.candidate_id)
            .then_with(|| left.reasons.cmp(&right.reasons))
    });
}

fn duplicate_ids(candidates: &[ModelCallCandidate]) -> BTreeMap<u64, usize> {
    let mut counts = BTreeMap::new();
    for candidate in candidates {
        *counts.entry(candidate.tier.id).or_insert(0) += 1;
    }
    counts.retain(|_, count| *count > 1);
    counts
}

fn valid_request(request: &ModelCallRequest) -> bool {
    !request.thread_id.trim().is_empty()
        && !request.turn_id.trim().is_empty()
        && !request.call_id.trim().is_empty()
        && valid_probability(request.minimum_success_rate)
        && request
            .maximum_marginal_cost_usd
            .is_none_or(|cost| cost.is_finite() && cost >= 0.0)
}

fn rejection_reason(
    request: &ModelCallRequest,
    candidate: &ModelCallCandidate,
) -> Option<CandidateRejectionReason> {
    if !candidate.tier.enabled {
        return Some(CandidateRejectionReason::DisabledModel);
    }
    if !valid_cost(candidate.marginal_cost_usd) {
        return Some(CandidateRejectionReason::InvalidMarginalCostUsd);
    }
    if !cost_truth_is_valid(candidate) {
        return Some(CandidateRejectionReason::InvalidCostTruth);
    }
    if !valid_probability(candidate.tier.last_success_rate) {
        return Some(CandidateRejectionReason::InvalidSuccessRate);
    }
    if !candidate.connected {
        return Some(CandidateRejectionReason::Disconnected);
    }
    if !candidate.adapter_capable {
        return Some(CandidateRejectionReason::AdapterIncapable);
    }
    if !candidate.quota_available {
        return Some(CandidateRejectionReason::QuotaExhausted);
    }
    if model_matches_any(candidate, &request.excluded_models) {
        return Some(CandidateRejectionReason::ExcludedModel);
    }
    if !request.allowed_models.is_empty() && !model_matches_any(candidate, &request.allowed_models)
    {
        return Some(CandidateRejectionReason::NotAllowedModel);
    }
    if request
        .preferred_provider
        .as_deref()
        .is_some_and(|provider| provider != candidate.tier.provider)
    {
        return Some(CandidateRejectionReason::PreferredProviderMismatch);
    }
    if request
        .preferred_model
        .as_deref()
        .is_some_and(|model| !model_matches(candidate, model))
    {
        return Some(CandidateRejectionReason::PreferredModelMismatch);
    }
    match request.privacy {
        PrivacyClass::Standard => {}
        PrivacyClass::LocalOnly if candidate.locality != ExecutionLocality::OnDevice => {
            return Some(CandidateRejectionReason::LocalOnlyOnDeviceRequired);
        }
        PrivacyClass::Sovereign
            if !matches!(
                candidate.locality,
                ExecutionLocality::OnDevice | ExecutionLocality::SovereignEndpoint
            ) =>
        {
            return Some(CandidateRejectionReason::SovereignLocalityRequired);
        }
        _ => {}
    }
    if candidate.tier.max_context_tokens < request.required_context_tokens {
        return Some(CandidateRejectionReason::InsufficientContext);
    }
    if !request.required_capabilities.iter().all(|capability| {
        tier_has_strength(&candidate.tier, capability)
            || (capability == "advanced_coding" && candidate.tier.capability_class >= 3)
    }) {
        return Some(CandidateRejectionReason::MissingRequiredCapability);
    }
    if candidate.tier.capability_class < request.minimum_quality_class {
        return Some(CandidateRejectionReason::MinimumQualityClass);
    }
    if candidate.tier.last_success_rate < request.minimum_success_rate {
        return Some(CandidateRejectionReason::MinimumSuccessRate);
    }
    if let Some(maximum) = request.maximum_marginal_cost_usd {
        match candidate.marginal_cost_usd {
            Some(cost) if cost > maximum => {
                return Some(CandidateRejectionReason::MaximumMarginalCostUsd);
            }
            None => return Some(CandidateRejectionReason::UnknownMarginalCostUsd),
            _ => {}
        }
    }
    None
}

fn cost_truth_is_valid(candidate: &ModelCallCandidate) -> bool {
    match candidate.cost_truth {
        CostTruth::LocalZeroCost => {
            candidate.locality == ExecutionLocality::OnDevice
                && candidate.marginal_cost_usd == Some(0.0)
        }
        CostTruth::TargetOnly | CostTruth::ProxyEstimate | CostTruth::ExactProviderReport => {
            candidate
                .marginal_cost_usd
                .is_some_and(|cost| cost.is_finite() && cost >= 0.0)
        }
        CostTruth::CannotConfirm => candidate.marginal_cost_usd.is_none(),
    }
}

fn model_matches_any(candidate: &ModelCallCandidate, models: &[String]) -> bool {
    models.iter().any(|model| model_matches(candidate, model))
}

fn model_matches(candidate: &ModelCallCandidate, model: &str) -> bool {
    model == candidate.tier.model_id
        || model == candidate.tier.provider_model_id
        || model == format!("{}/{}", candidate.tier.provider, candidate.tier.model_id)
}

fn valid_cost(cost: Option<f64>) -> bool {
    cost.is_none_or(|cost| cost.is_finite() && cost >= 0.0)
}

fn valid_probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

#[cfg(test)]
mod tests {
    use super::{CallRisk, SafetyClass};

    #[test]
    fn low_risk_auto_approval_fails_closed_for_higher_risk() {
        assert_eq!(
            SafetyClass::low_risk_auto_approval(&CallRisk::Low),
            SafetyClass::Approved
        );
        assert_eq!(
            SafetyClass::low_risk_auto_approval(&CallRisk::High),
            SafetyClass::Unapproved
        );
    }
}
