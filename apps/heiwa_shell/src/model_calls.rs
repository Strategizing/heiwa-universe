use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use heiwa_core::drex::{
    default_policy, plan_model_call, CostTruth, ModelCallCandidate, ModelCallPlan, ModelCallRequest,
};
use heiwa_evidence::{
    now_iso, CursorEvent, OperatorActor, OperatorEvent, OperatorEventType, OperatorRisk,
    OperatorSensitivity, OPERATOR_EVENT_SCHEMA_VERSION,
};
use heiwa_provider::adapter::{Message, ProviderAdapter, StreamEvent, TokenUsage};
use heiwa_session::operator::{OperatorSessionService, StartTurnRequest};
use serde_json::json;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

pub type AdapterResolver =
    dyn Fn(&str, &str) -> Option<Arc<dyn ProviderAdapter>> + Send + Sync + 'static;

pub struct ModelCallExecution {
    pub request: ModelCallRequest,
    pub candidates: Vec<ModelCallCandidate>,
    pub messages: Vec<Message>,
    pub remaining_budget_usd: Option<f64>,
    pub max_attempts: usize,
    pub cancel: watch::Receiver<bool>,
    /// Transient presentation-only deltas. Durable truth is still written
    /// exclusively through `OperatorSessionService`.
    pub delta_tx: Option<mpsc::Sender<StreamEvent>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ModelCallAttemptOutcome {
    Failed,
    Completed,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelCallAttemptRecord {
    pub candidate_id: u64,
    pub provider: String,
    pub model_id: String,
    pub outcome: ModelCallAttemptOutcome,
    pub failure_class: Option<ProviderFailureClass>,
    pub provider_invoked: bool,
    pub cost_usd: Option<f64>,
    pub cost_truth: CostTruth,
}

#[derive(Debug, Clone)]
pub struct ModelCallResult {
    /// Durable `route_completed` event id for this model-call stage.
    pub route_receipt_ref: String,
    pub provider: String,
    pub model_id: String,
    pub provider_model_id: String,
    pub rate_group: String,
    pub text: String,
    pub usage: TokenUsage,
    pub attempts: usize,
    pub failed_models: Vec<String>,
    pub cost_usd: f64,
    pub cost_truth: CostTruth,
    pub attempt_records: Vec<ModelCallAttemptRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ProviderFailureClass {
    Cancelled,
    RateLimited,
    Authentication,
    QuotaExhausted,
    Timeout,
    Availability,
    InvalidUsage,
    Provider,
}

impl ProviderFailureClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::RateLimited => "rate_limited",
            Self::Authentication => "authentication",
            Self::QuotaExhausted => "quota_exhausted",
            Self::Timeout => "timeout",
            Self::Availability => "availability",
            Self::InvalidUsage => "invalid_usage",
            Self::Provider => "provider",
        }
    }
}

#[derive(Debug)]
pub enum ModelCallError {
    Cancelled,
    InvalidBudget(String),
    MaxAttemptsZero,
    Planning(String),
    /// Boxed: `ModelCallPlan` is large, and an unboxed variant made the whole
    /// enum - and therefore every `Result<_, ModelCallError>` in this module -
    /// big enough to trip clippy::result_large_err on four functions.
    NoRoute(Box<ModelCallPlan>),
    EvidenceAppend {
        phase: &'static str,
        source: anyhow::Error,
    },
    AttemptsExhausted {
        attempts: usize,
        class: ProviderFailureClass,
        message: String,
    },
}

impl fmt::Display for ModelCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "model call cancelled"),
            Self::InvalidBudget(reason) => write!(f, "invalid model call budget: {reason}"),
            Self::MaxAttemptsZero => write!(f, "model call max_attempts must be at least one"),
            Self::Planning(error) => write!(f, "model call planning failed: {error}"),
            Self::NoRoute(plan) => write!(f, "no model call route: {}", plan.selection_reason),
            Self::EvidenceAppend { phase, source } => {
                write!(
                    f,
                    "model call evidence append failed during {phase}: {source}"
                )
            }
            Self::AttemptsExhausted {
                attempts,
                class,
                message,
            } => write!(
                f,
                "model call attempts exhausted after {attempts} attempt(s): {}: {message}",
                class.as_str()
            ),
        }
    }
}

