mod model_call {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;
    use heiwa_core::drex::{
        CallRisk, CostTruth, ExecutionLocality, ModelCallCandidate, ModelCallRequest,
        ModelCallStage, PrivacyClass, SafetyClass,
    };
    use heiwa_evidence::{OperatorEventType, OperatorJournal};
    use heiwa_loop::{LoopCallRequest, LoopConfig, LoopController, LoopModelCaller};
    use heiwa_protocol::ModelTier;
    use heiwa_provider::adapter::{Message, ProviderAdapter, Role, StreamEvent, TokenUsage};
    use heiwa_session::operator::{OperatorSessionService, StartTurnRequest};
    use heiwa_shell::model_calls::{
        ExecutorLoopCaller, ModelCallAttemptOutcome, ModelCallError, ModelCallExecution,
        ModelCallExecutor, ProviderFailureClass,
    };
    use tokio::sync::{mpsc, watch};

    struct EventAdapter {
        events: Vec<StreamEvent>,
    }

    struct CountingErrorAdapter {
        sends: Arc<AtomicUsize>,
        error: String,
        sabotage_stream: Option<PathBuf>,
    }

    struct CountingDoneAdapter {
        sends: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ProviderAdapter for CountingDoneAdapter {
        async fn send(
            &self,
            _model: &str,
            _messages: &[Message],
            stream_tx: mpsc::Sender<StreamEvent>,
        ) -> Result<()> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            stream_tx
                .send(StreamEvent::Done(TokenUsage::default()))
                .await?;
            Ok(())
        }

        async fn interrupt(&self) -> Result<()> {
            Ok(())
        }

        fn supported_models(&self) -> Vec<String> {
            vec![]
        }
    }

    #[async_trait]
    impl ProviderAdapter for CountingErrorAdapter {
        async fn send(
            &self,
            _model: &str,
            _messages: &[Message],
            stream_tx: mpsc::Sender<StreamEvent>,
        ) -> Result<()> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            if let Some(path) = &self.sabotage_stream {
                let backup = path.with_extension("jsonl.backup");
                std::fs::rename(path, backup)?;
                std::fs::create_dir(path)?;
            }
            stream_tx
                .send(StreamEvent::Error(self.error.clone()))
                .await?;
            Ok(())
        }

        async fn interrupt(&self) -> Result<()> {
            Ok(())
        }