impl std::error::Error for ModelCallError {}

struct FailedRouteAttempt<'a> {
    candidate: &'a ModelCallCandidate,
    attempt: usize,
    failure: &'a (ProviderFailureClass, String),
    cost_usd: Option<f64>,
    cost_truth: &'a CostTruth,
    remaining_budget_usd: Option<f64>,
}

pub struct ModelCallExecutor {
    resolver: Arc<AdapterResolver>,
    sessions: Arc<OperatorSessionService>,
}

impl ModelCallExecutor {
    pub fn new(resolver: Arc<AdapterResolver>, sessions: Arc<OperatorSessionService>) -> Self {
        Self { resolver, sessions }
    }

    pub async fn execute(
        &self,
        mut execution: ModelCallExecution,
    ) -> Result<ModelCallResult, ModelCallError> {
        if execution.max_attempts == 0 {
            return Err(ModelCallError::MaxAttemptsZero);
        }
        if execution
            .remaining_budget_usd
            .is_some_and(|budget| !budget.is_finite() || budget < 0.0)
        {
            return Err(ModelCallError::InvalidBudget(
                "remaining_budget_usd must be finite and non-negative".to_string(),
            ));
        }
        if *execution.cancel.borrow() {
            return Err(ModelCallError::Cancelled);
        }

        let max_attempts = execution.max_attempts.min(3);
        let mut attempts = 0usize;
        let mut remaining_budget = execution.remaining_budget_usd;
        let mut last_failure = None;
        let mut failed_models = Vec::new();
        let mut cumulative_cost = 0.0;
        let mut cumulative_truth = None;
        let mut attempt_records = Vec::new();

        while attempts < max_attempts {
            apply_remaining_budget(&mut execution.request, remaining_budget);
            let plan =
                plan_model_call(&execution.request, &execution.candidates, &default_policy())
                    .map_err(|error| ModelCallError::Planning(error.to_string()))?;

            let selected = plan.selected.clone();
            self.append_route_event(
                &execution.request,
                OperatorEventType::RoutePlanned,
                "route_planned",
                planned_payload(&execution.request, &plan, selected.as_ref(), attempts + 1),
            )?;

            let Some(candidate) = selected else {
                return Err(ModelCallError::NoRoute(Box::new(plan)));
            };

            attempts += 1;
            let provider = candidate.tier.provider.clone();
            let model_id = candidate.tier.model_id.clone();
            let provider_model_id = candidate.tier.provider_model_id.clone();
            self.append_route_event(
                &execution.request,
                OperatorEventType::RouteAttempted,
                "route_attempted",
                json!({
                    "attempt": attempts,
                    "provider": provider,
                    "model": model_id,
                    "provider_model": provider_model_id,
                }),
            )?;

            let Some(adapter) = (self.resolver)(&provider, &provider_model_id) else {
                let failure = (
                    ProviderFailureClass::Availability,
                    format!("provider resolver missing for {provider}/{provider_model_id}"),
                );
                let attempt_cost = None;
                let attempt_truth = CostTruth::CannotConfirm;
                self.append_failure(
                    &execution.request,
                    FailedRouteAttempt {
                        candidate: &candidate,
                        attempt: attempts,
                        failure: &failure,
                        cost_usd: attempt_cost,
                        cost_truth: &attempt_truth,
                        remaining_budget_usd: remaining_budget,
                    },
                )?;
                attempt_records.push(failed_attempt_record(
                    &candidate,
                    failure.0,
                    false,
                    attempt_cost,
                    attempt_truth,
                ));
                let failed_identity = qualified_model_identity(&candidate);
                execution
                    .request
                    .excluded_models
                    .push(failed_identity.clone());
                failed_models.push(failed_identity);
                last_failure = Some(failure);
                continue;
            };

            let started = Instant::now();
            match run_adapter(
                adapter,
                &provider_model_id,
                &execution.messages,
                &mut execution.cancel,
                execution.delta_tx.as_ref(),
            )
            .await
            {
                Ok((text, mut usage, emitted_delta)) => {
                    let charged_cost = completed_cost(&candidate, &usage).map_err(|message| {
                        ModelCallError::AttemptsExhausted {
                            attempts,
                            class: ProviderFailureClass::InvalidUsage,
                            message,
                        }
                    });
                    let charged_cost = match charged_cost {
                        Ok(cost) => cost,
                        Err(ModelCallError::AttemptsExhausted { class, message, .. }) => {
                            let failure = (class, message);
                            let (attempt_cost, attempt_truth) = attempted_cost(&candidate);
                            let next_budget =
                                subtract_optional_budget(remaining_budget, attempt_cost);
                            self.append_failure(
                                &execution.request,
                                FailedRouteAttempt {
                                    candidate: &candidate,
                                    attempt: attempts,
                                    failure: &failure,
                                    cost_usd: attempt_cost,
                                    cost_truth: &attempt_truth,
                                    remaining_budget_usd: next_budget,
                                },
                            )?;
                            add_cumulative_cost(
                                &mut cumulative_cost,
                                &mut cumulative_truth,
                                attempt_cost,
                                &attempt_truth,
                            );
                            remaining_budget = next_budget;
                            attempt_records.push(failed_attempt_record(
                                &candidate,
                                failure.0,
                                true,
                                attempt_cost,
                                attempt_truth,
                            ));
                            let failed_identity = qualified_model_identity(&candidate);
                            execution
                                .request
                                .excluded_models
                                .push(failed_identity.clone());
                            failed_models.push(failed_identity);
                            last_failure = Some(failure.clone());
                            if emitted_delta {
                                return Err(ModelCallError::AttemptsExhausted {
                                    attempts,
                                    class: failure.0,
                                    message: failure.1,
                                });
                            }
                            continue;
                        }
                        Err(other) => return Err(other),
                    };
                    remaining_budget = subtract_budget(remaining_budget, charged_cost.0);
                    usage.cost_usd = charged_cost.0;
                    add_cumulative_cost(
                        &mut cumulative_cost,
                        &mut cumulative_truth,
                        Some(charged_cost.0),
                        &charged_cost.1,
                    );
                    attempt_records.push(ModelCallAttemptRecord {
                        candidate_id: candidate.tier.id,
                        provider: provider.clone(),
                        model_id: model_id.clone(),
                        outcome: ModelCallAttemptOutcome::Completed,
                        failure_class: None,
                        provider_invoked: true,
                        cost_usd: Some(charged_cost.0),
                        cost_truth: charged_cost.1.clone(),
                    });
                    let route_receipt = self.append_route_event(
                        &execution.request,
                        OperatorEventType::RouteCompleted,
                        "route_completed",
                        json!({
                            "attempt": attempts,
                            "provider": provider,
                            "model": model_id,
                            "provider_model": provider_model_id,
                            "usage": usage,
                            "latency_ms": started.elapsed().as_millis(),
                            "cost_usd": charged_cost.0,
                            "cost_truth": charged_cost.1,
                            "cumulative_cost_usd": cumulative_cost,
                            "cumulative_cost_truth": cumulative_truth.clone(),
                            "remaining_budget_usd": remaining_budget,
                            "receipt_ref": serde_json::Value::Null,
                        }),
                    )?;
                    return Ok(ModelCallResult {
                        route_receipt_ref: route_receipt.event.event_id,
                        provider,
                        model_id,
                        provider_model_id,
                        rate_group: candidate.tier.rate_group,
                        text,
                        usage,
                        attempts,
                        failed_models,
                        cost_usd: cumulative_cost,
                        cost_truth: cumulative_truth.unwrap_or(CostTruth::CannotConfirm),
                        attempt_records,
                    });
                }
                Err(AdapterRunError::Cancelled) => {
                    let (attempt_cost, attempt_truth) = attempted_cost(&candidate);
                    self.append_cancelled_attempt(
                        &execution.request,
                        &candidate,
                        attempts,
                        attempt_cost,
                        &attempt_truth,
                    )?;
                    return Err(ModelCallError::Cancelled);
                }
                Err(AdapterRunError::Failed {
                    message,
                    emitted_delta,
                }) => {
                    let failure = (normalize_failure(&message), message);
                    let (attempt_cost, attempt_truth) = attempted_cost(&candidate);
                    let next_budget = subtract_optional_budget(remaining_budget, attempt_cost);
                    self.append_failure(
                        &execution.request,
                        FailedRouteAttempt {
                            candidate: &candidate,
                            attempt: attempts,
                            failure: &failure,
                            cost_usd: attempt_cost,
                            cost_truth: &attempt_truth,
                            remaining_budget_usd: next_budget,
                        },
                    )?;
                    add_cumulative_cost(
                        &mut cumulative_cost,
                        &mut cumulative_truth,
                        attempt_cost,
                        &attempt_truth,
                    );
                    remaining_budget = next_budget;
                    attempt_records.push(failed_attempt_record(
                        &candidate,
                        failure.0,
                        true,
                        attempt_cost,
                        attempt_truth,
                    ));
                    let failed_identity = qualified_model_identity(&candidate);
                    execution
                        .request
                        .excluded_models
                        .push(failed_identity.clone());
                    failed_models.push(failed_identity);
                    last_failure = Some(failure.clone());
                    if emitted_delta {
                        return Err(ModelCallError::AttemptsExhausted {
                            attempts,
                            class: failure.0,
                            message: failure.1,
                        });
                    }
                }
            }
        }

        let (class, message) = last_failure.unwrap_or((
            ProviderFailureClass::Provider,
            "no provider attempt completed".to_string(),
        ));
        Err(ModelCallError::AttemptsExhausted {
            attempts,
            class,
            message,
        })
    }

    /// Execute one already-admitted interactive/loop turn and close its
    /// durable lifecycle exactly once. The HTTP/Desktop runner owns the same
    /// lifecycle itself and therefore continues to call [`Self::execute`].
    pub async fn execute_canonical_turn(
        &self,
        execution: ModelCallExecution,
    ) -> Result<ModelCallResult, ModelCallError> {
        let request = execution.request.clone();
        self.append_turn_event(
            &request,
            OperatorEventType::AssistantStarted,
            "assistant_started",
            json!({}),
        )?;
        match self.execute(execution).await {
            Ok(result) => {
                self.append_turn_event(
                    &request,
                    OperatorEventType::AssistantCompleted,
                    "assistant_completed",
                    json!({
                        "text": result.text,
                        "usage": result.usage,
                        "cost_usd": result.cost_usd,
                        "cost_truth": result.cost_truth,
                    }),
                )?;
                self.append_turn_event(
                    &request,
                    OperatorEventType::TurnCompleted,
                    "turn_completed",
                    json!({"outcome": "completed"}),
                )?;
                Ok(result)
            }
            Err(error) => {
                let reason = if matches!(error, ModelCallError::Cancelled) {
                    "OPERATOR_CANCELLED"
                } else {
                    "EXECUTION_FAILED"
                };
                self.append_turn_event(
                    &request,
                    OperatorEventType::TurnInterrupted,
                    "turn_interrupted",
                    json!({
                        "reason": reason,
                        "message": error.to_string(),
                    }),
                )?;
                Err(error)
            }
        }
    }