        fn supported_models(&self) -> Vec<String> {
            vec![]
        }
    }

    struct BlockingAdapter {
        started: Arc<tokio::sync::Notify>,
        interrupted: Arc<AtomicBool>,
        dropped: Arc<AtomicBool>,
    }

    struct SendDropGuard(Arc<AtomicBool>);

    impl Drop for SendDropGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    struct TokenThenWaitAdapter {
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl ProviderAdapter for TokenThenWaitAdapter {
        async fn send(
            &self,
            _model: &str,
            _messages: &[Message],
            stream_tx: mpsc::Sender<StreamEvent>,
        ) -> Result<()> {
            stream_tx
                .send(StreamEvent::Token("first".to_string()))
                .await?;
            self.release.notified().await;
            stream_tx
                .send(StreamEvent::Done(TokenUsage::default()))
                .await?;
            Ok(())
        }

        async fn interrupt(&self) -> Result<()> {
            Ok(())
        }

        fn supported_models(&self) -> Vec<String> {
            vec![]
        }
    }

    struct DoneAndCancelAdapter {
        cancel: watch::Sender<bool>,
    }

    #[async_trait]
    impl ProviderAdapter for DoneAndCancelAdapter {
        async fn send(
            &self,
            _model: &str,
            _messages: &[Message],
            stream_tx: mpsc::Sender<StreamEvent>,
        ) -> Result<()> {
            stream_tx
                .send(StreamEvent::Done(TokenUsage::default()))
                .await?;
            self.cancel.send(true)?;
            Ok(())
        }

        async fn interrupt(&self) -> Result<()> {
            Ok(())
        }

        fn supported_models(&self) -> Vec<String> {
            vec![]
        }
    }

    #[async_trait]
    impl ProviderAdapter for BlockingAdapter {
        async fn send(
            &self,
            _model: &str,
            _messages: &[Message],
            _stream_tx: mpsc::Sender<StreamEvent>,
        ) -> Result<()> {
            let _drop_guard = SendDropGuard(self.dropped.clone());
            self.started.notify_one();
            std::future::pending::<()>().await;
            Ok(())
        }

        async fn interrupt(&self) -> Result<()> {
            self.interrupted.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn supported_models(&self) -> Vec<String> {
            vec![]
        }
    }

    #[async_trait]
    impl ProviderAdapter for EventAdapter {
        async fn send(
            &self,
            _model: &str,
            _messages: &[Message],
            stream_tx: mpsc::Sender<StreamEvent>,
        ) -> Result<()> {
            for event in &self.events {
                stream_tx.send(event.clone()).await?;
            }
            Ok(())
        }

        async fn interrupt(&self) -> Result<()> {
            Ok(())
        }

        fn supported_models(&self) -> Vec<String> {
            vec![]
        }
    }

    fn candidate(id: u64, provider: &str, model: &str, cost: f64) -> ModelCallCandidate {
        ModelCallCandidate {
            tier: ModelTier {
                id,
                model_id: model.to_string(),
                provider_model_id: model.to_string(),
                provider: provider.to_string(),
                rate_group: provider.to_string(),
                capability_class: 3,
                effort_knob: "default".to_string(),
                effort_level: 1,
                cost_per_turn: cost,
                max_context_tokens: 8_192,
                strengths_json: "[\"advanced_coding\"]".to_string(),
                vram_requirement_mb: 0,
                quantization_type: "none".to_string(),
                kv_cache_strategy: "none".to_string(),
                enabled: true,
                last_success_rate: 1.0,
                avg_latency_ms: 1,
                latency_p_95_ms: 1,
                updated_at: "".to_string(),
            },
            locality: ExecutionLocality::Remote,
            connected: true,
            adapter_capable: true,
            quota_available: true,
            marginal_cost_usd: Some(cost),
            cost_truth: CostTruth::TargetOnly,
        }
    }

    fn request(thread_id: &str, turn_id: &str) -> ModelCallRequest {
        ModelCallRequest {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            work_id: None,
            call_id: "call-1".to_string(),
            intent: "code".to_string(),
            stage: ModelCallStage::Execution,
            raw_text: "do the work".to_string(),
            privacy: PrivacyClass::Standard,
            risk: CallRisk::Low,
            safety: SafetyClass::Approved,
            required_capabilities: vec![],
            required_context_tokens: 1,
            minimum_quality_class: 1,
            minimum_success_rate: 0.0,
            maximum_marginal_cost_usd: Some(1.0),
            preferred_provider: None,
            preferred_model: None,
            allowed_models: vec![],
            excluded_models: vec![],
        }
    }

    fn service_and_turn() -> (
        tempfile::TempDir,
        Arc<OperatorSessionService>,
        heiwa_session::operator::TurnSubmission,
    ) {
        let evidence = tempfile::tempdir().unwrap();
        let service = Arc::new(OperatorSessionService::new(
            OperatorJournal::new(evidence.path().to_path_buf()).unwrap(),
        ));
        let submission = service
            .start_turn(
                "thread-1",
                StartTurnRequest::auto("request-1", "do the work"),
            )
            .unwrap();
        (evidence, service, submission)
    }

    fn execution(
        request: ModelCallRequest,
        candidates: Vec<ModelCallCandidate>,
        max_attempts: usize,
        cancel: watch::Receiver<bool>,
    ) -> ModelCallExecution {
        ModelCallExecution {
            request,
            candidates,
            messages: vec![Message {
                role: Role::User,
                content: "do the work".to_string(),
            }],
            remaining_budget_usd: Some(1.0),
            max_attempts,
            cancel,
            delta_tx: None,
        }
    }

    #[tokio::test]
    async fn failed_primary_is_evidenced_before_secondary_completion() {
        let evidence = tempfile::tempdir().unwrap();
        let service = Arc::new(OperatorSessionService::new(
            OperatorJournal::new(evidence.path().to_path_buf()).unwrap(),
        ));
        let submission = service
            .start_turn(
                "thread-1",
                StartTurnRequest::auto("request-1", "do the work"),
            )
            .unwrap();

        let adapters: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::from([
            (
                "primary".to_string(),
                Arc::new(EventAdapter {
                    events: vec![StreamEvent::Error("rate_limited".to_string())],
                }) as Arc<dyn ProviderAdapter>,
            ),
            (
                "secondary".to_string(),
                Arc::new(EventAdapter {
                    events: vec![
                        StreamEvent::Token("done".to_string()),
                        StreamEvent::Done(TokenUsage {
                            input_tokens: 5,
                            output_tokens: 1,
                            cost_usd: 0.02,
                            ..TokenUsage::default()
                        }),
                    ],
                }) as Arc<dyn ProviderAdapter>,
            ),
        ]);
        let resolver =
            Arc::new(move |provider: &str, _model: &str| adapters.get(provider).cloned());
        let executor = ModelCallExecutor::new(resolver, service.clone());
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let mut call_request = request("thread-1", &submission.turn_id);
        call_request.minimum_quality_class = 3;
        let mut below_quality_floor = candidate(0, "cheap-low-quality", "cheap-low-quality", 0.0);
        below_quality_floor.tier.capability_class = 2;

        let result = executor
            .execute(ModelCallExecution {
                request: call_request,
                candidates: vec![
                    below_quality_floor,
                    candidate(1, "secondary", "secondary-model", 0.02),
                    candidate(2, "primary", "primary-model", 0.01),
                ],
                messages: vec![Message {
                    role: Role::User,
                    content: "do the work".to_string(),
                }],
                remaining_budget_usd: Some(1.0),
                max_attempts: 3,
                cancel: cancel_rx,
                delta_tx: None,
            })
            .await
            .unwrap();

        assert_eq!(result.provider, "secondary");
        assert_eq!(result.model_id, "secondary-model");
        assert_eq!(result.text, "done");
        assert_eq!(result.attempts, 2);
        assert_eq!(result.failed_models, vec!["primary/primary-model"]);
        assert_eq!(result.cost_usd, 0.03);
        assert_eq!(result.cost_truth, CostTruth::ProxyEstimate);
        assert_eq!(result.attempt_records.len(), 2);
        assert_eq!(
            result.attempt_records[0].outcome,
            ModelCallAttemptOutcome::Failed
        );
        assert_eq!(result.attempt_records[0].cost_usd, Some(0.01));
        assert_eq!(result.attempt_records[0].cost_truth, CostTruth::TargetOnly);
        assert_eq!(
            result.attempt_records[1].outcome,
            ModelCallAttemptOutcome::Completed
        );
        assert_eq!(result.attempt_records[1].cost_usd, Some(0.02));
        assert_eq!(
            result.attempt_records[1].cost_truth,
            CostTruth::ExactProviderReport
        );

        let events = service
            .events_after("thread-1", Some(&submission.cursor), 20)
            .unwrap()
            .events;
        let kinds = events
            .iter()
            .map(|row| row.event.event_type.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                OperatorEventType::RoutePlanned,
                OperatorEventType::RouteAttempted,
                OperatorEventType::RouteFailed,
                OperatorEventType::RoutePlanned,
                OperatorEventType::RouteAttempted,
                OperatorEventType::RouteCompleted,
            ]
        );
        assert_eq!(events[0].event.payload["provider"], "primary");
        assert_eq!(events[0].event.payload["selected_id"], 2);
        assert_eq!(events[0].event.payload["rejections"][0]["candidate_id"], 0);
        assert_eq!(
            events[0].event.payload["rejections"][0]["reasons"][0],
            "minimum_quality_class"
        );
        assert_eq!(events[2].event.payload["failure_class"], "rate_limited");
        assert_eq!(events[2].event.payload["cost_usd"], 0.01);
        assert_eq!(events[2].event.payload["cost_truth"], "target_only");
        assert_eq!(events[2].event.payload["remaining_budget_usd"], 0.99);
        assert_eq!(events[3].event.payload["provider"], "secondary");
        assert_eq!(events[3].event.payload["selected_id"], 1);
        assert_eq!(events[3].event.payload["rejections"][0]["candidate_id"], 0);
        assert_eq!(
            events[3].event.payload["rejections"][0]["reasons"][0],
            "minimum_quality_class"
        );
        assert_eq!(events[5].event.payload["cost_usd"], 0.02);
        assert_eq!(
            events[5].event.payload["cost_truth"],
            "exact_provider_report"
        );
        assert_eq!(events[5].event.payload["remaining_budget_usd"], 0.97);
        assert_eq!(events[5].event.payload["cumulative_cost_usd"], 0.03);
        assert_eq!(
            events[5].event.payload["cumulative_cost_truth"],
            "proxy_estimate"
        );
    }

    #[tokio::test]
    async fn compression_and_drafting_each_emit_the_complete_local_route_sequence() {
        for stage in [ModelCallStage::Compression, ModelCallStage::Drafting] {
            let (_evidence, service, submission) = service_and_turn();
            let adapter = Arc::new(EventAdapter {
                events: vec![
                    StreamEvent::Token("local output".to_string()),
                    StreamEvent::Done(TokenUsage::default()),
                ],
            }) as Arc<dyn ProviderAdapter>;
            let executor = ModelCallExecutor::new(
                Arc::new(move |_, _| Some(adapter.clone())),
                service.clone(),
            );
            let (_cancel_tx, cancel_rx) = watch::channel(false);
            let mut call = request("thread-1", &submission.turn_id);
            call.call_id = format!("call-{stage:?}");
            call.stage = stage.clone();
            call.privacy = PrivacyClass::Sovereign;
            call.maximum_marginal_cost_usd = Some(0.0);
            let mut local = candidate(1, "ollama", "local-model", 0.0);
            local.locality = ExecutionLocality::OnDevice;
            local.tier.rate_group = "local_ollama".to_string();
            local.cost_truth = CostTruth::LocalZeroCost;

            let result = executor
                .execute(execution(call, vec![local], 1, cancel_rx))
                .await
                .unwrap();
            assert_eq!(result.text, "local output");
            assert_eq!(result.cost_truth, CostTruth::LocalZeroCost);

            let events = service
                .events_after("thread-1", Some(&submission.cursor), 10)
                .unwrap()
                .events;
            assert_eq!(
                events
                    .iter()
                    .map(|row| row.event.event_type.clone())
                    .collect::<Vec<_>>(),
                vec![
                    OperatorEventType::RoutePlanned,
                    OperatorEventType::RouteAttempted,
                    OperatorEventType::RouteCompleted,
                ]
            );
            assert_eq!(
                events[0].event.payload["stage"],
                serde_json::to_value(stage).unwrap()
            );
            assert_eq!(events[0].event.payload["privacy"], "sovereign");
            assert_eq!(events[2].event.payload["cost_truth"], "local_zero_cost");
        }
    }

    #[tokio::test]
    async fn evidence_failure_before_attempt_prevents_provider_send() {
        let (_evidence, service, submission) = service_and_turn();
        let sends = Arc::new(AtomicUsize::new(0));
        let adapter = Arc::new(CountingErrorAdapter {
            sends: sends.clone(),
            error: "unavailable".to_string(),
            sabotage_stream: None,
        }) as Arc<dyn ProviderAdapter>;
        let executor =
            ModelCallExecutor::new(Arc::new(move |_, _| Some(adapter.clone())), service.clone());
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let mut invalid = request("thread-1", &submission.turn_id);
        invalid.turn_id = "missing-turn".to_string();

        let error = executor
            .execute(execution(
                invalid,
                vec![candidate(1, "primary", "model", 0.01)],
                1,
                cancel_rx,
            ))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            ModelCallError::EvidenceAppend {
                phase: "route_planned",
                ..
            }
        ));
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn evidence_failure_after_provider_failure_prevents_fallback() {
        let (evidence, service, submission) = service_and_turn();
        let primary_sends = Arc::new(AtomicUsize::new(0));
        let secondary_sends = Arc::new(AtomicUsize::new(0));
        let stream_path = evidence.path().join("operator_events.jsonl");
        let primary = Arc::new(CountingErrorAdapter {
            sends: primary_sends.clone(),
            error: "rate_limited".to_string(),
            sabotage_stream: Some(stream_path),
        }) as Arc<dyn ProviderAdapter>;
        let secondary = Arc::new(CountingErrorAdapter {
            sends: secondary_sends.clone(),
            error: "unavailable".to_string(),
            sabotage_stream: None,
        }) as Arc<dyn ProviderAdapter>;
        let executor = ModelCallExecutor::new(
            Arc::new(move |provider, _| match provider {
                "primary" => Some(primary.clone()),
                "secondary" => Some(secondary.clone()),
                _ => None,
            }),
            service,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let error = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![
                    candidate(1, "primary", "primary-model", 0.01),
                    candidate(2, "secondary", "secondary-model", 0.02),
                ],
                3,
                cancel_rx,
            ))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            ModelCallError::EvidenceAppend {
                phase: "route_failed",
                ..
            }
        ));
        assert_eq!(primary_sends.load(Ordering::SeqCst), 1);
        assert_eq!(secondary_sends.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn selected_none_returns_structured_no_route_error() {
        let (_evidence, service, submission) = service_and_turn();
        let executor = ModelCallExecutor::new(Arc::new(|_, _| None), service.clone());
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let error = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![],
                3,
                cancel_rx,
            ))
            .await
            .unwrap_err();

        let ModelCallError::NoRoute(plan) = error else {
            panic!("expected NoRoute");
        };
        assert!(plan.selected.is_none());
        let events = service
            .events_after("thread-1", Some(&submission.cursor), 10)
            .unwrap()
            .events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.event_type, OperatorEventType::RoutePlanned);
    }

    #[tokio::test]
    async fn max_attempts_zero_sends_nothing_and_values_above_three_are_capped() {
        let (_evidence, service, submission) = service_and_turn();
        let sends = Arc::new(AtomicUsize::new(0));
        let adapter = Arc::new(CountingErrorAdapter {
            sends: sends.clone(),
            error: "provider failed".to_string(),
            sabotage_stream: None,
        }) as Arc<dyn ProviderAdapter>;
        let executor =
            ModelCallExecutor::new(Arc::new(move |_, _| Some(adapter.clone())), service.clone());
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let zero = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![candidate(1, "p1", "m1", 0.01)],
                0,
                cancel_rx,
            ))
            .await
            .unwrap_err();
        assert!(matches!(zero, ModelCallError::MaxAttemptsZero));
        assert_eq!(sends.load(Ordering::SeqCst), 0);

        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let error = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![
                    candidate(1, "p1", "m1", 0.01),
                    candidate(2, "p2", "m2", 0.02),
                    candidate(3, "p3", "m3", 0.03),
                    candidate(4, "p4", "m4", 0.04),
                ],
                99,
                cancel_rx,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ModelCallError::AttemptsExhausted { attempts: 3, .. }
        ));
        assert_eq!(sends.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn cancellation_before_invocation_is_free_and_inflight_attempt_is_durable_uncertain() {
        let (_evidence, service, submission) = service_and_turn();
        let started = Arc::new(tokio::sync::Notify::new());
        let interrupted = Arc::new(AtomicBool::new(false));
        let adapter = Arc::new(BlockingAdapter {
            started: started.clone(),
            interrupted: interrupted.clone(),
            dropped: Arc::new(AtomicBool::new(false)),
        }) as Arc<dyn ProviderAdapter>;
        let executor = Arc::new(ModelCallExecutor::new(
            Arc::new(move |_, _| Some(adapter.clone())),
            service.clone(),
        ));

        let (_cancel_tx, cancel_rx) = watch::channel(true);
        let error = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![candidate(1, "primary", "model", 0.01)],
                1,
                cancel_rx,
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, ModelCallError::Cancelled));

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let executor_task = {
            let executor = executor.clone();
            let turn_id = submission.turn_id.clone();
            tokio::spawn(async move {
                executor
                    .execute(execution(
                        request("thread-1", &turn_id),
                        vec![candidate(1, "primary", "model", 0.01)],
                        1,
                        cancel_rx,
                    ))
                    .await
            })
        };
        started.notified().await;
        cancel_tx.send(true).unwrap();
        assert!(matches!(
            executor_task.await.unwrap().unwrap_err(),
            ModelCallError::Cancelled
        ));
        assert!(interrupted.load(Ordering::SeqCst));

        let events = service
            .events_after("thread-1", Some(&submission.cursor), 20)
            .unwrap()
            .events;
        assert!(!events
            .iter()
            .any(|row| row.event.event_type == OperatorEventType::RouteCompleted));
        let cancelled = events
            .iter()
            .find(|row| {
                row.event.event_type == OperatorEventType::RouteFailed
                    && row.event.payload["failure_class"] == "cancelled"
            })
            .expect("invoked cancellation must append one durable route outcome");
        assert_eq!(cancelled.event.payload["outcome"], "uncertain");
        assert_eq!(cancelled.event.payload["provider_invoked"], true);
        assert_eq!(cancelled.event.payload["cost_usd"], 0.01);
        assert_eq!(cancelled.event.payload["cost_truth"], "target_only");
    }

    #[tokio::test]
    async fn dropping_inflight_compression_aborts_the_provider_task() {
        let (_evidence, service, submission) = service_and_turn();
        let started = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let adapter = Arc::new(BlockingAdapter {
            started: started.clone(),
            interrupted: Arc::new(AtomicBool::new(false)),
            dropped: dropped.clone(),
        }) as Arc<dyn ProviderAdapter>;
        let executor = Arc::new(ModelCallExecutor::new(
            Arc::new(move |_, _| Some(adapter.clone())),
            service,
        ));
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let mut compression = request("thread-1", &submission.turn_id);
        compression.stage = ModelCallStage::Compression;
        compression.privacy = PrivacyClass::Sovereign;
        compression.maximum_marginal_cost_usd = Some(0.0);
        let mut local = candidate(1, "ollama", "local", 0.0);
        local.locality = ExecutionLocality::OnDevice;
        local.tier.rate_group = "local_ollama".to_string();
        local.cost_truth = CostTruth::LocalZeroCost;
        let task = tokio::spawn(async move {
            executor
                .execute(execution(compression, vec![local], 1, cancel_rx))
                .await
        });

        started.notified().await;
        task.abort();
        let _ = task.await;
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dropping compression must abort provider work");
    }

    #[tokio::test]
    async fn missing_resolver_is_normalized_as_availability_without_send() {
        let (_evidence, service, submission) = service_and_turn();
        let executor = ModelCallExecutor::new(Arc::new(|_, _| None), service.clone());
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let error = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![candidate(1, "missing", "model", 0.01)],
                1,
                cancel_rx,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ModelCallError::AttemptsExhausted {
                attempts: 1,
                class: ProviderFailureClass::Availability,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn rate_auth_and_quota_errors_have_stable_normalized_classes() {
        let (_evidence, service, submission) = service_and_turn();
        let errors = HashMap::from([
            ("rate", "429 too many requests"),
            ("auth", "401 unauthorized"),
            ("quota", "quota exhausted"),
        ]);
        let resolver = Arc::new(move |provider: &str, _model: &str| {
            errors.get(provider).map(|message| {
                Arc::new(EventAdapter {
                    events: vec![StreamEvent::Error((*message).to_string())],
                }) as Arc<dyn ProviderAdapter>
            })
        });
        let executor = ModelCallExecutor::new(resolver, service.clone());
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let _ = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![
                    candidate(1, "rate", "m1", 0.01),
                    candidate(2, "auth", "m2", 0.02),
                    candidate(3, "quota", "m3", 0.03),
                ],
                3,
                cancel_rx,
            ))
            .await;

        let classes = service
            .events_after("thread-1", Some(&submission.cursor), 30)
            .unwrap()
            .events
            .into_iter()
            .filter(|row| row.event.event_type == OperatorEventType::RouteFailed)
            .map(|row| {
                row.event.payload["failure_class"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            classes,
            vec!["rate_limited", "authentication", "quota_exhausted"]
        );
    }

    #[tokio::test]
    async fn cost_over_budget_clamps_remaining_and_nonfinite_values_fail_safely() {
        let (_evidence, service, submission) = service_and_turn();
        let success = Arc::new(EventAdapter {
            events: vec![StreamEvent::Done(TokenUsage {
                cost_usd: 2.0,
                ..TokenUsage::default()
            })],
        }) as Arc<dyn ProviderAdapter>;
        let executor =
            ModelCallExecutor::new(Arc::new(move |_, _| Some(success.clone())), service.clone());
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let mut call = execution(
            request("thread-1", &submission.turn_id),
            vec![candidate(1, "primary", "model", 0.01)],
            1,
            cancel_rx,
        );
        call.remaining_budget_usd = Some(0.5);
        executor.execute(call).await.unwrap();
        let completed = service
            .events_after("thread-1", Some(&submission.cursor), 10)
            .unwrap()
            .events
            .into_iter()
            .find(|row| row.event.event_type == OperatorEventType::RouteCompleted)
            .unwrap();
        assert_eq!(completed.event.payload["remaining_budget_usd"], 0.0);

        let (_evidence, service, submission) = service_and_turn();
        let executor = ModelCallExecutor::new(Arc::new(|_, _| None), service);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let mut call = execution(
            request("thread-1", &submission.turn_id),
            vec![candidate(1, "primary", "model", 0.01)],
            1,
            cancel_rx,
        );
        call.remaining_budget_usd = Some(f64::NAN);
        assert!(matches!(
            executor.execute(call).await.unwrap_err(),
            ModelCallError::InvalidBudget(_)
        ));

        let (_evidence, service, submission) = service_and_turn();
        let nonfinite = Arc::new(EventAdapter {
            events: vec![StreamEvent::Done(TokenUsage {
                cost_usd: f64::INFINITY,
                ..TokenUsage::default()
            })],
        }) as Arc<dyn ProviderAdapter>;
        let executor =
            ModelCallExecutor::new(Arc::new(move |_, _| Some(nonfinite.clone())), service);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let error = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![candidate(1, "primary", "model", 0.01)],
                1,
                cancel_rx,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ModelCallError::AttemptsExhausted {
                class: ProviderFailureClass::InvalidUsage,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn provider_qualified_exclusion_keeps_same_named_secondary_eligible() {
        let (_evidence, service, submission) = service_and_turn();
        let adapters: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::from([
            (
                "primary".to_string(),
                Arc::new(EventAdapter {
                    events: vec![StreamEvent::Error("rate_limited".to_string())],
                }) as Arc<dyn ProviderAdapter>,
            ),
            (
                "secondary".to_string(),
                Arc::new(EventAdapter {
                    events: vec![StreamEvent::Done(TokenUsage::default())],
                }) as Arc<dyn ProviderAdapter>,
            ),
        ]);
        let executor = ModelCallExecutor::new(
            Arc::new(move |provider, _| adapters.get(provider).cloned()),
            service,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let result = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![
                    candidate(1, "primary", "shared-name", 0.01),
                    candidate(2, "secondary", "shared-name", 0.02),
                ],
                3,
                cancel_rx,
            ))
            .await
            .unwrap();

        assert_eq!(result.provider, "secondary");
        assert_eq!(result.failed_models, vec!["primary/shared-name"]);
    }

    #[tokio::test]
    async fn estimated_success_cost_is_returned_in_usage_and_cumulative_truth() {
        let (_evidence, service, submission) = service_and_turn();
        let adapter = Arc::new(EventAdapter {
            events: vec![StreamEvent::Done(TokenUsage::default())],
        }) as Arc<dyn ProviderAdapter>;
        let executor = ModelCallExecutor::new(Arc::new(move |_, _| Some(adapter.clone())), service);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let result = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![candidate(1, "primary", "model", 0.25)],
                1,
                cancel_rx,
            ))
            .await
            .unwrap();

        assert_eq!(result.usage.cost_usd, 0.25);
        assert_eq!(result.cost_usd, 0.25);
        assert_eq!(result.cost_truth, CostTruth::TargetOnly);
    }

    #[tokio::test]
    async fn cumulative_cost_overflow_stays_finite_and_downgrades_truth() {
        let (_evidence, service, submission) = service_and_turn();
        let adapters: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::from([
            (
                "primary".to_string(),
                Arc::new(EventAdapter {
                    events: vec![StreamEvent::Error("unavailable".to_string())],
                }) as Arc<dyn ProviderAdapter>,
            ),
            (
                "secondary".to_string(),
                Arc::new(EventAdapter {
                    events: vec![StreamEvent::Done(TokenUsage::default())],
                }) as Arc<dyn ProviderAdapter>,
            ),
        ]);
        let executor = ModelCallExecutor::new(
            Arc::new(move |provider, _| adapters.get(provider).cloned()),
            service,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let mut model_request = request("thread-1", &submission.turn_id);
        model_request.maximum_marginal_cost_usd = None;
        let mut call = execution(
            model_request,
            vec![
                candidate(1, "primary", "primary-model", f64::MAX),
                candidate(2, "secondary", "secondary-model", f64::MAX),
            ],
            2,
            cancel_rx,
        );
        call.remaining_budget_usd = None;

        let result = executor.execute(call).await.unwrap();

        assert_eq!(result.cost_usd, f64::MAX);
        assert!(result.cost_usd.is_finite());
        assert_eq!(result.cost_truth, CostTruth::CannotConfirm);
    }

    #[tokio::test]
    async fn transient_delta_is_observable_before_provider_done() {
        let (_evidence, service, submission) = service_and_turn();
        let release = Arc::new(tokio::sync::Notify::new());
        let adapter = Arc::new(TokenThenWaitAdapter {
            release: release.clone(),
        }) as Arc<dyn ProviderAdapter>;
        let executor = Arc::new(ModelCallExecutor::new(
            Arc::new(move |_, _| Some(adapter.clone())),
            service.clone(),
        ));
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let (delta_tx, mut delta_rx) = mpsc::channel(1);
        let mut call = execution(
            request("thread-1", &submission.turn_id),
            vec![candidate(1, "primary", "model", 0.01)],
            1,
            cancel_rx,
        );
        call.delta_tx = Some(delta_tx);
        let task = tokio::spawn(async move { executor.execute(call).await });

        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(1), delta_rx.recv())
                .await
                .unwrap(),
            Some(StreamEvent::Token(text)) if text == "first"
        ));
        assert!(!task.is_finished(), "Done must still be pending");
        release.notify_one();
        assert_eq!(task.await.unwrap().unwrap().text, "first");
    }

    #[tokio::test]
    async fn done_observed_persists_completed_usage_even_when_cancel_is_already_set() {
        let (_evidence, service, submission) = service_and_turn();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let adapter =
            Arc::new(DoneAndCancelAdapter { cancel: cancel_tx }) as Arc<dyn ProviderAdapter>;
        let executor =
            ModelCallExecutor::new(Arc::new(move |_, _| Some(adapter.clone())), service.clone());

        let result = executor
            .execute(execution(
                request("thread-1", &submission.turn_id),
                vec![candidate(1, "primary", "model", 0.01)],
                1,
                cancel_rx,
            ))
            .await
            .expect("Done is provider completion truth even when cancellation follows");
        assert_eq!(result.cost_usd, 0.01);
        assert_eq!(result.cost_truth, CostTruth::TargetOnly);
        let events = service
            .events_after("thread-1", Some(&submission.cursor), 20)
            .unwrap()
            .events;
        let completed = events
            .iter()
            .find(|row| row.event.event_type == OperatorEventType::RouteCompleted)
            .expect("Done must append route_completed");
        assert_eq!(completed.event.payload["cost_usd"], 0.01);
        assert_eq!(completed.event.payload["cost_truth"], "target_only");
    }

    #[tokio::test]
    async fn partial_primary_output_stops_without_invoking_secondary() {
        let (_evidence, service, submission) = service_and_turn();
        let secondary_sends = Arc::new(AtomicUsize::new(0));
        let primary = Arc::new(EventAdapter {
            events: vec![
                StreamEvent::Token("partial".to_string()),
                StreamEvent::Error("rate_limited".to_string()),
            ],
        }) as Arc<dyn ProviderAdapter>;
        let secondary = Arc::new(CountingDoneAdapter {
            sends: secondary_sends.clone(),
        }) as Arc<dyn ProviderAdapter>;
        let executor = ModelCallExecutor::new(
            Arc::new(move |provider, _| match provider {
                "primary" => Some(primary.clone()),
                "secondary" => Some(secondary.clone()),
                _ => None,
            }),
            service,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let (delta_tx, mut delta_rx) = mpsc::channel(8);
        let mut call = execution(
            request("thread-1", &submission.turn_id),
            vec![
                candidate(1, "primary", "primary-model", 0.01),
                candidate(2, "secondary", "secondary-model", 0.02),
            ],
            3,
            cancel_rx,
        );
        call.delta_tx = Some(delta_tx);

        let error = executor.execute(call).await.unwrap_err();

        assert!(matches!(
            error,
            ModelCallError::AttemptsExhausted { attempts: 1, .. }
        ));
        assert_eq!(secondary_sends.load(Ordering::SeqCst), 0);
        assert!(matches!(
            delta_rx.recv().await,
            Some(StreamEvent::Token(text)) if text == "partial"
        ));
        assert!(delta_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn partial_output_with_invalid_done_usage_stops_without_secondary() {
        let (_evidence, service, submission) = service_and_turn();
        let secondary_sends = Arc::new(AtomicUsize::new(0));
        let primary = Arc::new(EventAdapter {
            events: vec![
                StreamEvent::Token("partial".to_string()),
                StreamEvent::Done(TokenUsage {
                    cost_usd: f64::INFINITY,
                    ..TokenUsage::default()
                }),
            ],
        }) as Arc<dyn ProviderAdapter>;
        let secondary = Arc::new(CountingDoneAdapter {
            sends: secondary_sends.clone(),
        }) as Arc<dyn ProviderAdapter>;
        let executor = ModelCallExecutor::new(
            Arc::new(move |provider, _| match provider {
                "primary" => Some(primary.clone()),
                "secondary" => Some(secondary.clone()),
                _ => None,
            }),
            service,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let (delta_tx, mut delta_rx) = mpsc::channel(8);
        let mut call = execution(
            request("thread-1", &submission.turn_id),
            vec![
                candidate(1, "primary", "primary-model", 0.01),
                candidate(2, "secondary", "secondary-model", 0.02),
            ],
            3,
            cancel_rx,
        );
        call.delta_tx = Some(delta_tx);

        let error = executor.execute(call).await.unwrap_err();

        assert!(matches!(
            error,
            ModelCallError::AttemptsExhausted {
                attempts: 1,
                class: ProviderFailureClass::InvalidUsage,
                ..
            }
        ));
        assert_eq!(secondary_sends.load(Ordering::SeqCst), 0);
        assert!(matches!(
            delta_rx.recv().await,
            Some(StreamEvent::Token(text)) if text == "partial"
        ));
    }

    #[tokio::test]
    async fn missing_resolver_spends_zero_and_preserves_budget_for_secondary() {
        let (_evidence, service, submission) = service_and_turn();
        let secondary = Arc::new(EventAdapter {
            events: vec![StreamEvent::Done(TokenUsage::default())],
        }) as Arc<dyn ProviderAdapter>;
        let executor = ModelCallExecutor::new(
            Arc::new(move |provider, _| (provider == "secondary").then(|| secondary.clone())),
            service,
        );
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let mut model_request = request("thread-1", &submission.turn_id);
        model_request.maximum_marginal_cost_usd = Some(0.10);
        let primary = candidate(1, "primary", "primary-model", 0.06);
        let mut call = execution(
            model_request,
            vec![primary, candidate(2, "secondary", "secondary-model", 0.08)],
            3,
            cancel_rx,
        );
        call.remaining_budget_usd = Some(0.10);

        let result = executor.execute(call).await.unwrap();

        assert_eq!(result.provider, "secondary");
        assert_eq!(result.cost_usd, 0.08);
        assert_eq!(result.attempt_records[0].cost_usd, None);
        assert!(!result.attempt_records[0].provider_invoked);
    }

    #[tokio::test]
    async fn loop_cancel_waits_for_executor_adapter_task_drop() {
        let evidence_dir = tempfile::tempdir().unwrap();
        std::env::set_var("HEIWA_EVIDENCE_DIR", evidence_dir.path());
        let service = Arc::new(OperatorSessionService::new(
            OperatorJournal::new(evidence_dir.path().to_path_buf()).unwrap(),
        ));
        let started = Arc::new(tokio::sync::Notify::new());
        let interrupted = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let adapter = Arc::new(BlockingAdapter {
            started: started.clone(),
            interrupted: interrupted.clone(),
            dropped: dropped.clone(),
        }) as Arc<dyn ProviderAdapter>;
        let executor = Arc::new(ModelCallExecutor::new(
            Arc::new(move |_, _| Some(adapter.clone())),
            service.clone(),
        ));
        let caller = Arc::new(ExecutorLoopCaller::new(executor));
        let controller = Arc::new(LoopController::new(
            LoopConfig {
                user_id: "test-user".to_string(),
                objective: "cancel active executor".to_string(),
                max_turns: 1,
                max_cost_usd: 1.0,
                intent: "code".to_string(),
                risk: "low".to_string(),
                privacy: "standard".to_string(),
                runtime: "any".to_string(),
                approved: true,
            },
            vec![candidate(1, "primary", "model", 0.01).tier],
        ));
        let (status_tx, _status_rx) = mpsc::channel(4);
        let run_controller = controller.clone();
        let run = tokio::spawn(async move { run_controller.run(status_tx, caller).await });

        started.notified().await;
        controller.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), run)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        assert!(dropped.load(Ordering::SeqCst));
        assert!(interrupted.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn executor_loop_caller_fails_high_risk_closed_until_explicitly_approved() {
        let evidence = tempfile::tempdir().unwrap();
        let service = Arc::new(OperatorSessionService::new(
            OperatorJournal::new(evidence.path().to_path_buf()).unwrap(),
        ));
        let sends = Arc::new(AtomicUsize::new(0));
        let adapter = Arc::new(CountingDoneAdapter {
            sends: sends.clone(),
        }) as Arc<dyn ProviderAdapter>;
        let executor = Arc::new(ModelCallExecutor::new(
            Arc::new(move |_, _| Some(adapter.clone())),
            service.clone(),
        ));
        let caller = ExecutorLoopCaller::new(executor);

        let make_request = |call_id: &str, safety: SafetyClass| {
            let (_cancel_tx, cancel) = watch::channel(false);
            LoopCallRequest {
                thread_id: "loop-thread".to_string(),
                turn_id: format!("turn-{call_id}"),
                call_id: call_id.to_string(),
                stage: ModelCallStage::Execution,
                intent: "deploy".to_string(),
                raw_text: "perform high-risk action".to_string(),
                privacy: PrivacyClass::Standard,
                risk: CallRisk::High,
                safety,
                messages: vec![Message {
                    role: Role::User,
                    content: "perform high-risk action".to_string(),
                }],
                candidates: vec![candidate(1, "primary", "model", 0.01)],
                remaining_budget_usd: Some(1.0),
                prior_failed_models: vec![],
                max_attempts: 1,
                cancel,
            }
        };

        let error = caller
            .call(make_request("unapproved-call", SafetyClass::Unapproved))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("authority_approval_required"));
        assert_eq!(sends.load(Ordering::SeqCst), 0);

        caller
            .call(make_request("approved-call", SafetyClass::Approved))
            .await
            .unwrap();
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        let events = service
            .events_after("loop-thread", None, 100)
            .unwrap()
            .events;
        assert!(events.iter().any(|row| {
            row.event.event_type == OperatorEventType::TurnStarted
                && row.event.payload["client_request_id"] == "turn-approved-call"
        }));
        assert_eq!(
            events
                .iter()
                .filter(|row| row.event.event_type == OperatorEventType::UserMessage)
                .count(),
            2,
            "one user message per admitted loop iteration"
        );
        assert_eq!(
            events
                .iter()
                .filter(|row| row.event.event_type == OperatorEventType::AssistantCompleted)
                .count(),
            1,
            "successful loop iteration needs one durable assistant completion"
        );
        assert_eq!(
            events
                .iter()
                .filter(|row| row.event.event_type == OperatorEventType::TurnCompleted)
                .count(),
            1,
            "successful loop iteration needs one terminal event"
        );
        assert_eq!(
            events
                .iter()
                .filter(|row| row.event.event_type == OperatorEventType::TurnInterrupted)
                .count(),
            1,
            "rejected loop iteration needs one honest terminal event"
        );
        let thread = service.thread("loop-thread").unwrap();
        assert_eq!(
            thread
                .turns
                .iter()
                .filter(|turn| turn.status == "open")
                .count(),
            0,
            "loop calls must not return with durable open turns"
        );
    }

    #[tokio::test]
    async fn executor_loop_caller_never_executes_duplicate_admission() {
        let evidence = tempfile::tempdir().unwrap();
        let service = Arc::new(OperatorSessionService::new(
            OperatorJournal::new(evidence.path().to_path_buf()).unwrap(),
        ));
        let mut admitted = StartTurnRequest::auto("turn-duplicate-call", "duplicate loop work");
        admitted.route_policy.turn_budget_usd = Some(1.0);
        service.start_turn("loop-thread", admitted).unwrap();
        let sends = Arc::new(AtomicUsize::new(0));
        let adapter = Arc::new(CountingDoneAdapter {
            sends: sends.clone(),
        }) as Arc<dyn ProviderAdapter>;
        let executor = Arc::new(ModelCallExecutor::new(
            Arc::new(move |_, _| Some(adapter.clone())),
            service,
        ));
        let caller = ExecutorLoopCaller::new(executor);
        let (_cancel_tx, cancel) = watch::channel(false);

        let error = caller
            .call(LoopCallRequest {
                thread_id: "loop-thread".to_string(),
                turn_id: "turn-duplicate-call".to_string(),
                call_id: "call-duplicate".to_string(),
                stage: ModelCallStage::Execution,
                intent: "code".to_string(),
                raw_text: "duplicate loop work".to_string(),
                privacy: PrivacyClass::Standard,
                risk: CallRisk::Low,
                safety: SafetyClass::Approved,
                messages: vec![Message {
                    role: Role::User,
                    content: "duplicate loop work".to_string(),
                }],
                candidates: vec![candidate(1, "primary", "model", 0.01)],
                remaining_budget_usd: Some(1.0),
                prior_failed_models: vec![],
                max_attempts: 1,
                cancel,
            })
            .await
            .unwrap_err();

        assert!(error.to_string().contains("duplicate"), "{error}");
        assert_eq!(
            sends.load(Ordering::SeqCst),
            0,
            "duplicate admission must not invoke provider"
        );
    }
}