    fn append_failure(
        &self,
        request: &ModelCallRequest,
        failed: FailedRouteAttempt<'_>,
    ) -> Result<CursorEvent, ModelCallError> {
        self.append_route_event(
            request,
            OperatorEventType::RouteFailed,
            "route_failed",
            json!({
                "attempt": failed.attempt,
                "provider": failed.candidate.tier.provider,
                "model": failed.candidate.tier.model_id,
                "provider_model": failed.candidate.tier.provider_model_id,
                "failure_class": failed.failure.0.as_str(),
                "message": failed.failure.1,
                "cost_usd": failed.cost_usd,
                "cost_truth": failed.cost_truth,
                "remaining_budget_usd": failed.remaining_budget_usd,
            }),
        )
    }

    fn append_cancelled_attempt(
        &self,
        request: &ModelCallRequest,
        candidate: &ModelCallCandidate,
        attempt: usize,
        cost_usd: Option<f64>,
        cost_truth: &CostTruth,
    ) -> Result<CursorEvent, ModelCallError> {
        self.append_route_event(
            request,
            OperatorEventType::RouteFailed,
            "route_failed",
            json!({
                "attempt": attempt,
                "provider": candidate.tier.provider,
                "model": candidate.tier.model_id,
                "provider_model": candidate.tier.provider_model_id,
                "failure_class": ProviderFailureClass::Cancelled.as_str(),
                "message": "provider attempt interrupted before completion truth",
                "outcome": "uncertain",
                "provider_invoked": true,
                "cost_usd": cost_usd,
                "cost_truth": cost_truth,
                "remaining_budget_usd": serde_json::Value::Null,
            }),
        )
    }

    fn append_route_event(
        &self,
        request: &ModelCallRequest,
        event_type: OperatorEventType,
        phase: &'static str,
        payload: serde_json::Value,
    ) -> Result<CursorEvent, ModelCallError> {
        let risk_class = match request.risk {
            heiwa_core::drex::CallRisk::Low => OperatorRisk::Low,
            heiwa_core::drex::CallRisk::Medium => OperatorRisk::Medium,
            heiwa_core::drex::CallRisk::High => OperatorRisk::High,
            heiwa_core::drex::CallRisk::Critical => OperatorRisk::Critical,
        };
        self.sessions
            .append_event(OperatorEvent {
                schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
                event_id: format!("evt-{}", Uuid::new_v4()),
                thread_id: request.thread_id.clone(),
                turn_id: Some(request.turn_id.clone()),
                run_id: None,
                call_id: Some(request.call_id.clone()),
                work_id: None,
                event_type,
                occurred_at: now_iso(),
                actor: OperatorActor {
                    kind: "runtime".to_string(),
                    id: "model-call-executor".to_string(),
                },
                risk_class,
                sensitivity: OperatorSensitivity::LocalPrivate,
                parent_event_id: None,
                correlation_id: Some(request.call_id.clone()),
                source_refs: vec![],
                evidence_refs: vec![],
                payload,
            })
            .map_err(|source| ModelCallError::EvidenceAppend { phase, source })
    }

    fn append_turn_event(
        &self,
        request: &ModelCallRequest,
        event_type: OperatorEventType,
        phase: &'static str,
        payload: serde_json::Value,
    ) -> Result<CursorEvent, ModelCallError> {
        let risk_class = match request.risk {
            heiwa_core::drex::CallRisk::Low => OperatorRisk::Low,
            heiwa_core::drex::CallRisk::Medium => OperatorRisk::Medium,
            heiwa_core::drex::CallRisk::High => OperatorRisk::High,
            heiwa_core::drex::CallRisk::Critical => OperatorRisk::Critical,
        };
        self.sessions
            .append_event(OperatorEvent {
                schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
                event_id: format!("evt-{}", Uuid::new_v4()),
                thread_id: request.thread_id.clone(),
                turn_id: Some(request.turn_id.clone()),
                run_id: None,
                call_id: None,
                work_id: None,
                event_type,
                occurred_at: now_iso(),
                actor: OperatorActor {
                    kind: "runtime".to_string(),
                    id: "model-call-turn-lifecycle".to_string(),
                },
                risk_class,
                sensitivity: OperatorSensitivity::LocalPrivate,
                parent_event_id: None,
                correlation_id: Some(request.call_id.clone()),
                source_refs: vec![],
                evidence_refs: vec![],
                payload,
            })
            .map_err(|source| ModelCallError::EvidenceAppend { phase, source })
    }
}

pub struct ExecutorLoopCaller {
    executor: Arc<ModelCallExecutor>,
}

impl ExecutorLoopCaller {
    pub fn new(executor: Arc<ModelCallExecutor>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl heiwa_loop::LoopModelCaller for ExecutorLoopCaller {
    async fn call(
        &self,
        request: heiwa_loop::LoopCallRequest,
    ) -> anyhow::Result<heiwa_loop::LoopCallResult> {
        let mut submission =
            StartTurnRequest::auto(request.turn_id.clone(), request.raw_text.clone());
        submission.route_policy.excluded_models = request.prior_failed_models.clone();
        submission.route_policy.turn_budget_usd = request.remaining_budget_usd;
        submission.route_policy.privacy = request.privacy.as_str().to_string();
        let turn = self
            .executor
            .sessions
            .start_turn(&request.thread_id, submission)?;
        if turn.duplicate {
            anyhow::bail!(
                "duplicate model call admission for existing turn {}; refusing provider execution",
                turn.turn_id
            );
        }
        let result = self
            .executor
            .execute_canonical_turn(ModelCallExecution {
                request: ModelCallRequest {
                    thread_id: request.thread_id,
                    turn_id: turn.turn_id,
                    call_id: request.call_id,
                    intent: request.intent,
                    stage: request.stage,
                    raw_text: request.raw_text,
                    privacy: request.privacy,
                    risk: request.risk,
                    safety: request.safety,
                    required_capabilities: vec![],
                    required_context_tokens: 1,
                    minimum_quality_class: 1,
                    minimum_success_rate: 0.0,
                    maximum_marginal_cost_usd: request.remaining_budget_usd,
                    preferred_provider: None,
                    preferred_model: None,
                    allowed_models: vec![],
                    excluded_models: request.prior_failed_models,
                },
                candidates: request.candidates,
                messages: request.messages,
                remaining_budget_usd: request.remaining_budget_usd,
                max_attempts: request.max_attempts,
                cancel: request.cancel,
                delta_tx: None,
            })
            .await?;
        Ok(heiwa_loop::LoopCallResult {
            provider: result.provider,
            model_id: result.model_id,
            text: result.text,
            usage: result.usage,
            attempts: result.attempts,
            failed_models: result.failed_models,
            cost_usd: result.cost_usd,
            cost_truth: result.cost_truth,
        })
    }
}

fn planned_payload(
    request: &ModelCallRequest,
    plan: &ModelCallPlan,
    selected: Option<&ModelCallCandidate>,
    attempt: usize,
) -> serde_json::Value {
    json!({
        "attempt": attempt,
        "intent": request.intent,
        "provider": selected.map(|candidate| candidate.tier.provider.as_str()),
        "model": selected.map(|candidate| candidate.tier.model_id.as_str()),
        "provider_model": selected.map(|candidate| candidate.tier.provider_model_id.as_str()),
        "rate_group": selected.map(|candidate| candidate.tier.rate_group.as_str()),
        "privacy": request.privacy.as_str(),
        "request_id": request.call_id,
        "stage": plan.stage,
        "selected_id": plan.selected_id,
        "cost_truth": plan.selected_cost_truth,
        "selection_reason": plan.selection_reason,
        "policy_version": plan.policy_version,
        "rejections": plan.rejected,
    })
}

fn apply_remaining_budget(request: &mut ModelCallRequest, remaining: Option<f64>) {
    if let Some(remaining) = remaining {
        request.maximum_marginal_cost_usd = Some(
            request
                .maximum_marginal_cost_usd
                .map(|current| current.min(remaining))
                .unwrap_or(remaining),
        );
    }
}

fn completed_cost(
    candidate: &ModelCallCandidate,
    usage: &TokenUsage,
) -> Result<(f64, CostTruth), String> {
    if !usage.cost_usd.is_finite() || usage.cost_usd < 0.0 {
        return Err("provider returned non-finite or negative cost_usd".to_string());
    }
    if usage.cost_usd > 0.0 {
        return Ok((usage.cost_usd, CostTruth::ExactProviderReport));
    }
    match (candidate.marginal_cost_usd, &candidate.cost_truth) {
        (Some(cost), truth) if cost.is_finite() && cost >= 0.0 => Ok((cost, truth.clone())),
        (None, CostTruth::CannotConfirm) => Ok((0.0, CostTruth::CannotConfirm)),
        _ => Err("candidate has invalid completion cost truth".to_string()),
    }
}

fn attempted_cost(candidate: &ModelCallCandidate) -> (Option<f64>, CostTruth) {
    match candidate.marginal_cost_usd {
        Some(cost) if cost.is_finite() && cost >= 0.0 => (Some(cost), candidate.cost_truth.clone()),
        _ => (None, CostTruth::CannotConfirm),
    }
}

fn qualified_model_identity(candidate: &ModelCallCandidate) -> String {
    format!("{}/{}", candidate.tier.provider, candidate.tier.model_id)
}

fn failed_attempt_record(
    candidate: &ModelCallCandidate,
    failure_class: ProviderFailureClass,
    provider_invoked: bool,
    cost_usd: Option<f64>,
    cost_truth: CostTruth,
) -> ModelCallAttemptRecord {
    ModelCallAttemptRecord {
        candidate_id: candidate.tier.id,
        provider: candidate.tier.provider.clone(),
        model_id: candidate.tier.model_id.clone(),
        outcome: ModelCallAttemptOutcome::Failed,
        failure_class: Some(failure_class),
        provider_invoked,
        cost_usd,
        cost_truth,
    }
}

fn add_cumulative_cost(
    total: &mut f64,
    aggregate_truth: &mut Option<CostTruth>,
    cost_usd: Option<f64>,
    cost_truth: &CostTruth,
) {
    if let Some(cost) = cost_usd {
        let next = *total + cost;
        if next.is_finite() {
            *total = next.max(0.0);
        } else {
            *total = f64::MAX;
            *aggregate_truth = Some(CostTruth::CannotConfirm);
            return;
        }
    }
    *aggregate_truth = Some(match aggregate_truth.take() {
        None => cost_truth.clone(),
        Some(current) => combine_cost_truth(current, cost_truth.clone()),
    });
}

fn combine_cost_truth(left: CostTruth, right: CostTruth) -> CostTruth {
    use CostTruth::*;
    match (left, right) {
        (CannotConfirm, _) | (_, CannotConfirm) => CannotConfirm,
        (LocalZeroCost, other) | (other, LocalZeroCost) => other,
        (ExactProviderReport, ExactProviderReport) => ExactProviderReport,
        (TargetOnly, TargetOnly) => TargetOnly,
        (ProxyEstimate, _) | (_, ProxyEstimate) => ProxyEstimate,
        (TargetOnly, ExactProviderReport) | (ExactProviderReport, TargetOnly) => ProxyEstimate,
    }
}

fn subtract_budget(remaining: Option<f64>, cost: f64) -> Option<f64> {
    remaining.map(|budget| (budget - cost).max(0.0))
}

fn subtract_optional_budget(remaining: Option<f64>, cost: Option<f64>) -> Option<f64> {
    match cost {
        Some(cost) => subtract_budget(remaining, cost),
        None => remaining,
    }
}

fn normalize_failure(message: &str) -> ProviderFailureClass {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("rate_limit")
        || normalized.contains("rate limit")
        || normalized.contains("too many requests")
        || normalized.contains("429")
    {
        ProviderFailureClass::RateLimited
    } else if normalized.contains("auth")
        || normalized.contains("unauthorized")
        || normalized.contains("forbidden")
        || normalized.contains("401")
        || normalized.contains("403")
    {
        ProviderFailureClass::Authentication
    } else if normalized.contains("quota") || normalized.contains("credit") {
        ProviderFailureClass::QuotaExhausted
    } else if normalized.contains("timeout") || normalized.contains("timed out") {
        ProviderFailureClass::Timeout
    } else if normalized.contains("unavailable")
        || normalized.contains("connection")
        || normalized.contains("not found")
    {
        ProviderFailureClass::Availability
    } else {
        ProviderFailureClass::Provider
    }
}

enum AdapterRunError {
    Cancelled,
    Failed {
        message: String,
        emitted_delta: bool,
    },
}

async fn run_adapter(
    adapter: Arc<dyn ProviderAdapter>,
    model: &str,
    messages: &[Message],
    cancel: &mut watch::Receiver<bool>,
    delta_tx: Option<&mpsc::Sender<StreamEvent>>,
) -> Result<(String, TokenUsage, bool), AdapterRunError> {
    if *cancel.borrow() {
        return Err(AdapterRunError::Cancelled);
    }

    let (stream_tx, mut stream_rx) = mpsc::channel(32);
    let adapter_task = adapter.clone();
    let model = model.to_string();
    let messages = messages.to_vec();
    let task = tokio::spawn(async move {
        let error_tx = stream_tx.clone();
        if let Err(error) = adapter_task.send(&model, &messages, stream_tx).await {
            let _ = error_tx
                .send(StreamEvent::Error(format!("adapter error: {error}")))
                .await;
        }
    });
    // Cancelling/dropping the executor future must not detach provider work.
    // Explicit watch cancellation still calls `interrupt`; this guard covers
    // outer races (for example operator preparation cancellation) that drop
    // this future before the watch branch can run.
    struct AbortProviderOnDrop(Option<tokio::task::JoinHandle<()>>);
    impl Drop for AbortProviderOnDrop {
        fn drop(&mut self) {
            if let Some(task) = self.0.take() {
                task.abort();
            }
        }
    }
    let mut task = AbortProviderOnDrop(Some(task));

    let mut text = String::new();
    let mut emitted_delta = false;
    let mut cancel_open = true;
    loop {
        tokio::select! {
            biased;
            event = stream_rx.recv() => {
                match event {
                    Some(StreamEvent::Token(token)) => {
                        emitted_delta = true;
                        text.push_str(&token);
                        if let Some(delta_tx) = delta_tx {
                            let _ = delta_tx.send(StreamEvent::Token(token)).await;
                        }
                    }
                    Some(StreamEvent::ToolUse { name, input }) => {
                        emitted_delta = true;
                        if let Some(delta_tx) = delta_tx {
                            let _ = delta_tx.send(StreamEvent::ToolUse {
                                name: name.clone(),
                                input: input.clone(),
                            }).await;
                        }
                        text.push_str(&json!({
                            "tool_calls": [{
                                "name": name,
                                "arguments": input,
                            }]
                        }).to_string());
                    }
                    Some(StreamEvent::Done(usage)) => {
                        if let Some(task) = task.0.take() {
                            task.abort();
                            let _ = task.await;
                        }
                        return Ok((text, usage, emitted_delta));
                    }
                    Some(StreamEvent::Error(error)) => {
                        if let Some(task) = task.0.take() {
                            task.abort();
                            let _ = task.await;
                        }
                        return Err(AdapterRunError::Failed {
                            message: error,
                            emitted_delta,
                        });
                    }
                    None => {
                        if let Some(task) = task.0.take() {
                            task.abort();
                            let _ = task.await;
                        }
                        return Err(AdapterRunError::Failed {
                            message: "provider stream ended without completion".to_string(),
                            emitted_delta,
                        });
                    }
                }
            }
            changed = cancel.changed(), if cancel_open => {
                match changed {
                    Ok(()) if *cancel.borrow() => {
                        if let Some(task) = task.0.take() {
                            task.abort();
                            let _ = task.await;
                        }
                        let _ = tokio::time::timeout(Duration::from_millis(250), adapter.interrupt()).await;
                        return Err(AdapterRunError::Cancelled);
                    }
                    Ok(()) => {}
                    Err(_) => cancel_open = false,
                }
            }
        }
    }
}
