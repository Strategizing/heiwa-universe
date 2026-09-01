mod cli;
mod cmd;
mod home;

use anyhow::{anyhow, Result};
use chrono::Utc;
use heiwa_core::drex::{
    default_policy, plan_route, preflight_execution, CallRisk, CostTruth, DrexIngress,
    ExecutionLocality, ExecutionMode, ModelCallCandidate, ModelCallRequest, ModelCallStage,
    PrivacyClass, SafetyClass,
};
use heiwa_protocol::{
    parse_turn_intent, CockpitCommand, CockpitEvent, ExecutionRole, ExecutionScope, Permission,
    PrincipalKind, RiskClass, RoutingState, SessionPrincipal, SessionState, ToolCallReceipt,
    ToolLease, TranscriptBlock,
};
use heiwa_provider::adapter::{Message, ProviderAdapter, Role, TokenUsage};
use heiwa_repl::{parse_input, render_footer, ReplCommand, TelemetryState};
use heiwa_shell::agentic;
use heiwa_shell::model_calls::{
    ExecutorLoopCaller, ModelCallExecution, ModelCallExecutor, ModelCallResult,
};
use heiwa_shell::operator::{
    OperatorModelTurn, OperatorPreparationContext, OperatorStreamFrame, OperatorTurnPreparation,
    OperatorTurnRunner, OperatorTurnWork,
};
use serde_json::Value;
use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use heiwa_provider::routing::{
    canonical_provider_id, is_supported as provider_supports_loop_adapter,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoutePreference {
    Auto,
    LocalOnly,
    RemoteOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CockpitMode {
    Direct,
    Agentic,
}

impl CockpitMode {
    fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Agentic => "agentic",
        }
    }
}

// ---------------------------------------------------------------------------
// Shared session state — used by both plain REPL and cockpit controller
// ---------------------------------------------------------------------------

struct SessionPins {
    pinned_provider: Option<String>,
    pinned_model: Option<String>,
    route_preference: RoutePreference,
    cockpit_mode: CockpitMode,
    current_provider: String,
    current_model: String,
    principal: SessionPrincipal,
    scope: ExecutionScope,
}

impl SessionPins {
    fn new() -> Self {
        let working_dir = env::current_dir().unwrap_or_else(|_| heiwa_install::get_heiwa_dir());
        let mut scope = ExecutionScope::local_default(working_dir);
        grant_tool_lease(&mut scope, "shell", RiskClass::HostMutating);
        grant_tool_lease(&mut scope, "fs.read", RiskClass::HostSafeReadonly);
        grant_tool_lease(&mut scope, "fs.list", RiskClass::HostSafeReadonly);
        grant_tool_lease(&mut scope, "repo.grep", RiskClass::HostSafeReadonly);
        Self {
            pinned_provider: None,
            pinned_model: None,
            route_preference: RoutePreference::Auto,
            cockpit_mode: CockpitMode::Direct,
            current_provider: String::new(),
            current_model: String::new(),
            principal: SessionPrincipal::new(
                "agent:local-shell",
                PrincipalKind::Agent,
                ExecutionRole::Agent,
            ),
            scope,
        }
    }
}

fn grant_tool_lease(scope: &mut ExecutionScope, name: &str, risk_class: RiskClass) {
    if !scope.tool_leases.iter().any(|lease| lease.name == name) {
        scope.tool_leases.push(ToolLease {
            name: name.to_string(),
            risk_class,
            allowed: true,
        });
    }
}

/// Result of successfully routing a task to a model.
#[derive(Clone)]
struct RouteResult {
    candidates: Vec<ModelCallCandidate>,
    local_auxiliary_candidates: Vec<ModelCallCandidate>,
    model_id: String,
    provider: String,
    provider_model_id: String,
    rate_group: String,
    routing_metadata: String,
    intent_key: String,
    privacy: String,
    request_id: String,
    turn_started_at: String,
}

fn resolved_route_after_model_call(planned: &RouteResult, result: &ModelCallResult) -> RouteResult {
    let mut resolved = planned.clone();
    resolved.provider = result.provider.clone();
    resolved.model_id = result.model_id.clone();
    resolved.provider_model_id = result.provider_model_id.clone();
    resolved.rate_group = result.rate_group.clone();
    resolved.candidates.retain(|candidate| {
        let identity = format!("{}/{}", candidate.tier.provider, candidate.tier.model_id);
        !result.failed_models.contains(&identity)
    });
    resolved.routing_metadata = serde_json::json!({
        "planned": serde_json::from_str::<serde_json::Value>(&planned.routing_metadata)
            .unwrap_or_else(|_| serde_json::Value::String(planned.routing_metadata.clone())),
        "executed": {
            "provider": result.provider,
            "model_id": result.model_id,
            "provider_model_id": result.provider_model_id,
            "rate_group": result.rate_group,
            "attempts": result.attempts,
            "failed_models": result.failed_models,
            "cost_usd": result.cost_usd,
            "cost_truth": result.cost_truth,
        }
    })
    .to_string();
    resolved
}

#[derive(Debug, Clone)]
struct PreparedRoutePrompt {
    model_prompt: String,
    compression: Option<RouteCompressionMetadata>,
}

#[derive(Debug, Clone)]
struct RouteCompressionMetadata {
    applied: bool,
    reason: String,
    receipt_path: Option<String>,
    input_chars: usize,
    output_chars: usize,
    ratio: f64,
    input_tokens: usize,
    output_tokens: usize,
    estimated_usd_saved: f64,
}

#[derive(Debug, Clone)]
struct RouteCompressionResult {
    compressed: String,
    receipt_path: String,
    input_chars: usize,
    output_chars: usize,
    ratio: f64,
    input_tokens: usize,
    output_tokens: usize,
    estimated_usd_saved: f64,
}

/// Outcome of the routing pipeline.
enum RouteOutcome {
    /// Task routed to a model, ready to stream.
    Routed(Box<RouteResult>),
    /// DREX returned a deterministic response (no model needed).
    Deterministic(String),
}

const DEFAULT_SESSION_ID: &str = "default";
const TRANSCRIPT_CHAR_BUDGET: usize = 16_000;
const ROUTE_COMPRESSION_BYTE_THRESHOLD: usize = 4096;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    let is_tty = std::io::stdout().is_terminal();
    let is_plain = args.iter().any(|arg| arg == "--plain");
    let use_cockpit = is_tty && !is_plain;

    if args.len() < 2 || (args.len() == 2 && args[1] == "--plain") {
        run_repl(use_cockpit).await?;
        return Ok(());
    }

    if cli::try_handle(&args).await? {
        return Ok(());
    }

    match args[1].as_str() {
        "install" => match heiwa_install::run_install_target(args.get(2).map(String::as_str))? {
            heiwa_install::InstallOutcome::RuntimeBootstrap => {
                println!("Registering device...");
                register_current_device().await?;
            }
            heiwa_install::InstallOutcome::Plugin(plugin) => {
                println!("Plugin installed: {}", plugin.canonical);
                println!("  Path:   {}", plugin.install_dir.display());
                println!("  Remote: {}", plugin.clone_url);
                if let Some(reference) = plugin.source.reference.as_deref() {
                    println!("  Ref:    {}", reference);
                }
            }
        },
        // First run, inside the application. The roadmap's L2 requirement is
        // that a user reaches a working install without reading docs, so this
        // reports what is missing and what to do about each gap.
        "setup" => {
            let name = flag_value(&args, "--name");
            run_setup(name.as_deref()).await?;
        }
        "whoami" => match heiwa_identity::load() {
            Ok(Some(identity)) => {
                println!("{}", identity.display_name);
                println!("  installation: {}", identity.installation_id);
                println!("  created:      {}", identity.created_at);
            }
            Ok(None) => {
                println!("No local identity yet. Run `heiwa setup --name \"<your name>\"`.");
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        },
        // One non-interactive turn. This is the scriptable entry point — a
        // fresh install can prove itself without a TTY, and CI can drive a
        // real turn end to end rather than approximating one.
        "ask" => {
            let prompt = args[2..].join(" ");
            if prompt.trim().is_empty() {
                eprintln!("Usage: heiwa ask <prompt>");
                std::process::exit(2);
            }
            match execute_repl_turn(&prompt).await {
                Ok((response, _trace)) => println!("{}", response.trim_end()),
                Err(message) => {
                    eprintln!("{message}");
                    std::process::exit(1);
                }
            }
        }
        "login" => {
            if args.len() < 3 {
                println!("Usage: heiwa login [token]");
            } else {
                let identity = heiwa_provider::login_heiwa(&args[2])?;
                println!(
                    "Successfully logged in as {} ({})",
                    identity.display_name.as_deref().unwrap_or_default(),
                    identity.user_id
                );

                println!("Run 'heiwa register' to record this device locally.");
            }
        }
        "logout" => {
            heiwa_provider::clear_identity()?;
            println!("Successfully logged out from Heiwa.");
        }
        "register" => {
            register_current_device().await?;
        }
        "receipts" => {
            let receipts_db = heiwa_install::get_heiwa_dir().join("receipts.db");
            let evidence_dir = heiwa_evidence::journal_root()?;
            let receipts_dir = heiwa_evidence::receipts_root()?;
            println!("Receipts are recorded locally:");
            println!(
                "  SQLite:   {} ({})",
                receipts_db.display(),
                if receipts_db.exists() {
                    "present"
                } else {
                    "not created yet"
                }
            );
            println!(
                "  Journal:  {} ({})",
                evidence_dir.display(),
                if evidence_dir.exists() {
                    "present"
                } else {
                    "not created yet"
                }
            );
            println!(
                "  Receipts: {} ({})",
                receipts_dir.display(),
                if receipts_dir.exists() {
                    "present"
                } else {
                    "not created yet"
                }
            );
            let streams = heiwa_evidence::journal_summary(&evidence_dir)?;
            if !streams.is_empty() {
                println!("  Journal streams:");
                for stream in streams {
                    if stream.skipped_lines > 0 {
                        println!(
                            "    {:<24} {} event(s), {} unreadable line(s)",
                            stream.kind, stream.events, stream.skipped_lines
                        );
                    } else {
                        println!("    {:<24} {} event(s)", stream.kind, stream.events);
                    }
                }
            }
        }
        "devices" => {
            let manifest_path = heiwa_install::get_heiwa_dir().join("machine.json");
            if manifest_path.exists() {
                let content = std::fs::read_to_string(&manifest_path)?;
                let manifest: serde_json::Value = serde_json::from_str(&content)?;
                println!("Devices:");
                println!(
                    "  ID:       {}",
                    manifest["device_id"].as_str().unwrap_or("unknown")
                );
                println!(
                    "  Hostname: {}",
                    manifest["hostname"].as_str().unwrap_or("unknown")
                );
                println!(
                    "  OS:       {}",
                    manifest["os"].as_str().unwrap_or("unknown")
                );
                println!(
                    "  Arch:     {}",
                    manifest["arch"].as_str().unwrap_or("unknown")
                );
                println!(
                    "  Installed: {}",
                    manifest["installed_at"].as_str().unwrap_or("unknown")
                );

                println!("  Sync:     local-first (evidence under ~/.heiwa/evidence/; GitHub sync planned, gated on redaction — local-only today)");
            } else {
                println!("No device registered. Run 'heiwa install' first.");
            }
        }
        "doctor" => {
            let report = heiwa_install::check_installation()?;
            let include_ai_ops = args.iter().any(|arg| arg == "--ai-ops");
            let json_output = args.iter().any(|arg| arg == "--json");
            let identity = heiwa_provider::load_identity();
            let app_probe = crate::cmd::app::probe_local_app(crate::cmd::app::DEFAULT_PORT);
            let layout = heiwa_install::check_runtime_layout();
            let evidence_dir = heiwa_evidence::journal_root()?;
            let evidence_streams = heiwa_evidence::journal_summary(&evidence_dir)
                .unwrap_or_default()
                .into_iter()
                .map(|stream| {
                    serde_json::json!({
                        "kind": stream.kind,
                        "events": stream.events,
                        "skipped_lines": stream.skipped_lines,
                    })
                })
                .collect::<Vec<_>>();
            let evidence_status = serde_json::json!({
                "backend": "local-jsonl",
                "dir": evidence_dir.display().to_string(),
                "present": evidence_dir.exists(),
                "receipts_dir": heiwa_evidence::receipts_root()?.display().to_string(),
                "streams": evidence_streams,
            });
            let provider_statuses: Vec<heiwa_provider::LegacyProviderAccount> =
                ["claude", "codex", "gemini", "antigravity", "ollama"]
                    .iter()
                    .filter_map(|p| heiwa_provider::get_auth_status(p))
                    .collect();
            let provider_registry = heiwa_provider::AccountRegistry::load();
            let provider_accounts = provider_registry
                .accounts
                .iter()
                .map(|account| {
                    serde_json::json!({
                        "account_id": account.account_id,
                        "provider": account.provider,
                        "auth_kind": account.credential.kind_label(),
                        "rate_group": account.rate_group,
                        "status": &account.status,
                        "model_count": account.models.len(),
                    })
                })
                .collect::<Vec<_>>();
            let ai_ops = if include_ai_ops {
                Some(heiwa_install::check_ai_ops()?)
            } else {
                None
            };

            if json_output {
                let identity_json = identity.as_ref().map(|id| {
                    serde_json::json!({
                        "user_id": id.user_id,
                        "email": id.email,
                        "display_name": id.display_name,
                    })
                });
                println!(
                    "{}",
                    serde_json::json!({
                        "command": "doctor",
                        "runtimes": report,
                        "identity": identity_json,
                        "providers": provider_statuses,
                        "provider_accounts": provider_accounts,
                        "heiwa_app": app_probe,
                        "layout": layout,
                        "evidence": evidence_status,
                        "ai_ops": ai_ops,
                    })
                );
                return Ok(());
            }

            println!("Heiwa Doctor Report:");
            println!(
                "  Rust:   {}",
                report
                    .rust_version
                    .clone()
                    .unwrap_or_else(|| "Not found".to_string())
            );
            println!(
                "  Node:   {}",
                report
                    .node_version
                    .clone()
                    .unwrap_or_else(|| "Not found".to_string())
            );
            println!(
                "  Python: {}",
                report
                    .python_version
                    .clone()
                    .unwrap_or_else(|| "Not found".to_string())
            );
            println!();
            if let Some(identity) = identity {
                println!("Heiwa Identity:");
                println!("  User ID: {}", identity.user_id);
                println!(
                    "  Email:   {}",
                    identity.email.unwrap_or_else(|| "N/A".to_string())
                );
            } else {
                println!("Heiwa Identity: Not logged in (run 'heiwa login')");
            }
            println!();
            println!("Provider Accounts:");
            if provider_registry.accounts.is_empty() {
                println!("  none registered");
            } else {
                for account in &provider_registry.accounts {
                    println!(
                        "  {:<20} {:<20} ({}) [{:?}] — {} model{}",
                        account.account_id,
                        account.provider,
                        account.credential.kind_label(),
                        account.status,
                        account.models.len(),
                        if account.models.len() == 1 { "" } else { "s" },
                    );
                }
            }
            println!();
            println!("CLI Discovery (auth presence only):");
            for status in &provider_statuses {
                let kind = match status.auth_kind {
                    heiwa_provider::AuthKind::OauthCli => "oauth_cli",
                    heiwa_provider::AuthKind::ApiKey => "api_key",
                    heiwa_provider::AuthKind::RouterApi => "router_api",
                    heiwa_provider::AuthKind::LocalRuntime => "local_runtime",
                    heiwa_provider::AuthKind::CustomProfile => "custom_profile",
                };
                let label = format!("{}:", status.provider_id);
                println!("  {:<12} {} ({})", label, status.status, kind);
                let hint = match status.status.as_str() {
                    "installed_unverified" => {
                        Some(format!("heiwa auth login {}", status.provider_id))
                    }
                    "installed_stopped" if status.provider_id == "ollama" => {
                        Some("ollama serve".to_string())
                    }
                    "not_installed" => match status.provider_id.as_str() {
                        "ollama" => Some("brew install ollama".to_string()),
                        "antigravity" => {
                            Some("connect Antigravity in its provider-owned surface".to_string())
                        }
                        _ => Some(format!(
                            "install {} CLI (see provider docs)",
                            status.provider_id
                        )),
                    },
                    _ => None,
                };
                if let Some(hint) = hint {
                    println!("               Next: {hint}");
                }
            }

            println!();
            println!("Heiwa App:");
            println!("  URL:       {}", app_probe.url);
            println!(
                "  Reachable: {}",
                if app_probe.reachable { "yes" } else { "no" }
            );
            if let Some(ms) = app_probe.latency_ms {
                println!("  Latency:   {ms}ms");
            } else {
                println!("  Next:      heiwa app start --port {}", app_probe.port);
            }

            println!();
            println!("Runtime Layout ({}):", layout.root.display());
            for dir in &layout.directories {
                let status = if dir.exists {
                    if dir.writable {
                        "ok"
                    } else {
                        "read-only"
                    }
                } else {
                    "missing"
                };
                println!("  {:<9} {}", format!("{}:", dir.name), status);
            }
            if !layout.is_complete() {
                println!("  Next: heiwa install");
            }

            println!();
            println!("Evidence:");
            println!("  Backend:       local-jsonl (+ derived Lance index; GitHub sync planned, redaction-gated)");
            println!("  Dir:           {}", evidence_dir.display());
            println!(
                "  Present:       {}",
                if evidence_dir.exists() {
                    "yes"
                } else {
                    "not created yet"
                }
            );

            if let Some(ai_ops) = ai_ops {
                println!();
                println!("AI Ops:");
                print_ai_ops_check("MCP Notion HTTP config", ai_ops.mcp_notion_http);
                print_ai_ops_check("Biome config", ai_ops.biome_configured);
                print_ai_ops_check("npm lint -> Biome", ai_ops.npm_lint_uses_biome);
                print_ai_ops_check("CI Biome gate", ai_ops.ci_lint_uses_biome);
                print_ai_ops_check(
                    "CI Clippy dead_code gate",
                    ai_ops.ci_clippy_dead_code_enforced,
                );
                print_ai_ops_check(
                    "CI unused Rust deps gate",
                    ai_ops.ci_unused_deps_uses_cargo_machete,
                );
                println!(
                    "  Overall: {}",
                    if ai_ops.is_clean() {
                        "Clean"
                    } else {
                        "Needs work"
                    }
                );
            }
        }
        "auth" => {
            if args.len() < 3 {
                println!("Usage: heiwa auth [status|login|logout|add-key] [provider] [key]");
            } else {
                match args[2].as_str() {
                    "status" => {
                        // Auto-discover + show registry accounts
                        let mut registry = heiwa_provider::AccountRegistry::load();
                        heiwa_provider::detect::auto_discover(&mut registry).await;
                        if !registry.accounts.is_empty() {
                            println!("Registered Accounts:");
                            for a in &registry.accounts {
                                println!(
                                    "  {:<20} {:<12} ({}) [{:?}] — {} models",
                                    a.account_id,
                                    a.provider,
                                    a.credential.kind_label(),
                                    a.status,
                                    a.models.len(),
                                );
                            }
                            println!();
                        }
                        // Then show legacy CLI discovery
                        let providers = vec!["claude", "codex", "gemini", "antigravity", "ollama"];
                        println!("CLI Discovery:");
                        for p in providers {
                            if let Some(status) = heiwa_provider::get_auth_status(p) {
                                let loop_capable = if provider_supports_loop_adapter(p) {
                                    " [loop]"
                                } else {
                                    ""
                                };
                                println!(
                                    "  {:<12} {:<20} ({:?}){}",
                                    p, status.status, status.auth_kind, loop_capable
                                );
                            }
                        }
                    }
                    "add-key" => {
                        if args.len() < 5 {
                            println!("Usage: heiwa auth add-key <provider> <api-key>");
                            println!();
                            println!("Providers: anthropic, openai, google, openrouter");
                        } else {
                            let provider = &args[3];
                            let api_key = &args[4];
                            let rate_group = match provider.as_str() {
                                "anthropic" => "anthropic_api",
                                "openai" => "openai_api",
                                "google" => "google_api",
                                "openrouter" => "openrouter",
                                _ => provider.as_str(),
                            };

                            let mut registry = heiwa_provider::AccountRegistry::load();
                            match heiwa_provider::registry::add_api_key_account(
                                &mut registry,
                                provider,
                                api_key,
                                rate_group,
                            ) {
                                Ok(account_id) => {
                                    println!(
                                        "Stored {} API key in Keychain as '{}'",
                                        provider, account_id
                                    );
                                    // Verify key and detect models
                                    print!("Verifying...");
                                    io::stdout().flush()?;
                                    if let Some(account) = registry
                                        .accounts
                                        .iter_mut()
                                        .find(|a| a.account_id == account_id)
                                    {
                                        match heiwa_provider::detect::verify_api_key(account).await
                                        {
                                            Ok(()) => {
                                                println!(
                                                    " {} models available",
                                                    account.models.len()
                                                );
                                                for m in &account.models {
                                                    println!(
                                                        "  {} (class:{})",
                                                        m.model_id, m.capability_class
                                                    );
                                                }
                                                registry.save()?;
                                            }
                                            Err(e) => {
                                                println!(" verification failed: {}", e);
                                                registry.save()?;
                                            }
                                        }
                                    }
                                }
                                Err(e) => eprintln!("Failed to store key: {}", e),
                            }
                        }
                    }
                    "login" => {
                        if args.len() < 4 {
                            println!("Usage: heiwa auth login [provider]");
                        } else {
                            heiwa_provider::login(&args[3])?;
                        }
                    }
                    "logout" => {
                        if args.len() < 4 {
                            println!("Usage: heiwa auth logout [provider]");
                        } else {
                            heiwa_provider::logout(&args[3])?;
                        }
                    }
                    _ => println!("Unknown auth subcommand: {}", args[2]),
                }
            }
        }
        "providers" => {
            let mut registry = heiwa_provider::AccountRegistry::load();
            // Auto-discover local providers (Ollama, etc.)
            let discoveries = heiwa_provider::detect::auto_discover(&mut registry).await;
            for d in &discoveries {
                println!("  [auto] {}", d);
            }

            if !registry.accounts.is_empty() {
                println!("Provider Accounts:");
                for account in &registry.accounts {
                    let model_count = account.models.len();
                    let loop_cap = if provider_supports_loop_adapter(&account.provider) {
                        " [loop]"
                    } else {
                        ""
                    };
                    println!(
                        "  {:<20} {} ({}) [{:?}] — {} model{}{}",
                        account.account_id,
                        account.provider,
                        account.credential.kind_label(),
                        account.status,
                        model_count,
                        if model_count == 1 { "" } else { "s" },
                        loop_cap,
                    );
                }
            }

            // Show discoverable CLIs not yet in the registry
            let cli_providers = vec!["claude", "codex", "gemini", "antigravity"];
            let mut unregistered = Vec::new();
            for p in cli_providers {
                if let Some(status) = heiwa_provider::get_auth_status(p) {
                    let in_registry = registry.accounts.iter().any(|a| a.provider == p);
                    if !in_registry {
                        let loop_cap = if provider_supports_loop_adapter(p) {
                            " [loop]"
                        } else {
                            ""
                        };
                        unregistered.push(format!(
                            "  {:<20} {} ({:?}){}",
                            p, status.status, status.auth_kind, loop_cap,
                        ));
                    }
                }
            }
            if !unregistered.is_empty() {
                println!("CLI Discovery:");
                for line in &unregistered {
                    println!("{}", line);
                }
            }

            if registry.accounts.is_empty() && unregistered.is_empty() {
                println!("No providers connected.");
                println!("  heiwa auth add-key <provider> <key>  — register an API key");
            }
        }
        "models" => {
            let mut registry = heiwa_provider::AccountRegistry::load();
            heiwa_provider::detect::auto_discover(&mut registry).await;
            let models = registry.all_models();
            if models.is_empty() {
                println!("No models detected. Connect a provider first:");
                println!("  heiwa auth add-key anthropic <your-api-key>");
                println!("  heiwa auth add-key openai <your-api-key>");
            } else {
                let mut current_group = String::new();
                for m in &models {
                    if m.rate_group != current_group {
                        current_group = m.rate_group.clone();
                        let account = registry.get(&m.account_id);
                        let kind = account
                            .map(|a| a.credential.kind_label())
                            .unwrap_or("unknown");
                        println!("\n  {} ({}) [rate: {}]", m.provider, kind, m.rate_group);
                    }
                    let truth_marker = match m.inventory_truth {
                        heiwa_provider::InventoryTruth::Verified => "",
                        heiwa_provider::InventoryTruth::Inferred => " ~inferred",
                        heiwa_provider::InventoryTruth::UserConfigured => " *user",
                    };
                    println!(
                        "    {:<24} class:{}  {:>6} ctx  ${:.4}/1k in{}",
                        m.model_id,
                        m.capability_class,
                        format_context(m.context_window),
                        m.cost_per_1k_input,
                        truth_marker,
                    );
                }
                println!();
            }
        }
        "route" => {
            run_route_command(&args).await?;
        }
        "session" => {
            if args.len() >= 3 && args[2] == "attach" {
                println!("Running session attach...");
            } else {
                println!("Usage: heiwa session attach");
            }
        }
        "loop" => {
            if args.len() < 3 {
                println!("Usage: heiwa loop [max_turns] \"objective\" [--intent code] [--risk low] [--privacy standard] [--approved]");
            } else {
                let max_turns = args[2].parse::<u32>().unwrap_or(10);
                let objective = if args.len() >= 4 {
                    args[3..].join(" ")
                } else {
                    "no objective provided".to_string()
                };

                let identity = heiwa_provider::load_identity()
                    .ok_or_else(|| anyhow!("Not logged in. Please run 'heiwa login' first."))?;

                let intent = if let Some(i) = args.iter().position(|a| a == "--intent") {
                    args[i + 1].clone()
                } else {
                    "code".to_string()
                };
                let risk = if let Some(i) = args.iter().position(|a| a == "--risk") {
                    args[i + 1].clone()
                } else {
                    "low".to_string()
                };
                let privacy = if let Some(i) = args.iter().position(|a| a == "--privacy") {
                    args[i + 1].clone()
                } else {
                    "standard".to_string()
                };
                let approved = args.iter().any(|arg| arg == "--approved");

                let config = heiwa_loop::LoopConfig {
                    user_id: identity.user_id,
                    objective,
                    max_turns,
                    max_cost_usd: 1.0,
                    intent,
                    risk,
                    privacy,
                    runtime: "any".to_string(),
                    approved,
                };

                let mut registry = heiwa_provider::AccountRegistry::load();
                heiwa_provider::detect::auto_discover(&mut registry).await;
                let model_tiers = get_live_model_tiers(&registry);
                if model_tiers.is_empty() {
                    return Err(anyhow!(
                        "No loop-capable models found. Run 'heiwa providers' to check."
                    ));
                }

                let controller = heiwa_loop::LoopController::new(config, model_tiers);
                let (tx, mut rx) = tokio::sync::mpsc::channel(10);

                println!("Loop initiated: {}", controller.get_id());

                let caller = default_loop_model_caller().map_err(anyhow::Error::msg)?;

                let c = controller;
                tokio::spawn(async move {
                    if let Err(e) = c.run(tx, caller).await {
                        eprintln!("Loop error: {}", e);
                    }
                });

                while let Some(status) = rx.recv().await {
                    println!(
                        "[{}] Turn: {} | Cost: ${:.4}",
                        status.status, status.current_turn, status.total_cost_usd
                    );
                    if status.status == "COMPLETED"
                        || status.status == "CANCELLED"
                        || status.status == "FAILED"
                    {
                        break;
                    }
                }
            }
        }
        "shell" => {
            let use_cockpit = std::io::stdout().is_terminal();
            run_repl(use_cockpit).await?;
        }
        "--help" | "-h" | "help" => {
            print_help();
        }
        "--version" | "-V" | "version" => {
            println!("heiwa {}", env!("CARGO_PKG_VERSION"));
        }
        _ => {
            println!("Heiwa AI runtime and shell");
            println!("Unknown command: {}", args[1]);
            print_help();
        }
    }

    Ok(())
}

/// Value of a `--flag value` argument, if present.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1)
        .filter(|value| !value.starts_with("--"))
        .cloned()
}

/// Observe this installation and project what first run still needs.
async fn onboarding_state() -> heiwa_identity::onboarding::OnboardingState {
    use heiwa_identity::onboarding::{OnboardingFacts, OnboardingState};

    let paths = heiwa_config::HeiwaPaths::try_resolve();
    let identity = match &paths {
        Some(_) => heiwa_identity::load().ok().flatten(),
        // Without a root there is nowhere to have stored one; reporting
        // "no identity" here would be true but is not the actionable gap.
        None => None,
    };

    let mut registry = heiwa_provider::AccountRegistry::load();
    heiwa_provider::detect::auto_discover(&mut registry).await;
    let fleet = heiwa_provider::health::FleetHealth::project(&registry.accounts);

    OnboardingState::project(&OnboardingFacts {
        has_state_root: paths.is_some(),
        identity: identity.as_ref().map(|id| id.display_name.as_str()),
        has_routable_account: fleet.has_routable_account(),
        provider_guidance: fleet.guidance(),
    })
}

/// First-run setup. Establishes what it can and reports the rest.
async fn run_setup(name: Option<&str>) -> Result<()> {
    // Identity first: it is the anchor the rest attaches to, and it is the
    // one gap this command can close on its own.
    if heiwa_config::HeiwaPaths::try_resolve().is_some() {
        match heiwa_identity::load() {
            Ok(None) => {
                let display_name = name
                    .map(str::to_string)
                    .unwrap_or_else(default_display_name);
                let created = chrono::Utc::now().to_rfc3339();
                match heiwa_identity::establish(&display_name, &created) {
                    Ok(identity) => println!("Created local identity: {}", identity.display_name),
                    Err(error) => {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                }
            }
            Ok(Some(identity)) => {
                if let Some(name) = name.filter(|name| *name != identity.display_name) {
                    match heiwa_identity::rename(name) {
                        Ok(renamed) => println!("Renamed to {}", renamed.display_name),
                        Err(error) => {
                            eprintln!("{error}");
                            std::process::exit(1);
                        }
                    }
                }
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    let state = onboarding_state().await;
    println!("{}", state.report());
    if !state.complete {
        // A non-zero exit is what makes this scriptable: a first-run check in
        // CI or an installer can branch on it.
        std::process::exit(1);
    }
    Ok(())
}

/// A neutral name for an installation the user has not named.
///
/// Deliberately not the OS username: this string is shown in the interface
/// and will travel with connector metadata, and lifting a login name into a
/// display name is the single-seat assumption in miniature.
fn default_display_name() -> String {
    "Heiwa user".to_string()
}

fn print_help() {
    println!("Heiwa — BYOK terminal agent");
    println!();
    println!("Usage: heiwa [COMMAND]");
    println!();
    println!("Commands:");
    println!("  install [gh:owner/repo[@ref]] Bootstrap Heiwa or install a GitHub plugin");
    println!("  login [token]                 Sign in to Heiwa");
    println!("  logout                        Sign out from Heiwa");
    println!("  doctor [--ai-ops] [--json]    Check installation, identity, providers, local app reachability");
    println!("  register                      Register the current device");
    println!("  receipts                      Show run receipt status");
    println!("  devices                       Show registered devices");
    println!("  auth status                   Show all connected accounts and CLI discovery");
    println!("  auth add-key <provider> <key> Register an API key for a provider");
    println!("  auth login <provider>         Login to a provider CLI");
    println!("  auth logout <provider>        Logout from a provider CLI");
    println!("  providers                     List connected accounts and models");
    println!("  models                        List all detected models by rate group");
    println!("  life <command>                Inspect/import life readmodel data");
    println!("  app [runtime status]          Probe local Heiwa.app runtime readiness");
    println!("  workers heartbeat             Register local worker liveness");
    println!("  workers status                Show worker registry");
    println!("  mesh status|enroll            Node identity for this machine (no peers yet)");
    println!("  work list|create              Durable Work on this installation");
    println!("  workspace status|prepare      Repository hold for a Work");
    println!("  auto status|create|tick       Manage local background automations");
    println!("  approvals list|show|decide    Manage local approval packets");
    println!("  mail status|accounts          Mail.app metadata-only bridge probe");
    println!("  setup [--name <name>]         First-run setup: identity, provider, readiness");
    println!("  whoami                        Show this installation's local identity");
    println!("  ask <prompt>                  Run one non-interactive turn and print the reply");
    println!("  route preview <prompt>        Preview DREX routing without execution");
    println!("  session attach                Attach to a Heiwa session");
    println!("  loop [turns] <objective>      Run a bounded execution loop");
    println!("  shell                         Enter interactive mode");
    println!("  help                          Print this message");
}

async fn run_route_command(args: &[String]) -> Result<()> {
    match args.get(2).map(String::as_str) {
        Some("preview") => {
            let prompt = args
                .get(3..)
                .map(|parts| parts.join(" "))
                .unwrap_or_default();
            if prompt.trim().is_empty() {
                println!("Usage: heiwa route preview <prompt>");
                return Ok(());
            }

            let mut registry = heiwa_provider::AccountRegistry::load();
            heiwa_provider::detect::auto_discover(&mut registry).await;
            let model_tiers = get_live_model_tiers(&registry);
            let pins = SessionPins::new();
            let now_unix = Utc::now().timestamp();
            let quota_ledger = open_default_quota_ledger();

            let privacy = privacy_for_task(&prompt);
            println!("route preview");
            println!("  privacy: {}", privacy);
            let quota_lines =
                quota_budget_preview_lines(&model_tiers, quota_ledger.as_ref(), now_unix);
            if !quota_lines.is_empty() {
                println!("  quota:");
                for line in quota_lines {
                    println!("    {line}");
                }
            }

            match route_task_with_quota(
                &prompt,
                &pins,
                &model_tiers,
                quota_ledger.as_ref(),
                now_unix,
            ) {
                Ok(RouteOutcome::Deterministic(response)) => {
                    println!("  mode: deterministic");
                    println!("  response: {}", response);
                }
                Ok(RouteOutcome::Routed(route)) => {
                    let mode = if is_local_provider(&route.provider) {
                        "local_model"
                    } else {
                        "remote_model"
                    };
                    println!("  mode: {}", mode);
                    println!("  intent: {}", route.intent_key);
                    println!("  provider: {}", route.provider);
                    println!("  model: {}", route.model_id);
                    println!("  provider_model: {}", route.provider_model_id);
                    println!("  rate_group: {}", route.rate_group);
                    println!("  metadata: {}", route.routing_metadata);
                }
                Err(error) => {
                    println!("  mode: unavailable");
                    println!("  error: {}", error);
                }
            }
        }
        _ => {
            println!("Usage: heiwa route preview <prompt>");
        }
    }
    Ok(())
}

fn print_ai_ops_check(label: &str, ok: bool) {
    println!("  {:<30} {}", label, if ok { "ok" } else { "missing" });
}

fn format_context(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 {
        format!("{}k", tokens / 1_000)
    } else {
        format!("{}", tokens)
    }
}

async fn register_current_device() -> Result<()> {
    let identity = match heiwa_provider::load_identity() {
        Some(id) => id,
        None => {
            println!("Not logged in. Please run 'heiwa login' first.");
            return Ok(());
        }
    };

    let _report = heiwa_install::check_installation()?;
    let manifest_path = heiwa_install::get_heiwa_dir().join("machine.json");

    let device_id = if manifest_path.exists() {
        let content = std::fs::read_to_string(&manifest_path)?;
        let manifest: serde_json::Value = serde_json::from_str(&content)?;
        manifest["device_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string()
    } else {
        "unknown".to_string()
    };

    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    println!(
        "Registering device {} for user {}...",
        device_id, identity.user_id
    );

    let evidence = EvidenceClient::local();
    evidence.journal(
        "devices",
        serde_json::json!({
            "device_id": device_id,
            "user_id": identity.user_id,
            "hostname": hostname,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        }),
    );

    // Journal provider statuses
    let mut registry = heiwa_provider::AccountRegistry::load();
    heiwa_provider::detect::auto_discover(&mut registry).await;
    for account in &registry.accounts {
        evidence.journal(
            "provider_status",
            serde_json::json!({
                "account_id": account.account_id,
                "provider": account.provider,
                "device_id": device_id,
                "credential_kind": account.credential.kind_label(),
                "status": format!("{:?}", account.status),
                "models": account.models,
            }),
        );
        println!(
            "  Recorded provider {} status: {:?}",
            account.provider, account.status
        );
    }

    println!("Device and provider statuses recorded to ~/.heiwa/evidence/.");
    Ok(())
}

pub(crate) fn get_live_model_tiers(
    registry: &heiwa_provider::AccountRegistry,
) -> Vec<heiwa_protocol::ModelTier> {
    get_live_model_tiers_with(registry, |binary| {
        heiwa_provider::resolve_command(binary).is_some()
    })
}

/// The same projection against an explicit installed-binary probe.
///
/// Health filtering asks the host whether the executor exists, so a test that
/// does not state the answer asserts against whatever the machine has
/// installed: the tier list for a `claude` seat or an `ollama` runtime is
/// non-empty on a developer laptop and empty on a CI runner.
pub(crate) fn get_live_model_tiers_with(
    registry: &heiwa_provider::AccountRegistry,
    is_installed: impl Fn(&str) -> bool,
) -> Vec<heiwa_protocol::ModelTier> {
    // Health-filtered, not stored-status-filtered: a route is only real if
    // the account behind it can execute a turn right now.
    let mut models = registry
        .routable_models_with(is_installed)
        .into_iter()
        .filter(|m| provider_supports_loop_adapter(&m.provider))
        .collect::<Vec<_>>();
    models.sort_by_key(|model| live_model_identity(model));
    let mut used_ids = std::collections::HashSet::new();

    models
        .into_iter()
        .map(|m| {
            let mut strengths = vec!["chat"];
            if m.supports_tools {
                strengths.push("tool_use");
            }
            if m.supports_vision {
                strengths.push("vision");
            }
            if m.capability_class >= 4 {
                strengths.push("advanced_coding");
            }

            let mut id = stable_live_model_id(m);
            while !used_ids.insert(id) {
                id = id.wrapping_add(1);
                if id == 0 {
                    id = 1;
                }
            }

            heiwa_protocol::ModelTier {
                id,
                model_id: m.model_id.clone(),
                provider_model_id: m.provider_model_id.clone(),
                provider: canonical_provider_id(&m.provider).to_string(),
                rate_group: m.rate_group.clone(),
                capability_class: m.capability_class,
                effort_knob: "default".to_string(),
                effort_level: 1,
                cost_per_turn: m.cost_per_1k_input * 4.0, // ~4k tokens/turn estimate
                max_context_tokens: m.context_window,
                strengths_json: serde_json::to_string(&strengths).unwrap_or_default(),
                vram_requirement_mb: 0,
                quantization_type: "none".to_string(),
                kv_cache_strategy: "standard".to_string(),
                enabled: true,
                last_success_rate: 1.0,
                avg_latency_ms: if m.rate_group == "local" { 50 } else { 200 },
                latency_p_95_ms: if m.rate_group == "local" { 100 } else { 500 },
                updated_at: Utc::now().to_rfc3339(),
            }
        })
        .collect()
}

fn live_model_identity(model: &heiwa_provider::DetectedModel) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        canonical_provider_id(&model.provider),
        model.model_id,
        model.provider_model_id,
        model.account_id,
        model.rate_group
    )
}

fn stable_live_model_id(model: &heiwa_provider::DetectedModel) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in live_model_identity(model).bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

/// Local evidence appender: JSONL truth with derived Lance recall.
/// Journals to `~/.heiwa/evidence/<kind>.jsonl`; silently no-ops if the
/// evidence directory cannot be created.
#[derive(Clone)]
pub(crate) struct EvidenceClient(Option<Arc<heiwa_core::evidence::JsonlTransport>>);

impl EvidenceClient {
    pub(crate) fn local() -> Self {
        Self(
            heiwa_core::evidence::JsonlTransport::default_local()
                .ok()
                .map(Arc::new),
        )
    }

    pub(crate) fn is_available(&self) -> bool {
        self.0.is_some()
    }

    pub(crate) fn journal(&self, kind: &str, payload: serde_json::Value) {
        use heiwa_core::evidence::EvidenceTransport;
        if let Some(transport) = &self.0 {
            let _ = transport.journal(kind, payload);
        }
    }
}

fn print_boot_provider_matrix() {
    // At-a-glance provider sync panel shown on shell boot. This is what a
    // premium CLI looks like — the user sees *what's connected* without
    // having to type anything first.
    const GREEN: &str = "\x1b[32m";
    const YELLOW: &str = "\x1b[33m";
    const DIM: &str = "\x1b[2m";
    const RESET: &str = "\x1b[0m";

    println!("{}Provider sync{}", DIM, RESET);
    let providers = ["ollama", "claude", "gemini", "antigravity", "codex"];
    for pid in providers {
        let Some(acc) = heiwa_provider::get_auth_status(pid) else {
            continue;
        };
        let (glyph, colour) = match acc.status.as_str() {
            "connected" | "running" => ("●", GREEN),
            "installed_unverified" | "installed_stopped" => ("○", YELLOW),
            _ => ("·", DIM),
        };
        println!(
            "  {}{}{} {:<12} {}  {}[{}]{}",
            colour, glyph, RESET, pid, acc.status, DIM, acc.rate_group, RESET
        );
    }
    println!(
        "{}  Use /providers to re-sync, /models to list, /help for commands.{}",
        DIM, RESET
    );
    println!();
}

async fn run_repl(use_cockpit: bool) -> Result<()> {
    if !use_cockpit {
        println!("Heiwa Interactive Shell");
        println!("Type /help for commands, !command for shell escape, or enter a task.");
        println!();
        print_boot_provider_matrix();
    }

    let evidence_client = EvidenceClient::local();
    if !use_cockpit {
        println!("  Evidence: local-first (~/.heiwa/evidence/)");
    }

    // Start device heartbeat if connected
    let _heartbeat_device_id = {
        let manifest_path = heiwa_install::get_heiwa_dir().join("machine.json");
        if manifest_path.exists() {
            std::fs::read_to_string(&manifest_path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                .and_then(|m| m["device_id"].as_str().map(|s| s.to_string()))
        } else {
            None
        }
    };

    // Load registry once at REPL start
    let mut registry = heiwa_provider::AccountRegistry::load();
    heiwa_provider::detect::auto_discover(&mut registry).await;
    let model_tiers = get_live_model_tiers(&registry);

    if model_tiers.is_empty() && !use_cockpit {
        println!("No loop-capable models available. Run 'heiwa providers' or 'heiwa auth add-key' to connect.");
    }

    // Receipt store: env × provider × model × agent × tokens × latency ×
    // actual_cost × counterfactual_cost. See docs/architecture/receipts.md.
    let heiwa_home = heiwa_install::get_heiwa_dir();
    let receipts = heiwa_receipts::ReceiptStore::open(heiwa_home.join("receipts.db")).ok();
    let rates = heiwa_receipts::runtime::load_rates_or_default(&heiwa_home);
    if receipts.is_none() {
        debug_log(format_args!(
            "receipts store unavailable; runs will not be recorded"
        ));
    }

    let persisted = heiwa_session::load_transcript(DEFAULT_SESSION_ID)
        .unwrap_or_else(|_| heiwa_session::PersistedTranscript::empty(DEFAULT_SESSION_ID));

    let mut state = SessionState {
        session_id: persisted.session_id.clone(),
        transcript: persisted.blocks(),
        routing: RoutingState {
            current_provider: "none".to_string(),
            current_model: "none".to_string(),
            mode: CockpitMode::Direct.label().to_string(),
            explanation: None,
        },
        devices: vec![],
        receipts: vec![],
    };

    let mut turn_count = 0;
    let mut pins = SessionPins::new();

    if let Some(first) = model_tiers.first() {
        pins.current_provider = first.provider.clone();
        pins.current_model = first.model_id.clone();
        state.routing.current_provider = pins.current_provider.clone();
        state.routing.current_model = pins.current_model.clone();
    }

    if use_cockpit {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<CockpitEvent>();
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<CockpitCommand>();

        // Spawn the async controller — it owns routing, execution, evidence
        let ctrl_evidence = evidence_client.clone();
        let ctrl_tiers = model_tiers.clone();
        let ctrl_session_id = state.session_id.clone();
        let ctrl_transcript = state.transcript.clone();
        tokio::spawn(async move {
            run_cockpit_controller(
                cmd_rx,
                event_tx,
                ctrl_evidence,
                ctrl_tiers,
                ctrl_session_id,
                ctrl_transcript,
            )
            .await;
        });

        // Run TUI on the main thread (blocking) — it owns terminal I/O
        let evidence_available = evidence_client.is_available();
        heiwa_tui::run_cockpit(event_rx, cmd_tx, state, evidence_available)?;

        return Ok(());
    }

    loop {
        let footer_state = TelemetryState {
            provider: if pins.current_provider.is_empty() {
                "none".to_string()
            } else {
                pins.current_provider.clone()
            },
            model: if pins.current_model.is_empty() {
                "none".to_string()
            } else {
                pins.current_model.clone()
            },
            route: current_route_label(
                pins.route_preference,
                pins.pinned_provider.as_deref(),
                pins.pinned_model.as_deref(),
            ),
            status: "ready".to_string(),
            turn_count,
            loop_info: None,
        };

        print!("\r{}", render_footer(&footer_state));
        print!("\n> ");
        io::stdout().flush()?;

        let mut input = String::new();
        let bytes_read = io::stdin().read_line(&mut input)?;
        if bytes_read == 0 {
            // EOF (Ctrl-D or non-TTY stdin closed). Exit cleanly instead of
            // spinning on zero-byte reads forever.
            println!();
            break;
        }
        let input = input.trim();

        if input == "exit" || input == "quit" {
            break;
        }

        let cmd = parse_input(input);
        match cmd {
            ReplCommand::Task(t) => {
                if t.is_empty() {
                    continue;
                }

                match route_task(&t, &pins, &model_tiers) {
                    Err(msg) => {
                        println!("{}", msg);
                        continue;
                    }
                    Ok(RouteOutcome::Deterministic(response)) => {
                        if let Err(error) =
                            execute_deterministic_surface_turn(&state.session_id, &t, &response)
                                .await
                        {
                            eprintln!("Operator turn error: {error}");
                            turn_count += 1;
                            continue;
                        }
                        append_state_block(&mut state, TranscriptBlock::User(t.clone()));
                        append_state_block(
                            &mut state,
                            TranscriptBlock::Assistant(response.clone()),
                        );
                        println!("{}", response);
                        turn_count += 1;
                        continue;
                    }
                    Ok(RouteOutcome::Routed(route)) => {
                        let prepared = prepare_outbound_prompt_for_route(&route, &t).await;
                        let messages = build_messages_from_transcript(
                            &state.transcript,
                            &prepared.model_prompt,
                            &pins,
                        );
                        append_state_block(&mut state, TranscriptBlock::User(t.clone()));
                        let input_text: String = messages
                            .iter()
                            .map(|m| m.content.clone())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let receipt_started = std::time::Instant::now();
                        let (delta_tx, mut delta_rx) = tokio::sync::mpsc::channel(32);
                        let delta_task = tokio::spawn(async move {
                            while let Some(delta) = delta_rx.recv().await {
                                match delta {
                                    heiwa_provider::adapter::StreamEvent::Token(text) => {
                                        print!("{text}");
                                        let _ = io::stdout().flush();
                                    }
                                    heiwa_provider::adapter::StreamEvent::ToolUse {
                                        name, ..
                                    } => {
                                        println!("\n[tool: {name}]");
                                    }
                                    _ => {}
                                }
                            }
                        });
                        let result = match execute_routed_model_call(
                            &route,
                            messages,
                            &state.session_id,
                            &t,
                            Some(delta_tx),
                        )
                        .await
                        {
                            Ok(result) => result,
                            Err(error) => {
                                eprintln!("Model call error: {error}");
                                turn_count += 1;
                                continue;
                            }
                        };
                        let _ = delta_task.await;
                        let resolved_route = resolved_route_after_model_call(&route, &result);
                        pins.current_provider = resolved_route.provider.clone();
                        pins.current_model = resolved_route.model_id.clone();
                        record_route_evidence(&evidence_client, &resolved_route, &t);
                        let usage = Some(usage_for_model_call(&result));
                        let full_response = result.text.clone();
                        println!();
                        let response_for_receipt = full_response.clone();
                        append_state_block(&mut state, TranscriptBlock::Assistant(full_response));

                        if let Some(ref u) = usage {
                            if u.input_tokens > 0 || u.cost_usd > 0.0 {
                                println!(
                                    "  [{} in / {} out | ${:.4}]",
                                    u.input_tokens, u.output_tokens, u.cost_usd
                                );
                            }
                        }
                        record_run_evidence(
                            &evidence_client,
                            &resolved_route,
                            usage.as_ref(),
                            Some(&result),
                        );
                        let receipt_latency_ms = receipt_started.elapsed().as_millis() as i64;
                        if let Some(ref receipts) = receipts {
                            record_call_receipt(
                                receipts,
                                &rates,
                                CallReceiptInput {
                                    result: &result,
                                    usage: usage.as_ref(),
                                    session_id: &state.session_id,
                                    input_text: &input_text,
                                    output_text: &response_for_receipt,
                                    latency_ms: receipt_latency_ms,
                                },
                            );
                        }
                        turn_count += 1;
                    }
                }
            }
            ReplCommand::Shell(s) => {
                println!("Escaping to shell: {}", s);
                match run_scoped_shell(&s, &pins.scope, &pins.principal) {
                    Ok(o) => {
                        io::stdout().write_all(&o.stdout)?;
                        io::stderr().write_all(&o.stderr)?;
                    }
                    Err(e) => eprintln!("Shell error: {}", e),
                }
            }
            ReplCommand::Slash(c, args) => {
                match c.as_str() {
                    // Plain-mode specific: re-discovers providers at call time
                    "providers" => {
                        let mut reg = heiwa_provider::AccountRegistry::load();
                        heiwa_provider::detect::auto_discover(&mut reg).await;
                        let tiers = get_live_model_tiers(&reg);
                        for t in tiers {
                            println!(
                                "  {} ({}) class:{}",
                                t.model_id, t.provider, t.capability_class
                            );
                        }
                    }
                    // Plain-mode specific: runs loop controller inline
                    "loop" => {
                        let max_turns = args
                            .first()
                            .and_then(|s| s.parse::<u32>().ok())
                            .unwrap_or(5);
                        let objective = if args.len() > 1 {
                            args[1..].join(" ")
                        } else {
                            "explore context".to_string()
                        };

                        println!("Starting loop: '{}' ({} turns)", objective, max_turns);

                        let identity = heiwa_provider::load_identity().unwrap_or(
                            heiwa_provider::HeiwaIdentity {
                                user_id: "anonymous".to_string(),
                                auth_token: "".to_string(),
                                email: None,
                                display_name: None,
                            },
                        );

                        let config = heiwa_loop::LoopConfig {
                            user_id: identity.user_id,
                            objective,
                            max_turns,
                            max_cost_usd: 1.0,
                            intent: "research".to_string(),
                            risk: "low".to_string(),
                            privacy: "standard".to_string(),
                            runtime: "any".to_string(),
                            approved: args.iter().any(|arg| arg == "--approved"),
                        };

                        let mut reg = heiwa_provider::AccountRegistry::load();
                        heiwa_provider::detect::auto_discover(&mut reg).await;
                        let loop_tiers = get_live_model_tiers(&reg);

                        let controller = heiwa_loop::LoopController::new(config, loop_tiers);
                        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

                        let caller = match default_loop_model_caller() {
                            Ok(caller) => caller,
                            Err(error) => {
                                eprintln!("Loop caller error: {error}");
                                continue;
                            }
                        };

                        tokio::spawn(async move {
                            let _ = controller.run(tx, caller).await;
                        });

                        while let Some(status) = rx.recv().await {
                            let telemetry = TelemetryState {
                                provider: pins.current_provider.clone(),
                                model: pins.current_model.clone(),
                                route: current_route_label(
                                    pins.route_preference,
                                    pins.pinned_provider.as_deref(),
                                    pins.pinned_model.as_deref(),
                                ),
                                status: status.status.clone(),
                                turn_count,
                                loop_info: Some((status.current_turn, max_turns)),
                            };
                            print!("\r{}\r", render_footer(&telemetry));
                            io::stdout().flush()?;

                            if status.status == "COMPLETED"
                                || status.status == "CANCELLED"
                                || status.status == "FAILED"
                            {
                                println!("\nLoop finished: {}", status.status);
                                break;
                            }
                        }
                    }
                    // All other slash commands use shared handler
                    _ => match handle_slash(&c, &args, &model_tiers, &mut pins) {
                        Some(text) => println!("{}", text),
                        None => break,
                    },
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Cockpit controller — async task that processes CockpitCommands
// ---------------------------------------------------------------------------

async fn run_cockpit_controller(
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<CockpitCommand>,
    event_tx: tokio::sync::mpsc::UnboundedSender<CockpitEvent>,
    evidence_client: EvidenceClient,
    model_tiers: Vec<heiwa_protocol::ModelTier>,
    session_id: String,
    mut transcript: Vec<TranscriptBlock>,
) {
    let mut pins = SessionPins::new();

    if let Some(first) = model_tiers.first() {
        pins.current_provider = first.provider.clone();
        pins.current_model = first.model_id.clone();
    }

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            CockpitCommand::Quit => break,
            CockpitCommand::SubmitInput(input) => {
                let parsed = parse_input(&input);
                match parsed {
                    ReplCommand::Task(t) => {
                        if t.is_empty() {
                            continue;
                        }
                        let _ = event_tx.send(CockpitEvent::StatusUpdate("routing...".into()));
                        if pins.cockpit_mode == CockpitMode::Agentic {
                            if let Err(error) = run_cockpit_operator_turn(
                                &session_id,
                                &t,
                                Some(pins.scope.clone()),
                                &mut pins,
                                &mut transcript,
                                &event_tx,
                            )
                            .await
                            {
                                let _ = event_tx.send(CockpitEvent::StreamError(error));
                            }
                            let _ = event_tx.send(CockpitEvent::StatusUpdate("ready".into()));
                            continue;
                        }

                        match route_task(&t, &pins, &model_tiers) {
                            Err(msg) => {
                                let _ = event_tx.send(CockpitEvent::TranscriptAppend(
                                    TranscriptBlock::Evidence(msg),
                                ));
                                let _ = event_tx.send(CockpitEvent::StatusUpdate("ready".into()));
                                continue;
                            }
                            Ok(RouteOutcome::Deterministic(response)) => {
                                if let Err(error) =
                                    execute_deterministic_surface_turn(&session_id, &t, &response)
                                        .await
                                {
                                    let _ = event_tx.send(CockpitEvent::StreamError(error));
                                    let _ =
                                        event_tx.send(CockpitEvent::StatusUpdate("ready".into()));
                                    continue;
                                }
                                append_controller_block(
                                    &session_id,
                                    &mut transcript,
                                    TranscriptBlock::User(t.clone()),
                                    &event_tx,
                                );
                                append_controller_block(
                                    &session_id,
                                    &mut transcript,
                                    TranscriptBlock::Assistant(response.clone()),
                                    &event_tx,
                                );
                                let _ = event_tx.send(CockpitEvent::TranscriptAppend(
                                    TranscriptBlock::Assistant(response),
                                ));
                                let _ = event_tx.send(CockpitEvent::StatusUpdate("ready".into()));
                                continue;
                            }
                            Ok(RouteOutcome::Routed(route)) => {
                                if pins.cockpit_mode == CockpitMode::Agentic {
                                    if route.provider != "ollama" {
                                        let _ = event_tx.send(CockpitEvent::TranscriptAppend(
                                            TranscriptBlock::Evidence(
                                                "agentic mode currently supports ollama only"
                                                    .to_string(),
                                            ),
                                        ));
                                        let _ = event_tx
                                            .send(CockpitEvent::StatusUpdate("ready".into()));
                                        continue;
                                    }

                                    let _ = event_tx.send(CockpitEvent::StatusUpdate(
                                        "agentic: planning tools...".into(),
                                    ));

                                    let prepared =
                                        prepare_outbound_prompt_for_route(&route, &t).await;
                                    let mut messages = build_messages_from_transcript(
                                        &transcript,
                                        &prepared.model_prompt,
                                        &pins,
                                    );
                                    messages.insert(
                                        1,
                                        Message {
                                            role: Role::System,
                                            content: agentic::tool_instruction_prompt(),
                                        },
                                    );
                                    append_controller_block(
                                        &session_id,
                                        &mut transcript,
                                        TranscriptBlock::User(t.clone()),
                                        &event_tx,
                                    );

                                    let first_result = match collect_adapter_response(
                                        &route,
                                        messages.clone(),
                                        &session_id,
                                        &t,
                                    )
                                    .await
                                    {
                                        Ok(result) => result,
                                        Err(error) => {
                                            let _ = event_tx.send(CockpitEvent::StreamError(error));
                                            let _ = event_tx
                                                .send(CockpitEvent::StatusUpdate("ready".into()));
                                            continue;
                                        }
                                    };
                                    let first_route =
                                        resolved_route_after_model_call(&route, &first_result);
                                    pins.current_provider = first_route.provider.clone();
                                    pins.current_model = first_route.model_id.clone();
                                    let _ =
                                        event_tx.send(CockpitEvent::RoutingUpdate(RoutingState {
                                            current_provider: pins.current_provider.clone(),
                                            current_model: pins.current_model.clone(),
                                            mode: pins.cockpit_mode.label().to_string(),
                                            explanation: Some(first_route.routing_metadata.clone()),
                                        }));
                                    record_route_evidence(&evidence_client, &first_route, &t);
                                    let first_response = first_result.text.clone();
                                    let first_usage = Some(usage_for_model_call(&first_result));

                                    let tool_calls = agentic::parse_tool_calls(&first_response);
                                    if tool_calls.is_empty() {
                                        let _ = event_tx.send(CockpitEvent::StreamToken(
                                            first_response.clone(),
                                        ));
                                        append_controller_block(
                                            &session_id,
                                            &mut transcript,
                                            TranscriptBlock::Assistant(first_response),
                                            &event_tx,
                                        );
                                        send_done_event(&event_tx, first_usage.as_ref());
                                        record_run_evidence(
                                            &evidence_client,
                                            &first_route,
                                            first_usage.as_ref(),
                                            Some(&first_result),
                                        );
                                        let _ = event_tx
                                            .send(CockpitEvent::StatusUpdate("ready".into()));
                                        continue;
                                    }

                                    let _ = event_tx.send(CockpitEvent::StatusUpdate(
                                        "agentic: running tools...".into(),
                                    ));
                                    match agentic::execute_tool_calls(
                                        pins.scope.clone(),
                                        tool_calls,
                                        &first_route.provider,
                                        &first_route.model_id,
                                    )
                                    .await
                                    {
                                        Ok((receipts, tool_entries)) => {
                                            for receipt in &receipts {
                                                record_tool_call_evidence(
                                                    &evidence_client,
                                                    receipt,
                                                    &session_id,
                                                );
                                            }
                                            for entry in &tool_entries {
                                                append_controller_block(
                                                    &session_id,
                                                    &mut transcript,
                                                    TranscriptBlock::Tool(
                                                        entry.name.clone(),
                                                        entry.output.clone(),
                                                    ),
                                                    &event_tx,
                                                );
                                                let _ =
                                                    event_tx.send(CockpitEvent::TranscriptAppend(
                                                        TranscriptBlock::Tool(
                                                            entry.name.clone(),
                                                            entry.output.clone(),
                                                        ),
                                                    ));
                                            }

                                            let _ = event_tx.send(CockpitEvent::StatusUpdate(
                                                "agentic: finalizing...".into(),
                                            ));
                                            messages.push(Message {
                                                role: Role::Assistant,
                                                content: first_response,
                                            });
                                            messages.push(Message {
                                                role: Role::System,
                                                content: agentic::tool_result_prompt(&tool_entries),
                                            });

                                            match collect_adapter_response(
                                                &first_route,
                                                messages,
                                                &session_id,
                                                &t,
                                            )
                                            .await
                                            {
                                                Err(error) => {
                                                    let _ = event_tx
                                                        .send(CockpitEvent::StreamError(error));
                                                }
                                                Ok(final_result) => {
                                                    let aggregate_result =
                                                        aggregate_model_call_results(
                                                            &first_result,
                                                            &final_result,
                                                        );
                                                    let final_route =
                                                        resolved_route_after_model_call(
                                                            &first_route,
                                                            &aggregate_result,
                                                        );
                                                    pins.current_provider =
                                                        final_route.provider.clone();
                                                    pins.current_model =
                                                        final_route.model_id.clone();
                                                    let _ = event_tx.send(
                                                        CockpitEvent::RoutingUpdate(RoutingState {
                                                            current_provider: pins
                                                                .current_provider
                                                                .clone(),
                                                            current_model: pins
                                                                .current_model
                                                                .clone(),
                                                            mode: pins
                                                                .cockpit_mode
                                                                .label()
                                                                .to_string(),
                                                            explanation: Some(
                                                                final_route
                                                                    .routing_metadata
                                                                    .clone(),
                                                            ),
                                                        }),
                                                    );
                                                    let final_response = final_result.text.clone();
                                                    let final_usage = Some(usage_for_model_call(
                                                        &aggregate_result,
                                                    ));
                                                    let _ =
                                                        event_tx.send(CockpitEvent::StreamToken(
                                                            final_response.clone(),
                                                        ));
                                                    append_controller_block(
                                                        &session_id,
                                                        &mut transcript,
                                                        TranscriptBlock::Assistant(final_response),
                                                        &event_tx,
                                                    );
                                                    let usage = final_usage;
                                                    send_done_event(&event_tx, usage.as_ref());
                                                    record_run_evidence(
                                                        &evidence_client,
                                                        &final_route,
                                                        usage.as_ref(),
                                                        Some(&aggregate_result),
                                                    );
                                                }
                                            }
                                        }
                                        Err(error) => {
                                            let _ = event_tx.send(CockpitEvent::StreamError(
                                                format!("tool loop error: {error}"),
                                            ));
                                        }
                                    }
                                    let _ =
                                        event_tx.send(CockpitEvent::StatusUpdate("ready".into()));
                                    continue;
                                }

                                let _ = event_tx
                                    .send(CockpitEvent::StatusUpdate("streaming...".into()));

                                // Stream response
                                let prepared = prepare_outbound_prompt_for_route(&route, &t).await;
                                let messages = build_messages_from_transcript(
                                    &transcript,
                                    &prepared.model_prompt,
                                    &pins,
                                );
                                append_controller_block(
                                    &session_id,
                                    &mut transcript,
                                    TranscriptBlock::User(t.clone()),
                                    &event_tx,
                                );
                                let (delta_tx, mut delta_rx) = tokio::sync::mpsc::channel(32);
                                let delta_events = event_tx.clone();
                                let delta_task = tokio::spawn(async move {
                                    while let Some(delta) = delta_rx.recv().await {
                                        match delta {
                                            heiwa_provider::adapter::StreamEvent::Token(text) => {
                                                let _ = delta_events
                                                    .send(CockpitEvent::StreamToken(text));
                                            }
                                            heiwa_provider::adapter::StreamEvent::ToolUse {
                                                name,
                                                ..
                                            } => {
                                                let _ = delta_events.send(
                                                    CockpitEvent::TranscriptAppend(
                                                        TranscriptBlock::Tool(
                                                            name,
                                                            "executed".to_string(),
                                                        ),
                                                    ),
                                                );
                                            }
                                            _ => {}
                                        }
                                    }
                                });
                                match execute_routed_model_call(
                                    &route,
                                    messages,
                                    &session_id,
                                    &t,
                                    Some(delta_tx),
                                )
                                .await
                                {
                                    Ok(result) => {
                                        let _ = delta_task.await;
                                        let resolved_route =
                                            resolved_route_after_model_call(&route, &result);
                                        pins.current_provider = resolved_route.provider.clone();
                                        pins.current_model = resolved_route.model_id.clone();
                                        let _ = event_tx.send(CockpitEvent::RoutingUpdate(
                                            RoutingState {
                                                current_provider: pins.current_provider.clone(),
                                                current_model: pins.current_model.clone(),
                                                mode: pins.cockpit_mode.label().to_string(),
                                                explanation: Some(
                                                    resolved_route.routing_metadata.clone(),
                                                ),
                                            },
                                        ));
                                        record_route_evidence(
                                            &evidence_client,
                                            &resolved_route,
                                            &t,
                                        );
                                        let usage = usage_for_model_call(&result);
                                        let full_response = result.text.clone();
                                        append_controller_block(
                                            &session_id,
                                            &mut transcript,
                                            TranscriptBlock::Assistant(full_response),
                                            &event_tx,
                                        );
                                        send_done_event(&event_tx, Some(&usage));
                                        record_run_evidence(
                                            &evidence_client,
                                            &resolved_route,
                                            Some(&usage),
                                            Some(&result),
                                        );
                                    }
                                    Err(error) => {
                                        let _ = event_tx.send(CockpitEvent::StreamError(error));
                                    }
                                }
                                let _ = event_tx.send(CockpitEvent::StatusUpdate("ready".into()));
                            }
                        }
                    }
                    ReplCommand::Shell(s) => {
                        match run_scoped_shell(&s, &pins.scope, &pins.principal) {
                            Ok(o) => {
                                let stdout_str = String::from_utf8_lossy(&o.stdout).to_string();
                                let stderr_str = String::from_utf8_lossy(&o.stderr).to_string();
                                let combined = if stderr_str.is_empty() {
                                    stdout_str
                                } else {
                                    format!("{}\n{}", stdout_str, stderr_str)
                                };
                                let _ = event_tx.send(CockpitEvent::TranscriptAppend(
                                    TranscriptBlock::Tool(format!("shell: {}", s), combined),
                                ));
                            }
                            Err(e) => {
                                let _ = event_tx.send(CockpitEvent::TranscriptAppend(
                                    TranscriptBlock::Evidence(format!("shell error: {}", e)),
                                ));
                            }
                        }
                    }
                    ReplCommand::Slash(c, args) => {
                        let msg = handle_slash(&c, &args, &model_tiers, &mut pins);
                        if let Some(text) = msg {
                            let _ = event_tx.send(CockpitEvent::TranscriptAppend(
                                TranscriptBlock::Evidence(text),
                            ));
                        }
                        let _ = event_tx.send(CockpitEvent::RoutingUpdate(RoutingState {
                            current_provider: pins.current_provider.clone(),
                            current_model: pins.current_model.clone(),
                            mode: pins.cockpit_mode.label().to_string(),
                            explanation: None,
                        }));
                        let _ = event_tx.send(CockpitEvent::StatusUpdate("ready".into()));
                    }
                }
            }
        }
    }
}

/// Handle slash commands, returning text to display. Shared by both modes.
fn handle_slash(
    cmd: &str,
    args: &[String],
    model_tiers: &[heiwa_protocol::ModelTier],
    pins: &mut SessionPins,
) -> Option<String> {
    match cmd {
        "help" => Some(
            "commands: /cwd [folder] /add-dir <folder|glob> /dirs /provider [name|auto] /providers /model [name|auto] /models /route [auto|local|remote] /mode [direct|agentic] /status /clear /loop /exit"
                .to_string(),
        ),
        "cwd" => match args.first() {
            None => Some(format!("cwd: {}", pins.scope.working_dir.display())),
            Some(raw) => match resolve_existing_dir(raw, Some(&pins.scope.working_dir)) {
                Ok(path) => {
                    pins.scope.set_working_dir(path.clone());
                    Some(format!("cwd: {}", path.display()))
                }
                Err(error) => Some(error),
            },
        },
        "add-dir" | "adddir" => {
            if args.is_empty() {
                return Some("usage: /add-dir <folder|glob> [more...]".into());
            }
            let mut added = Vec::new();
            let mut errors = Vec::new();
            for raw in args {
                match expand_dir_arg(raw, Some(&pins.scope.working_dir)) {
                    Ok(paths) if paths.is_empty() => errors.push(format!("no matches: {}", raw)),
                    Ok(paths) => {
                        for path in paths {
                            if pins.scope.add_allowed_dir(path.clone()) {
                                added.push(path);
                            }
                        }
                    }
                    Err(error) => errors.push(error),
                }
            }
            let mut lines = Vec::new();
            if !added.is_empty() {
                lines.push(format!(
                    "added dirs:\n{}",
                    added
                        .iter()
                        .map(|path| format!("  {}", path.display()))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
            if !errors.is_empty() {
                lines.push(format!("errors:\n  {}", errors.join("\n  ")));
            }
            if lines.is_empty() {
                lines.push("no new dirs".to_string());
            }
            Some(lines.join("\n"))
        }
        "dirs" => Some(format!(
            "cwd: {}\nallowed dirs:\n{}",
            pins.scope.working_dir.display(),
            pins.scope
                .allowed_dirs
                .iter()
                .map(|path| format!("  {}", path.display()))
                .collect::<Vec<_>>()
                .join("\n")
        )),
        "providers" => {
            if model_tiers.is_empty() {
                Some("no loop-capable providers available".into())
            } else {
                let providers = available_providers(model_tiers);
                let list: Vec<String> = providers
                    .iter()
                    .map(|p| {
                        let count = model_tiers.iter().filter(|t| &t.provider == p).count();
                        format!("{} ({} models)", p, count)
                    })
                    .collect();
                Some(list.join("\n"))
            }
        }
        "auth" => Some("manage auth via 'heiwa auth' in the terminal".into()),
        "loop" => Some("loop: use '/loop [turns] [objective]' in plain mode or 'heiwa loop'".into()),
        "provider" => {
            let available = available_providers(model_tiers);
            match args.first().map(|s| s.as_str()) {
                None => {
                    let active = pins.pinned_provider.as_deref().unwrap_or("auto");
                    Some(format!(
                        "provider: {} | available: {}",
                        active,
                        available.join(", ")
                    ))
                }
                Some("auto") | Some("clear") => {
                    pins.pinned_provider = None;
                    pins.pinned_model = None;
                    Some("provider routing reset to auto".into())
                }
                Some(p) => {
                    if available.iter().any(|x| x == p) {
                        pins.pinned_provider = Some(p.to_string());
                        if let Some(model) = pins.pinned_model.as_ref() {
                            let matches = model_tiers
                                .iter()
                                .any(|t| t.model_id == *model && t.provider == p);
                            if !matches {
                                pins.pinned_model = None;
                            }
                        }
                        Some(format!("pinned provider to {}", p))
                    } else {
                        Some(format!("unknown provider '{}'", p))
                    }
                }
            }
        }
        "model" => match args.first().map(|s| s.as_str()) {
            None => {
                let active = pins.pinned_model.as_deref().unwrap_or("auto");
                let list: Vec<String> = model_tiers
                    .iter()
                    .map(|t| format!("{} ({})", t.model_id, t.provider))
                    .collect();
                Some(format!("model: {} | available: {}", active, list.join(", ")))
            }
            Some("auto") | Some("clear") => {
                pins.pinned_model = None;
                Some("model routing reset to auto".into())
            }
            Some(m) => {
                if let Some(tier) = model_tiers
                    .iter()
                    .find(|t| t.model_id == m || t.provider_model_id == m)
                {
                    pins.pinned_model = Some(tier.model_id.clone());
                    pins.pinned_provider = Some(tier.provider.clone());
                    Some(format!(
                        "pinned model to {} ({})",
                        tier.model_id, tier.provider
                    ))
                } else {
                    Some(format!("unknown model '{}'", m))
                }
            }
        },
        "models" => {
            if model_tiers.is_empty() {
                Some("no loop-capable models available".into())
            } else {
                let list: Vec<String> = model_tiers
                    .iter()
                    .map(|t| {
                        format!(
                            "{} ({}) class:{}",
                            t.model_id, t.provider, t.capability_class
                        )
                    })
                    .collect();
                Some(list.join("\n"))
            }
        }
        "route" => match args.first().map(|s| s.as_str()) {
            None => Some(format!(
                "route: {} | options: auto, local, remote",
                route_preference_label(pins.route_preference)
            )),
            Some("auto") => {
                pins.route_preference = RoutePreference::Auto;
                Some("route preference: auto".into())
            }
            Some("local") => {
                pins.route_preference = RoutePreference::LocalOnly;
                Some("route preference: local-only".into())
            }
            Some("remote") => {
                pins.route_preference = RoutePreference::RemoteOnly;
                Some("route preference: remote-only".into())
            }
            Some(other) => Some(format!("unknown route preference '{}'", other)),
        },
        "mode" => match args.first().map(|s| s.as_str()) {
            None => Some(format!("mode: {}", pins.cockpit_mode.label())),
            Some("direct") => {
                pins.cockpit_mode = CockpitMode::Direct;
                Some("mode: direct".into())
            }
            Some("agentic") => {
                pins.cockpit_mode = CockpitMode::Agentic;
                Some("mode: agentic".into())
            }
            Some(other) => Some(format!("unknown mode '{}'", other)),
        },
        "status" => Some(format!(
            "provider: {} | model: {} | mode: {} | route: {} | pinned_provider: {} | pinned_model: {} | cwd: {} | dirs: {} | sandbox: {:?}",
            if pins.current_provider.is_empty() {
                "none"
            } else {
                &pins.current_provider
            },
            if pins.current_model.is_empty() {
                "none"
            } else {
                &pins.current_model
            },
            pins.cockpit_mode.label(),
            route_preference_label(pins.route_preference),
            pins.pinned_provider.as_deref().unwrap_or("auto"),
            pins.pinned_model.as_deref().unwrap_or("auto"),
            pins.scope.working_dir.display(),
            pins.scope.allowed_dirs.len(),
            pins.scope.sandbox_mode,
        )),
        "clear" => {
            pins.pinned_provider = None;
            pins.pinned_model = None;
            pins.route_preference = RoutePreference::Auto;
            pins.cockpit_mode = CockpitMode::Direct;
            Some("cleared route, provider, and model pins".into())
        }
        "exit" | "quit" => None,
        _ => Some(format!("unknown command: /{}", cmd)),
    }
}

fn resolve_existing_dir(raw: &str, base: Option<&Path>) -> Result<PathBuf, String> {
    let path = expand_home(raw);
    let path = if path.is_absolute() {
        path
    } else {
        base.unwrap_or_else(|| Path::new(".")).join(path)
    };
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("invalid directory '{}': {}", raw, error))?;
    if !canonical.is_dir() {
        return Err(format!("not a directory: {}", canonical.display()));
    }
    Ok(canonical)
}

fn expand_dir_arg(raw: &str, base: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    if let Some(parent_raw) = raw.strip_suffix("/*") {
        let parent = resolve_existing_dir(parent_raw, base)?;
        let mut dirs = Vec::new();
        let entries = std::fs::read_dir(&parent)
            .map_err(|error| format!("cannot read '{}': {}", parent.display(), error))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_dir() {
                dirs.push(
                    entry
                        .path()
                        .canonicalize()
                        .map_err(|error| error.to_string())?,
                );
            }
        }
        dirs.sort();
        return Ok(dirs);
    }

    resolve_existing_dir(raw, base).map(|path| vec![path])
}

fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" {
        return crate::home::heiwa_home().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = crate::home::heiwa_home() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn run_scoped_shell(
    command: &str,
    scope: &ExecutionScope,
    principal: &SessionPrincipal,
) -> Result<std::process::Output, String> {
    let shell_gate = scope.authorize_tool(principal, "shell", Permission::RunShell);
    if !shell_gate.is_allowed() {
        return Err(shell_gate.reason().to_string());
    }
    if !scope.allows_path(&scope.working_dir) {
        return Err(format!(
            "cwd is outside execution scope: {}",
            scope.working_dir.display()
        ));
    }
    if let Some(path) = first_disallowed_path_reference(command, scope) {
        return Err(format!(
            "shell command references path outside execution scope: {}",
            path.display()
        ));
    }

    std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&scope.working_dir)
        .output()
        .map_err(|error| error.to_string())
}

fn first_disallowed_path_reference(command: &str, scope: &ExecutionScope) -> Option<PathBuf> {
    command
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')'))
        .filter_map(normalize_shell_path_token)
        .find(|path| !scope.allows_path(path))
}

fn normalize_shell_path_token(token: &str) -> Option<PathBuf> {
    let token = token
        .trim_matches(|c| matches!(c, '\'' | '"' | '`' | ',' | ':'))
        .trim();
    if token.is_empty() || token.starts_with('-') {
        return None;
    }
    if token.starts_with('/') || token == "~" || token.starts_with("~/") {
        return Some(expand_home(token));
    }
    None
}

fn build_messages_from_transcript(
    transcript: &[TranscriptBlock],
    current_input: &str,
    pins: &SessionPins,
) -> Vec<Message> {
    let mut transcript_messages = Vec::new();
    let mut used_chars = current_input.len();

    for block in transcript.iter().rev() {
        let Some((role, content)) = transcript_block_to_message(block) else {
            continue;
        };
        let content_len = content.len();
        if used_chars + content_len > TRANSCRIPT_CHAR_BUDGET && !transcript_messages.is_empty() {
            break;
        }
        used_chars += content_len;
        transcript_messages.push(Message { role, content });
    }

    transcript_messages.reverse();
    let mut messages = vec![Message {
        role: Role::System,
        content: working_context_prompt(pins),
    }];
    // The turn is persisted before the prompt is built, so the transcript's
    // last entry is usually the message being sent. Appending it again sent
    // the newest message twice — billed twice, and the one most likely to
    // carry pasted context. Append only when it is not already there.
    let already_present = transcript_messages.last().is_some_and(|message| {
        matches!(message.role, Role::User) && message.content == current_input
    });
    messages.extend(transcript_messages);
    if !already_present {
        messages.push(Message {
            role: Role::User,
            content: current_input.to_string(),
        });
    }
    messages
}

async fn prepare_outbound_prompt_for_route(
    route: &RouteResult,
    input: &str,
) -> PreparedRoutePrompt {
    if !route_should_compress(route, input) {
        return PreparedRoutePrompt {
            model_prompt: input.to_string(),
            compression: None,
        };
    }
    let sessions = match default_model_call_runtime() {
        Ok(runtime) => runtime.sessions,
        Err(error) => {
            return prepare_outbound_prompt_for_route_with(route, input, |_, _| Err(error));
        }
    };
    let source = format!(
        "route:{}:{}:{}",
        route.request_id, route.provider, route.intent_key
    );
    let submission = match sessions.start_turn(
        "auxiliary-compression",
        heiwa_session::operator::StartTurnRequest::auto(
            format!("compress-{}", uuid::Uuid::new_v4()),
            format!("compress source={source} chars={}", input.chars().count()),
        ),
    ) {
        Ok(submission) => submission,
        Err(error) => {
            return prepare_outbound_prompt_for_route_with(route, input, |_, _| {
                Err(error.to_string())
            });
        }
    };
    let context = OperatorPreparationContext {
        thread_id: submission.thread_id,
        turn_id: submission.turn_id,
    };
    let result = routed_compression_receipt(
        &context,
        route.local_auxiliary_candidates.clone(),
        input,
        &source,
        vec![],
        Some(pricing_for_provider(&route.provider)),
        None,
    )
    .await;
    let terminal_result = finish_auxiliary_turn(&sessions, &context, result.is_ok(), "compression");
    prepare_outbound_prompt_for_route_with(route, input, |_, _| {
        terminal_result.map_err(|error| error.to_string())?;
        result.map(|receipt| RouteCompressionResult {
            compressed: receipt.compressed,
            receipt_path: receipt.receipt_path,
            input_chars: receipt.input_chars,
            output_chars: receipt.output_chars,
            ratio: receipt.ratio,
            input_tokens: receipt.input_tokens,
            output_tokens: receipt.output_tokens,
            estimated_usd_saved: receipt.estimated_usd_saved,
        })
    })
}

async fn prepare_outbound_prompt_for_route_cancellable(
    context: &OperatorPreparationContext,
    route: &RouteResult,
    input: &str,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
) -> Result<PreparedRoutePrompt, String> {
    if !route_should_compress(route, input) {
        return Ok(PreparedRoutePrompt {
            model_prompt: input.to_string(),
            compression: None,
        });
    }
    let source = format!(
        "route:{}:{}:{}",
        route.request_id, route.provider, route.intent_key
    );
    let result = routed_compression_receipt(
        context,
        route.local_auxiliary_candidates.clone(),
        input,
        &source,
        vec![],
        Some(pricing_for_provider(&route.provider)),
        Some(cancelled.as_ref()),
    )
    .await
    .map(|receipt| RouteCompressionResult {
        compressed: receipt.compressed,
        receipt_path: receipt.receipt_path,
        input_chars: receipt.input_chars,
        output_chars: receipt.output_chars,
        ratio: receipt.ratio,
        input_tokens: receipt.input_tokens,
        output_tokens: receipt.output_tokens,
        estimated_usd_saved: receipt.estimated_usd_saved,
    })
    .map_err(|error| error.to_string());
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err("operator preparation cancelled during compression".to_string());
    }
    Ok(prepare_outbound_prompt_for_route_with(
        route,
        input,
        move |_, _| result,
    ))
}

fn pricing_for_provider(provider: &str) -> cmd::compress::PricingInputs {
    // USD per million tokens, sourced from public 2026-05 list pricing.
    // Defaults bias toward the operator's typical lane (Sonnet 4.6 / GPT-5 / Gemini 3.1).
    // Override via env later if needed. Local providers cost zero.
    let (input_rate, output_rate, token_count_kind, exact_count_source) = match provider {
        "claude" => (
            3.0,
            15.0,
            "proxy_estimate",
            Some("anthropic_messages_count_tokens_api"),
        ),
        "codex" => (3.0, 15.0, "proxy_estimate", None),
        "gemini" => (1.25, 10.0, "proxy_estimate", None),
        "antigravity" => (1.25, 10.0, "proxy_estimate", None),
        "ollama" => (0.0, 0.0, "local_zero_cost", None),
        _ => (5.0, 15.0, "proxy_estimate", None),
    };
    cmd::compress::PricingInputs {
        target_provider: provider.to_string(),
        usd_per_million_input_tokens: input_rate,
        usd_per_million_output_tokens: output_rate,
        tokenizer_id: "cl100k_base".to_string(),
        token_count_kind: token_count_kind.to_string(),
        exact_count_source: exact_count_source.map(str::to_string),
    }
}

fn prepare_outbound_prompt_for_route_with<F>(
    route: &RouteResult,
    input: &str,
    compressor: F,
) -> PreparedRoutePrompt
where
    F: FnOnce(&str, &str) -> Result<RouteCompressionResult, String>,
{
    if !route_should_compress(route, input) {
        return PreparedRoutePrompt {
            model_prompt: input.to_string(),
            compression: None,
        };
    }

    let source = format!(
        "route:{}:{}:{}",
        route.request_id, route.provider, route.intent_key
    );
    match compressor(input, &source) {
        Ok(result) if result.compressed.trim().is_empty() => PreparedRoutePrompt {
            model_prompt: input.to_string(),
            compression: Some(RouteCompressionMetadata {
                applied: false,
                reason: "empty_output".to_string(),
                receipt_path: Some(result.receipt_path),
                input_chars: result.input_chars,
                output_chars: result.output_chars,
                ratio: result.ratio,
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
                estimated_usd_saved: 0.0,
            }),
        },
        Ok(result) if result.output_chars >= result.input_chars => PreparedRoutePrompt {
            model_prompt: input.to_string(),
            compression: Some(RouteCompressionMetadata {
                applied: false,
                reason: "not_smaller".to_string(),
                receipt_path: Some(result.receipt_path),
                input_chars: result.input_chars,
                output_chars: result.output_chars,
                ratio: result.ratio,
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
                estimated_usd_saved: 0.0,
            }),
        },
        Ok(result) => PreparedRoutePrompt {
            model_prompt: result.compressed,
            compression: Some(RouteCompressionMetadata {
                applied: true,
                reason: "compressed".to_string(),
                receipt_path: Some(result.receipt_path),
                input_chars: result.input_chars,
                output_chars: result.output_chars,
                ratio: result.ratio,
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
                estimated_usd_saved: result.estimated_usd_saved,
            }),
        },
        Err(error) => PreparedRoutePrompt {
            model_prompt: input.to_string(),
            compression: Some(RouteCompressionMetadata {
                applied: false,
                reason: format!("failed:{error}"),
                receipt_path: None,
                input_chars: input.chars().count(),
                output_chars: input.chars().count(),
                ratio: 1.0,
                input_tokens: 0,
                output_tokens: 0,
                estimated_usd_saved: 0.0,
            }),
        },
    }
}

fn route_should_compress(route: &RouteResult, input: &str) -> bool {
    !is_local_provider(&route.provider)
        && input.len() > ROUTE_COMPRESSION_BYTE_THRESHOLD
        && route_intent_allows_compression(&route.intent_key)
}

fn route_intent_allows_compression(intent_key: &str) -> bool {
    matches!(intent_key, "chat" | "build" | "research" | "strategy")
}

fn compression_trace_suffix(compression: Option<&RouteCompressionMetadata>) -> String {
    let Some(compression) = compression else {
        return String::new();
    };
    if compression.applied {
        let tokens_saved = compression.input_tokens as i64 - compression.output_tokens as i64;
        format!(
            " compression=applied ratio={:.3} chars={}->{} tokens={}->{} saved={} usd_saved={:.6} receipt={}",
            compression.ratio,
            compression.input_chars,
            compression.output_chars,
            compression.input_tokens,
            compression.output_tokens,
            tokens_saved,
            compression.estimated_usd_saved,
            compression.receipt_path.as_deref().unwrap_or("none")
        )
    } else {
        format!(" compression=skipped reason={}", compression.reason)
    }
}

fn working_context_prompt(pins: &SessionPins) -> String {
    let dirs = pins
        .scope
        .allowed_dirs
        .iter()
        .map(|path| format!("  - {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let tools = pins
        .scope
        .tool_leases
        .iter()
        .filter(|lease| lease.allowed)
        .map(|lease| format!("  - {} ({})", lease.name, lease.risk_class))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Heiwa working context:\nprincipal: {} ({:?}/{:?})\ncurrent directory: {}\nallowed directories:\n{}\nsandbox: {:?}\nnetwork: {:?}\nactive tool leases:\n{}",
        pins.principal.id,
        pins.principal.kind,
        pins.principal.role,
        pins.scope.working_dir.display(),
        dirs,
        pins.scope.sandbox_mode,
        pins.scope.network_policy,
        tools
    )
}

fn transcript_block_to_message(block: &TranscriptBlock) -> Option<(Role, String)> {
    match block {
        TranscriptBlock::User(text) => Some((Role::User, text.clone())),
        TranscriptBlock::Assistant(text) => Some((Role::Assistant, text.clone())),
        TranscriptBlock::Tool(name, output) => {
            Some((Role::System, format!("Tool {} output:\n{}", name, output)))
        }
        TranscriptBlock::Evidence(text) => Some((Role::System, format!("Evidence:\n{}", text))),
    }
}

fn append_state_block(state: &mut SessionState, block: TranscriptBlock) {
    state.transcript.push(block);
}

fn append_controller_block(
    session_id: &str,
    transcript: &mut Vec<TranscriptBlock>,
    block: TranscriptBlock,
    event_tx: &tokio::sync::mpsc::UnboundedSender<CockpitEvent>,
) {
    let _ = (session_id, event_tx);
    transcript.push(block);
}

async fn execute_deterministic_surface_turn(
    thread_id: &str,
    prompt: &str,
    response: &str,
) -> Result<(), String> {
    let runner = default_model_call_runtime()?.runner;
    let request = heiwa_session::operator::StartTurnRequest::auto(
        format!("surface-{}", uuid::Uuid::new_v4()),
        prompt,
    );
    let mut handle = runner
        .submit(
            thread_id,
            request,
            OperatorTurnWork::Deterministic {
                response: response.to_string(),
                route: route_event_payload("deterministic", None),
                done: repl_trace_payload("deterministic", None, None, None, None),
            },
        )
        .map_err(|error| error.to_string())?;
    while let Ok(frame) = handle.recv().await {
        if frame.is_terminal()
            && matches!(
                frame,
                OperatorStreamFrame::Durable(ref row)
                    if row.event.turn_id.as_deref() == Some(handle.turn_id.as_str())
            )
        {
            return Ok(());
        }
        if let OperatorStreamFrame::Error {
            turn_id, message, ..
        } = frame
        {
            if turn_id == handle.turn_id {
                return Err(message);
            }
        }
    }
    Err("operator turn ended without a durable terminal event".to_string())
}

// ---------------------------------------------------------------------------
// Shared execution core — used by both plain REPL and cockpit controller
// ---------------------------------------------------------------------------

/// Returns true if the provider has a working adapter.
fn has_adapter(provider: &str) -> bool {
    provider_supports_loop_adapter(provider)
}

/// Route a task through DREX, returning the adapter + metadata needed to stream.
fn route_task(
    task: &str,
    pins: &SessionPins,
    model_tiers: &[heiwa_protocol::ModelTier],
) -> Result<RouteOutcome, String> {
    route_task_inner(task, pins, model_tiers, None, Utc::now().timestamp(), true)
}

fn route_task_with_quota(
    task: &str,
    pins: &SessionPins,
    model_tiers: &[heiwa_protocol::ModelTier],
    quota_ledger: Option<&heiwa_quota::QuotaLedger>,
    now_unix: i64,
) -> Result<RouteOutcome, String> {
    route_task_inner(task, pins, model_tiers, quota_ledger, now_unix, false)
}

fn route_task_inner(
    task: &str,
    pins: &SessionPins,
    model_tiers: &[heiwa_protocol::ModelTier],
    quota_ledger: Option<&heiwa_quota::QuotaLedger>,
    now_unix: i64,
    use_default_quota_ledger: bool,
) -> Result<RouteOutcome, String> {
    let turn_request = parse_turn_intent(task);
    let (provider_pin, model_pin) = match (turn_request.provider_pin, turn_request.model_pin) {
        (Some(p), Some(m)) => (Some(p), Some(m)),
        (Some(p), None) => (Some(p), None),
        _ => (None, None),
    };

    let final_provider_pin = provider_pin.as_deref().or(pins.pinned_provider.as_deref());
    let final_model_pin = model_pin.as_deref().or(pins.pinned_model.as_deref());

    let privacy = privacy_for_task(task);
    let ingress = DrexIngress {
        intent: turn_request.intent.as_drex_key().to_string(),
        risk: "low".to_string(),
        raw_text: task.to_string(),
        privacy: privacy.to_string(),
        runtime: runtime_for_route_preference(pins.route_preference).to_string(),
        available_vram_mb: 8192,
        required_context_tokens: 1024,
    };
    let policy = default_policy();

    let early_preflight = preflight_execution(&ingress, &[], &policy);
    match early_preflight.execution_mode {
        ExecutionMode::Deterministic | ExecutionMode::Clarify => {
            let response = early_preflight.response_text.unwrap_or_default();
            return Ok(RouteOutcome::Deterministic(response));
        }
        _ => {}
    }

    // Filter to providers with working adapters before DREX ever sees them.
    let adapter_capable: Vec<heiwa_protocol::ModelTier> = model_tiers
        .iter()
        .filter(|t| has_adapter(&t.provider))
        .cloned()
        .collect();

    if adapter_capable.is_empty() {
        // Zero routable providers is a state the user can act on, not an
        // internal failure: say which of their accounts were skipped and
        // why, or how to connect a first one.
        let guidance = heiwa_provider::health::FleetHealth::load().guidance();
        return Err(if guidance.is_empty() {
            format!(
                "No models with working adapters. Supported providers: {}.",
                heiwa_provider::routing::SUPPORTED_PROVIDERS.join(", "),
            )
        } else {
            guidance
        });
    }

    // Auxiliary calls have their own DREX policy. Preserve on-device
    // inventory before main-turn pins or local/remote preference narrow the
    // execution candidates, so a remote-only main call can still use a
    // sovereign zero-cost compression call.
    let local_auxiliary_candidates = adapter_capable
        .iter()
        .map(model_call_candidate)
        .filter(|candidate| candidate.locality == ExecutionLocality::OnDevice)
        .collect();

    let routed_tiers = filtered_model_tiers(
        &adapter_capable,
        pins.route_preference,
        final_provider_pin,
        final_model_pin,
    );

    if routed_tiers.is_empty() {
        let reason = if let Some(model) = final_model_pin {
            format!("Model '{model}' not available.")
        } else if let Some(provider) = final_provider_pin {
            let supported: Vec<&str> = adapter_capable
                .iter()
                .map(|t| t.provider.as_str())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            format!(
                "Provider '{provider}' not available. Supported: {}.",
                supported.join(", "),
            )
        } else {
            "No models available.".to_string()
        };
        return Err(format!("Routing failed: {}", reason));
    }

    let default_quota_ledger = if quota_ledger.is_none() && use_default_quota_ledger {
        open_default_quota_ledger()
    } else {
        None
    };
    let active_quota_ledger = quota_ledger.or(default_quota_ledger.as_ref());
    let quota_admission = quota_admitted_model_tiers(&routed_tiers, active_quota_ledger, now_unix);
    if quota_admission.admitted.is_empty() {
        let groups = if quota_admission.exhausted_groups.is_empty() {
            "none".to_string()
        } else {
            quota_admission.exhausted_groups.join(", ")
        };
        return Err(format!(
            "Routing failed: quota exhausted for candidate rate groups: {}.",
            groups
        ));
    }

    let preflight = preflight_execution(&ingress, &quota_admission.admitted, &policy);

    match preflight.execution_mode {
        ExecutionMode::Deterministic | ExecutionMode::Clarify => {
            let response = preflight.response_text.unwrap_or_default();
            return Ok(RouteOutcome::Deterministic(response));
        }
        _ => {}
    }

    // Preserve the complete admitted inventory for per-call DREX planning.
    // `routed_tiers` already enforced explicit provider/model pins and
    // local-only/remote-only session policy. The legacy Auto preflight is a
    // useful initial-route hint, but narrowing to that lane here would make
    // an opposite-lane model invisible to a later call's quality floor,
    // budget, allow/exclude set, or privacy policy.
    let effective_tiers = quota_admission.admitted;

    let route = plan_route(&ingress, &effective_tiers, &policy)
        .map_err(|e| format!("Routing failed: {}", e))?;

    let selected = route
        .selected_model
        .as_ref()
        .ok_or_else(|| "No model matched for this task.".to_string())?;

    Ok(RouteOutcome::Routed(Box::new(RouteResult {
        candidates: effective_tiers.iter().map(model_call_candidate).collect(),
        local_auxiliary_candidates,
        model_id: selected.model_id.clone(),
        provider: selected.provider.clone(),
        provider_model_id: selected.provider_model_id.clone(),
        rate_group: selected.rate_group.clone(),
        routing_metadata: route.routing_metadata,
        intent_key: turn_request.intent.as_drex_key().to_string(),
        privacy: privacy.to_string(),
        request_id: uuid::Uuid::new_v4().to_string(),
        turn_started_at: Utc::now().to_rfc3339(),
    })))
}

/// Detect privacy cues that force the sovereign (local-only) lane.
///
/// Hard rule: sovereign work stays local-first. A false positive only costs
/// remote quality on a task the operator framed as private; a false negative
/// leaks framing the operator marked sensitive — so match generously.
pub(crate) fn privacy_for_task(task: &str) -> &'static str {
    let lower = task.to_lowercase();
    const SOVEREIGN_HINTS: [&str; 6] = [
        "privat", // private, privately, privacy
        "confidential",
        "sensitive",
        "sovereign",
        "personal",
        "do not share",
    ];
    if SOVEREIGN_HINTS.iter().any(|hint| lower.contains(hint)) {
        "sovereign"
    } else {
        "standard"
    }
}

/// Resolve a provider adapter by name.
///
/// Selection itself lives in `heiwa_provider::routing` so every surface —
/// this CLI, the desktop runtime, and the fresh-install harness — resolves a
/// provider the same way.
fn resolve_adapter(provider: &str, model_id: &str) -> Result<Arc<dyn ProviderAdapter>, String> {
    heiwa_provider::routing::resolve_adapter(provider, model_id)
}

fn model_call_candidate(tier: &heiwa_protocol::ModelTier) -> ModelCallCandidate {
    let on_device = matches!(tier.provider.as_str(), "ollama" | "local")
        && matches!(tier.rate_group.as_str(), "local" | "local_ollama");
    let marginal_cost_usd = if tier.cost_per_turn == 0.0 && !on_device {
        None
    } else {
        Some(tier.cost_per_turn)
    };
    ModelCallCandidate {
        tier: tier.clone(),
        locality: if on_device {
            ExecutionLocality::OnDevice
        } else {
            ExecutionLocality::Unverified
        },
        connected: true,
        adapter_capable: true,
        quota_available: true,
        marginal_cost_usd,
        cost_truth: if tier.cost_per_turn == 0.0 {
            if on_device {
                CostTruth::LocalZeroCost
            } else {
                CostTruth::CannotConfirm
            }
        } else {
            CostTruth::ProxyEstimate
        },
    }
}

#[derive(Clone)]
struct DefaultModelCallRuntime {
    executor: Arc<ModelCallExecutor>,
    sessions: Arc<heiwa_session::operator::OperatorSessionService>,
    runner: Arc<OperatorTurnRunner>,
}

fn default_model_call_runtime() -> Result<DefaultModelCallRuntime, String> {
    static RUNTIME: OnceLock<Result<DefaultModelCallRuntime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            let sessions = Arc::new(heiwa_session::operator::OperatorSessionService::new(
                heiwa_evidence::OperatorJournal::new(
                    heiwa_evidence::journal_root().map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?,
            ));
            let resolver =
                Arc::new(|provider: &str, model: &str| resolve_adapter(provider, model).ok());
            let executor = Arc::new(ModelCallExecutor::new(resolver, sessions.clone()));
            let runner = Arc::new(OperatorTurnRunner::new(sessions.clone(), executor.clone()));
            Ok(DefaultModelCallRuntime {
                executor,
                sessions,
                runner,
            })
        })
        .clone()
}

fn default_loop_model_caller() -> Result<Arc<dyn heiwa_loop::LoopModelCaller>, String> {
    let runtime = default_model_call_runtime()?;
    Ok(Arc::new(ExecutorLoopCaller::new(runtime.executor)))
}

async fn execute_model_call(execution: ModelCallExecution) -> Result<ModelCallResult, String> {
    let runtime = default_model_call_runtime()?;
    runtime
        .executor
        .execute(execution)
        .await
        .map_err(|error| error.to_string())
}

async fn execute_canonical_model_turn(
    execution: ModelCallExecution,
) -> Result<ModelCallResult, String> {
    let runtime = default_model_call_runtime()?;
    runtime
        .executor
        .execute_canonical_turn(execution)
        .await
        .map_err(|error| error.to_string())
}

async fn execute_routed_model_call(
    route: &RouteResult,
    messages: Vec<Message>,
    thread_id: &str,
    raw_text: &str,
    delta_tx: Option<tokio::sync::mpsc::Sender<heiwa_provider::adapter::StreamEvent>>,
) -> Result<ModelCallResult, String> {
    let sessions = default_model_call_runtime()?.sessions;
    let call_id = format!("call-{}", uuid::Uuid::new_v4());
    let turn = sessions
        .start_turn(
            thread_id,
            heiwa_session::operator::StartTurnRequest::auto(call_id.clone(), raw_text),
        )
        .map_err(|error| error.to_string())?;
    let privacy = PrivacyClass::parse(&route.privacy).map_err(str::to_string)?;
    let (_cancel_tx, cancel) = tokio::sync::watch::channel(false);
    execute_canonical_model_turn(ModelCallExecution {
        request: ModelCallRequest {
            thread_id: thread_id.to_string(),
            turn_id: turn.turn_id,
            work_id: None,
            call_id,
            intent: route.intent_key.clone(),
            stage: ModelCallStage::Execution,
            raw_text: raw_text.to_string(),
            privacy,
            risk: CallRisk::Low,
            safety: SafetyClass::low_risk_auto_approval(&CallRisk::Low),
            required_capabilities: vec![],
            required_context_tokens: 1,
            minimum_quality_class: 1,
            minimum_success_rate: 0.0,
            maximum_marginal_cost_usd: None,
            preferred_provider: None,
            preferred_model: None,
            allowed_models: vec![],
            excluded_models: vec![],
        },
        candidates: route.candidates.clone(),
        messages,
        remaining_budget_usd: None,
        max_attempts: 3,
        cancel,
        delta_tx,
    })
    .await
}

async fn execute_compression_model_call(
    context: &OperatorPreparationContext,
    candidates: Vec<ModelCallCandidate>,
    body: &str,
    allowed_models: Vec<String>,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> Result<ModelCallResult, String> {
    let (_cancel_tx, cancel) = tokio::sync::watch::channel(
        cancelled.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)),
    );
    let local_candidates = candidates
        .into_iter()
        .filter(|candidate| candidate.locality == ExecutionLocality::OnDevice)
        .collect();
    execute_model_call(ModelCallExecution {
        request: ModelCallRequest {
            thread_id: context.thread_id.clone(),
            turn_id: context.turn_id.clone(),
            work_id: None,
            call_id: format!("call-compression-{}", uuid::Uuid::new_v4()),
            intent: "compression".to_string(),
            stage: ModelCallStage::Compression,
            raw_text: format!("compress {} characters locally", body.chars().count()),
            privacy: PrivacyClass::Sovereign,
            risk: CallRisk::Low,
            safety: SafetyClass::Approved,
            required_capabilities: vec![],
            required_context_tokens: 1,
            minimum_quality_class: 1,
            minimum_success_rate: 0.0,
            maximum_marginal_cost_usd: Some(0.0),
            preferred_provider: None,
            preferred_model: None,
            allowed_models,
            excluded_models: vec![],
        },
        candidates: local_candidates,
        messages: vec![
            Message {
                role: Role::System,
                content: cmd::compress::COMPRESSION_PROMPT.to_string(),
            },
            Message {
                role: Role::User,
                content: body.to_string(),
            },
        ],
        remaining_budget_usd: Some(0.0),
        max_attempts: 3,
        cancel,
        delta_tx: None,
    })
    .await
}

async fn routed_compression_receipt(
    context: &OperatorPreparationContext,
    candidates: Vec<ModelCallCandidate>,
    body: &str,
    source: &str,
    allowed_models: Vec<String>,
    pricing: Option<cmd::compress::PricingInputs>,
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> Result<cmd::compress::CompressionReceipt, String> {
    let started = std::time::Instant::now();
    let result =
        execute_compression_model_call(context, candidates, body, allowed_models, cancelled)
            .await?;
    let compressed = cmd::compress::strip_think_blocks(&result.text)
        .trim()
        .to_string();
    let model = format!("{}/{}", result.provider, result.provider_model_id);
    cmd::compress::finish_compression_receipt(
        body,
        source,
        &model,
        pricing,
        compressed,
        started.elapsed().as_millis() as u64,
    )
    .map_err(|error| error.to_string())
}

pub(crate) async fn execute_standalone_compression(
    body: &str,
    source: &str,
    requested_model: &str,
) -> anyhow::Result<cmd::compress::CompressionReceipt> {
    let candidates = discovered_model_call_candidates().await;
    let sessions = default_model_call_runtime()
        .map_err(anyhow::Error::msg)?
        .sessions;
    let client_request_id = format!("compress-{}", uuid::Uuid::new_v4());
    let submission = sessions
        .start_turn(
            "auxiliary-compression",
            heiwa_session::operator::StartTurnRequest::auto(
                client_request_id,
                format!("compress source={source} chars={}", body.chars().count()),
            ),
        )
        .map_err(|error| anyhow!(error.to_string()))?;
    let context = OperatorPreparationContext {
        thread_id: submission.thread_id,
        turn_id: submission.turn_id,
    };
    let result = routed_compression_receipt(
        &context,
        candidates,
        body,
        source,
        vec![requested_model.to_string()],
        None,
        None,
    )
    .await;
    let model_succeeded = result.is_ok();
    let final_result = match result {
        Ok(receipt) => Ok(receipt),
        Err(_) => cmd::compress::finish_compression_receipt(
            body,
            source,
            "deterministic/no-compression",
            None,
            body.to_string(),
            0,
        ),
    };
    finish_auxiliary_turn(&sessions, &context, model_succeeded, "compression")?;
    final_result
}

fn finish_auxiliary_turn(
    sessions: &heiwa_session::operator::OperatorSessionService,
    context: &OperatorPreparationContext,
    succeeded: bool,
    stage: &str,
) -> anyhow::Result<()> {
    sessions.append_event(heiwa_evidence::OperatorEvent {
        schema_version: heiwa_evidence::OPERATOR_EVENT_SCHEMA_VERSION,
        event_id: format!("evt-{}", uuid::Uuid::new_v4()),
        thread_id: context.thread_id.clone(),
        turn_id: Some(context.turn_id.clone()),
        run_id: None,
        call_id: None,
        work_id: None,
        event_type: heiwa_evidence::OperatorEventType::TurnCompleted,
        occurred_at: heiwa_evidence::now_iso(),
        actor: heiwa_evidence::OperatorActor {
            kind: "runtime".to_string(),
            id: "model-call-executor".to_string(),
        },
        risk_class: heiwa_evidence::OperatorRisk::Low,
        sensitivity: heiwa_evidence::OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: vec![],
        evidence_refs: vec![],
        payload: serde_json::json!({
            "stage": stage,
            "outcome": if succeeded { "completed" } else { "deterministic_fallback" },
        }),
    })?;
    Ok(())
}

pub(crate) async fn execute_mail_draft_model_call(
    candidates: Vec<ModelCallCandidate>,
    prompt: &str,
) -> Result<ModelCallResult, String> {
    let sessions = default_model_call_runtime()?.sessions;
    let submission = sessions
        .start_turn(
            "auxiliary-mail-drafting",
            heiwa_session::operator::StartTurnRequest::auto(
                format!("mail-draft-{}", uuid::Uuid::new_v4()),
                "metadata-only local mail draft",
            ),
        )
        .map_err(|error| error.to_string())?;
    let context = OperatorPreparationContext {
        thread_id: submission.thread_id,
        turn_id: submission.turn_id,
    };
    let (_cancel_tx, cancel) = tokio::sync::watch::channel(false);
    let result = execute_model_call(ModelCallExecution {
        request: ModelCallRequest {
            thread_id: context.thread_id.clone(),
            turn_id: context.turn_id.clone(),
            work_id: None,
            call_id: format!("call-drafting-{}", uuid::Uuid::new_v4()),
            intent: "mail_drafting".to_string(),
            stage: ModelCallStage::Drafting,
            raw_text: "draft from mail metadata locally".to_string(),
            privacy: PrivacyClass::Sovereign,
            risk: CallRisk::Low,
            safety: SafetyClass::Approved,
            required_capabilities: vec![],
            required_context_tokens: 1,
            minimum_quality_class: 1,
            minimum_success_rate: 0.0,
            maximum_marginal_cost_usd: Some(0.0),
            preferred_provider: None,
            preferred_model: None,
            allowed_models: vec![],
            excluded_models: vec![],
        },
        candidates: candidates
            .into_iter()
            .filter(|candidate| candidate.locality == ExecutionLocality::OnDevice)
            .collect(),
        messages: vec![Message {
            role: Role::User,
            content: prompt.to_string(),
        }],
        remaining_budget_usd: Some(0.0),
        max_attempts: 3,
        cancel,
        delta_tx: None,
    })
    .await;
    finish_auxiliary_turn(&sessions, &context, result.is_ok(), "drafting")
        .map_err(|error| error.to_string())?;
    result
}

pub(crate) async fn discovered_model_call_candidates() -> Vec<ModelCallCandidate> {
    let mut registry = heiwa_provider::AccountRegistry::load();
    heiwa_provider::detect::auto_discover(&mut registry).await;
    get_live_model_tiers(&registry)
        .iter()
        .map(model_call_candidate)
        .collect()
}

/// Record a DREX route decision in the local evidence journal.
fn record_route_evidence(evidence: &EvidenceClient, route: &RouteResult, task: &str) {
    evidence.journal(
        "route_decisions",
        serde_json::json!({
            "request_id": route.request_id,
            "task": task,
            "intent": route.intent_key,
            "risk": "low",
            "privacy": route.privacy,
            "provider": route.provider,
            "model_id": route.model_id,
            "locality": if is_local_provider(&route.provider) { "local" } else { "remote" },
            "routing_metadata": route.routing_metadata,
            "confidence": 0.9,
        }),
    );
}

/// Record a completed run in the local receipt store.
struct CallReceiptInput<'a> {
    result: &'a ModelCallResult,
    usage: Option<&'a TokenUsage>,
    session_id: &'a str,
    input_text: &'a str,
    output_text: &'a str,
    latency_ms: i64,
}

fn record_call_receipt(
    receipts: &heiwa_receipts::ReceiptStore,
    rates: &heiwa_receipts::RateTable,
    input: CallReceiptInput<'_>,
) {
    use heiwa_receipts::{runtime, CallReceipt};

    let env = runtime::env_for_provider(&input.result.provider);
    let tokens_in = input
        .usage
        .map(|u| u.input_tokens as i64)
        .filter(|&n| n > 0)
        .unwrap_or_else(|| runtime::estimate_tokens(input.input_text));
    let tokens_out = input
        .usage
        .map(|u| u.output_tokens as i64)
        .filter(|&n| n > 0)
        .unwrap_or_else(|| runtime::estimate_tokens(input.output_text));

    let (costs, _found) = runtime::compute_or_zero(
        rates,
        env,
        &input.result.provider,
        &input.result.model_id,
        tokens_in,
        tokens_out,
    );

    let mut receipt = CallReceipt::new(
        Utc::now().timestamp(),
        env,
        input.result.provider.clone(),
        input.result.model_id.clone(),
        "repl",
        tokens_in,
        tokens_out,
        input.latency_ms,
        costs.actual_cad,
        costs.counterfactual_cad,
        input.session_id,
        None,
    );
    receipt.model_call_cost_usd = Some(input.result.cost_usd);
    receipt.model_call_cost_truth = Some(cost_truth_label(&input.result.cost_truth).to_string());
    receipt.model_call_attempts = Some(input.result.attempts.min(i64::MAX as usize) as i64);
    receipt.failed_attempt_cost_usd = Some(
        input
            .result
            .attempt_records
            .iter()
            .filter(|attempt| {
                attempt.provider_invoked
                    && attempt.outcome == heiwa_shell::model_calls::ModelCallAttemptOutcome::Failed
            })
            .filter_map(|attempt| attempt.cost_usd)
            .fold(0.0, |total, cost| {
                let next = total + cost;
                if next.is_finite() {
                    next
                } else {
                    f64::MAX
                }
            }),
    );

    if let Err(error) = receipts.insert(&receipt) {
        debug_log(format_args!("receipt insert failed: {error}"));
    }
}

fn cost_truth_label(truth: &CostTruth) -> &'static str {
    match truth {
        CostTruth::LocalZeroCost => "local_zero_cost",
        CostTruth::TargetOnly => "target_only",
        CostTruth::ProxyEstimate => "proxy_estimate",
        CostTruth::ExactProviderReport => "exact_provider_report",
        CostTruth::CannotConfirm => "cannot_confirm",
    }
}

/// Journal a completed run to local evidence.
fn record_run_evidence(
    evidence: &EvidenceClient,
    route: &RouteResult,
    usage: Option<&TokenUsage>,
    result: Option<&ModelCallResult>,
) {
    let run_id = format!("run-{}", uuid::Uuid::new_v4());
    let turn_ended_at = Utc::now();
    let turn_ended_at_rfc3339 = turn_ended_at.to_rfc3339();
    let user_id = heiwa_provider::load_identity()
        .map(|id| id.user_id)
        .unwrap_or_else(|| "anonymous".to_string());

    evidence.journal(
        "runs",
        serde_json::json!({
            "run_id": run_id,
            "user_id": user_id,
            "request_id": route.request_id,
            "started_at": route.turn_started_at,
            "ended_at": turn_ended_at_rfc3339,
            "status": if usage.is_some() { "SUCCESS" } else { "COMPLETED_NO_USAGE" },
            "model_id": route.model_id,
            "tokens_input": usage.map(|u| u.input_tokens as i64).unwrap_or(0),
            "tokens_output": usage.map(|u| u.output_tokens as i64).unwrap_or(0),
            "cost_usd": result.map(|call| call.cost_usd)
                .or_else(|| usage.map(|u| u.cost_usd))
                .unwrap_or(0.0),
            "cost_truth": result.map(|call| &call.cost_truth),
            "attempts": result.map(|call| call.attempts),
            "attempt_records": result.map(|call| &call.attempt_records),
        }),
    );

    if let Some(ledger) = open_default_quota_ledger() {
        if let Err(error) = record_local_quota_run(
            &ledger,
            &run_id,
            route,
            usage,
            result.map(|call| call.cost_usd),
            turn_ended_at.timestamp(),
        ) {
            debug_log(format_args!("quota ledger write failed: {error}"));
        }
    }
}

const QUOTA_ADMISSION_WINDOW_SECONDS: i64 = 86_400;
const REMOTE_RATE_GROUP_TOKEN_LIMIT: i64 = 200_000;
const LOCAL_QUOTA_WINDOW_SECONDS: i64 = QUOTA_ADMISSION_WINDOW_SECONDS;

fn record_local_quota_run(
    ledger: &heiwa_quota::QuotaLedger,
    run_id: &str,
    route: &RouteResult,
    usage: Option<&TokenUsage>,
    model_call_cost_usd: Option<f64>,
    ended_at_unix: i64,
) -> heiwa_quota::Result<()> {
    let (tokens_input, tokens_output, cost, status) = match usage {
        Some(u) => (
            u.input_tokens as i64,
            u.output_tokens as i64,
            model_call_cost_usd.unwrap_or(u.cost_usd),
            "SUCCESS",
        ),
        None => (0, 0, 0.0, "COMPLETED_NO_USAGE"),
    };
    let started_at_unix = chrono::DateTime::parse_from_rfc3339(&route.turn_started_at)
        .map(|dt| dt.timestamp())
        .unwrap_or(ended_at_unix);
    let tokens = tokens_input.saturating_add(tokens_output);

    ledger.record_use(
        &route.provider,
        &route.rate_group,
        LOCAL_QUOTA_WINDOW_SECONDS,
        tokens,
        1,
        ended_at_unix,
    )?;
    ledger.record_run(&heiwa_quota::RunRecord {
        id: run_id.to_string(),
        provider: route.provider.clone(),
        model_id: route.model_id.clone(),
        started_at_unix,
        ended_at_unix,
        tokens_input,
        tokens_output,
        cost,
        status: status.to_string(),
        meta: serde_json::json!({
            "request_id": route.request_id,
            "intent": route.intent_key,
            "provider_model_id": route.provider_model_id,
            "rate_group": route.rate_group,
            "routing_metadata": route.routing_metadata,
        }),
    })?;
    Ok(())
}

#[derive(Debug)]
struct QuotaAdmission {
    admitted: Vec<heiwa_protocol::ModelTier>,
    exhausted_groups: Vec<String>,
}

fn quota_admitted_model_tiers(
    model_tiers: &[heiwa_protocol::ModelTier],
    ledger: Option<&heiwa_quota::QuotaLedger>,
    now_unix: i64,
) -> QuotaAdmission {
    let mut admitted = Vec::new();
    let mut exhausted_groups = Vec::new();
    let mut seen_exhausted = std::collections::HashSet::new();

    for tier in model_tiers {
        let Some(token_limit) = quota_token_limit_for_tier(tier) else {
            admitted.push(tier.clone());
            continue;
        };
        let Some(ledger) = ledger else {
            let label = format!("{} (ledger unavailable)", quota_group_label(tier));
            if seen_exhausted.insert(label.clone()) {
                exhausted_groups.push(label);
            }
            continue;
        };

        match ledger.remaining_budget(
            &tier.provider,
            &tier.rate_group,
            QUOTA_ADMISSION_WINDOW_SECONDS,
            token_limit,
            now_unix,
        ) {
            Ok(budget) if budget.exhausted => {
                let label = quota_group_label(tier);
                if seen_exhausted.insert(label.clone()) {
                    exhausted_groups.push(label);
                }
            }
            Ok(_) => admitted.push(tier.clone()),
            Err(error) => {
                let label = format!("{} (ledger error)", quota_group_label(tier));
                debug_log(format_args!(
                    "quota admission read failed for {}: {}",
                    quota_group_label(tier),
                    error
                ));
                if seen_exhausted.insert(label.clone()) {
                    exhausted_groups.push(label);
                }
            }
        }
    }

    QuotaAdmission {
        admitted,
        exhausted_groups,
    }
}

fn quota_budget_preview_lines(
    model_tiers: &[heiwa_protocol::ModelTier],
    ledger: Option<&heiwa_quota::QuotaLedger>,
    now_unix: i64,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for tier in model_tiers
        .iter()
        .filter(|tier| has_adapter(&tier.provider))
    {
        let label = quota_group_label(tier);
        if !seen.insert(label.clone()) {
            continue;
        }

        let Some(token_limit) = quota_token_limit_for_tier(tier) else {
            lines.push(format!("{label}: unmetered"));
            continue;
        };
        let Some(ledger) = ledger else {
            lines.push(format!("{label}: ledger unavailable"));
            continue;
        };

        match ledger.remaining_budget(
            &tier.provider,
            &tier.rate_group,
            QUOTA_ADMISSION_WINDOW_SECONDS,
            token_limit,
            now_unix,
        ) {
            Ok(budget) => lines.push(format!(
                "{}: {}/{} tokens remaining, resets {}",
                label,
                budget.tokens_remaining,
                budget.token_limit,
                format_unix_timestamp(budget.window_resets_at_unix)
            )),
            Err(error) => lines.push(format!("{label}: ledger error ({error})")),
        }
    }

    lines
}

fn quota_token_limit_for_tier(tier: &heiwa_protocol::ModelTier) -> Option<i64> {
    if is_local_provider(&tier.provider) || tier.rate_group == "local" {
        return None;
    }

    env::var("HEIWA_REMOTE_RATE_GROUP_TOKEN_LIMIT")
        .ok()
        .and_then(|raw| raw.parse::<i64>().ok())
        .filter(|limit| *limit > 0)
        .or(Some(REMOTE_RATE_GROUP_TOKEN_LIMIT))
}

fn quota_group_label(tier: &heiwa_protocol::ModelTier) -> String {
    format!("{}/{}", tier.provider, tier.rate_group)
}

fn open_default_quota_ledger() -> Option<heiwa_quota::QuotaLedger> {
    match heiwa_quota::QuotaLedger::open(heiwa_quota::QuotaLedger::default_path()) {
        Ok(ledger) => Some(ledger),
        Err(error) => {
            debug_log(format_args!("quota ledger open failed: {error}"));
            None
        }
    }
}

fn format_unix_timestamp(timestamp: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| timestamp.to_string())
}

fn debug_log(args: std::fmt::Arguments<'_>) {
    if env::var_os("HEIWA_DEBUG").is_some() {
        eprintln!("debug: {args}");
    }
}

fn record_tool_call_evidence(
    evidence: &EvidenceClient,
    receipt: &ToolCallReceipt,
    session_id: &str,
) {
    let user_id = heiwa_provider::load_identity()
        .map(|id| id.user_id)
        .unwrap_or_else(|| "anonymous".to_string());
    evidence.journal(
        "tool_calls",
        serde_json::json!({
            "receipt_id": receipt.id,
            "user_id": user_id,
            "call_id": receipt.call_id,
            "session_id": session_id,
            "tool_name": receipt.tool_name,
            "status": receipt.status.as_str(),
            "started_at": receipt.started_at,
            "completed_at": receipt.completed_at,
            "receipt": receipt,
            "error": receipt.error,
        }),
    );
}

async fn collect_adapter_response(
    route: &RouteResult,
    messages: Vec<Message>,
    thread_id: &str,
    raw_text: &str,
) -> Result<ModelCallResult, String> {
    execute_routed_model_call(route, messages, thread_id, raw_text, None).await
}

fn merge_usage(first: Option<TokenUsage>, second: Option<TokenUsage>) -> Option<TokenUsage> {
    match (first, second) {
        (None, None) => None,
        (Some(usage), None) | (None, Some(usage)) => Some(usage),
        (Some(a), Some(b)) => Some(TokenUsage {
            input_tokens: a.input_tokens + b.input_tokens,
            output_tokens: a.output_tokens + b.output_tokens,
            cache_read_tokens: a.cache_read_tokens + b.cache_read_tokens,
            cache_write_tokens: a.cache_write_tokens + b.cache_write_tokens,
            cost_usd: a.cost_usd + b.cost_usd,
        }),
    }
}

fn usage_for_model_call(result: &ModelCallResult) -> TokenUsage {
    let mut usage = result.usage.clone();
    usage.cost_usd = result.cost_usd;
    usage
}

fn aggregate_model_call_results(
    first: &ModelCallResult,
    final_result: &ModelCallResult,
) -> ModelCallResult {
    let raw_cost = first.cost_usd + final_result.cost_usd;
    let (cost_usd, cost_truth) = if raw_cost.is_finite() {
        (
            raw_cost,
            aggregate_cost_truth(first.cost_truth.clone(), final_result.cost_truth.clone()),
        )
    } else {
        (f64::MAX, CostTruth::CannotConfirm)
    };
    let mut failed_models = first.failed_models.clone();
    for model in &final_result.failed_models {
        if !failed_models.contains(model) {
            failed_models.push(model.clone());
        }
    }
    let mut attempt_records = first.attempt_records.clone();
    attempt_records.extend(final_result.attempt_records.clone());
    let mut usage = merge_usage(Some(first.usage.clone()), Some(final_result.usage.clone()))
        .unwrap_or_default();
    usage.cost_usd = cost_usd;
    ModelCallResult {
        route_receipt_ref: final_result.route_receipt_ref.clone(),
        provider: final_result.provider.clone(),
        model_id: final_result.model_id.clone(),
        provider_model_id: final_result.provider_model_id.clone(),
        rate_group: final_result.rate_group.clone(),
        text: final_result.text.clone(),
        usage,
        attempts: first.attempts.saturating_add(final_result.attempts),
        failed_models,
        cost_usd,
        cost_truth,
        attempt_records,
    }
}

fn aggregate_cost_truth(left: CostTruth, right: CostTruth) -> CostTruth {
    if left == right {
        return left;
    }
    match (left, right) {
        (CostTruth::CannotConfirm, _) | (_, CostTruth::CannotConfirm) => CostTruth::CannotConfirm,
        (CostTruth::LocalZeroCost, other) | (other, CostTruth::LocalZeroCost) => other,
        _ => CostTruth::ProxyEstimate,
    }
}

fn send_done_event(
    event_tx: &tokio::sync::mpsc::UnboundedSender<CockpitEvent>,
    usage: Option<&TokenUsage>,
) {
    let usage = usage.cloned().unwrap_or_default();
    let _ = event_tx.send(CockpitEvent::StreamDone {
        tokens_in: usage.input_tokens as i64,
        tokens_out: usage.output_tokens as i64,
        cost: usage.cost_usd,
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn filtered_model_tiers(
    model_tiers: &[heiwa_protocol::ModelTier],
    route_preference: RoutePreference,
    pinned_provider: Option<&str>,
    pinned_model: Option<&str>,
) -> Vec<heiwa_protocol::ModelTier> {
    model_tiers
        .iter()
        .filter(|tier| match route_preference {
            RoutePreference::Auto => true,
            RoutePreference::LocalOnly => is_local_provider(&tier.provider),
            RoutePreference::RemoteOnly => !is_local_provider(&tier.provider),
        })
        .filter(|tier| {
            pinned_provider
                .map(|provider| tier.provider == provider)
                .unwrap_or(true)
        })
        .filter(|tier| {
            pinned_model
                .map(|model| tier.model_id == model)
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

fn available_providers(model_tiers: &[heiwa_protocol::ModelTier]) -> Vec<String> {
    let mut providers = Vec::new();
    for tier in model_tiers {
        if !providers.contains(&tier.provider) {
            providers.push(tier.provider.clone());
        }
    }
    providers
}

fn current_route_label(
    route_preference: RoutePreference,
    pinned_provider: Option<&str>,
    pinned_model: Option<&str>,
) -> String {
    if let Some(model) = pinned_model {
        format!("model:{}", model)
    } else if let Some(provider) = pinned_provider {
        format!("provider:{}", provider)
    } else if route_preference != RoutePreference::Auto {
        format!("route:{}", route_preference_label(route_preference))
    } else {
        "direct".to_string()
    }
}

fn route_preference_label(route_preference: RoutePreference) -> &'static str {
    match route_preference {
        RoutePreference::Auto => "auto",
        RoutePreference::LocalOnly => "local",
        RoutePreference::RemoteOnly => "remote",
    }
}

fn runtime_for_route_preference(route_preference: RoutePreference) -> &'static str {
    match route_preference {
        RoutePreference::LocalOnly => "local",
        RoutePreference::RemoteOnly | RoutePreference::Auto => "any",
    }
}

fn is_local_provider(provider: &str) -> bool {
    matches!(provider, "ollama" | "local" | "vllm" | "litellm")
}

/// Events emitted by the streaming REPL pipeline, consumed by the SSE
/// endpoint and collected by the blocking endpoint.
pub(crate) enum ReplStreamEvent {
    /// Route decision metadata, sent before any tokens.
    Route(serde_json::Value),
    /// One incremental model token.
    Token(String),
    /// Terminal event with the structured trace.
    Done(serde_json::Value),
    /// Terminal failure.
    Error(String),
}

fn route_event_payload(mode: &str, route: Option<&RouteResult>) -> serde_json::Value {
    match route {
        Some(route) => serde_json::json!({
            "mode": mode,
            "intent": route.intent_key,
            "provider": route.provider,
            "model": route.model_id,
            "provider_model": route.provider_model_id,
            "rate_group": route.rate_group,
            "privacy": route.privacy,
            "request_id": route.request_id,
        }),
        None => serde_json::json!({ "mode": mode }),
    }
}

fn repl_trace_payload(
    mode: &str,
    route: Option<&RouteResult>,
    usage: Option<&TokenUsage>,
    result: Option<&ModelCallResult>,
    compression: Option<&RouteCompressionMetadata>,
) -> serde_json::Value {
    let cost_usd = result
        .map(|call| call.cost_usd)
        .or_else(|| usage.map(|u| u.cost_usd))
        .unwrap_or(0.0);
    let (intent, provider, model, rate_group, privacy) = match route {
        Some(route) => (
            route.intent_key.as_str(),
            route.provider.as_str(),
            route.model_id.as_str(),
            route.rate_group.as_str(),
            route.privacy.as_str(),
        ),
        None => ("chat", "heiwa", "deterministic", "local", "standard"),
    };
    serde_json::json!({
        "intent": intent,
        "mode": mode,
        "provider": provider,
        "model": model,
        "rate_group": rate_group,
        "privacy": privacy,
        "cost_usd": cost_usd,
        "cost_truth": result.map(|call| &call.cost_truth),
        "attempts": result.map(|call| call.attempts),
        "failed_models": result.map(|call| &call.failed_models),
        "compression": compression.map(|c| serde_json::json!({
            "applied": c.applied,
            "reason": c.reason,
            "ratio": c.ratio,
            "estimated_usd_saved": c.estimated_usd_saved,
        })),
        "summary": format!(
            "intent={intent} route={provider}/{model} cost=${cost_usd:.4}{}",
            compression_trace_suffix(compression)
        ),
    })
}

async fn run_cockpit_operator_turn(
    thread_id: &str,
    prompt: &str,
    tool_scope: Option<ExecutionScope>,
    pins: &mut SessionPins,
    transcript: &mut Vec<TranscriptBlock>,
    event_tx: &tokio::sync::mpsc::UnboundedSender<CockpitEvent>,
) -> Result<(), String> {
    let mut request = heiwa_session::operator::StartTurnRequest::auto(
        format!("cockpit-{}", uuid::Uuid::new_v4()),
        prompt,
    );
    request.route_policy.preferred_provider = pins.pinned_provider.clone();
    request.route_policy.preferred_model = pins.pinned_model.clone();
    request.route_policy.mode = match pins.route_preference {
        RoutePreference::Auto => heiwa_session::operator::RouteMode::Auto,
        RoutePreference::LocalOnly => heiwa_session::operator::RouteMode::LocalOnly,
        RoutePreference::RemoteOnly => heiwa_session::operator::RouteMode::RemoteOnly,
    };
    let mut handle = submit_operator_turn_with_scope(thread_id, request, tool_scope)
        .await
        .map_err(|error| error.to_string())?;
    transcript.push(TranscriptBlock::User(prompt.to_string()));

    while let Ok(frame) = handle.recv().await {
        match frame {
            OperatorStreamFrame::AssistantDelta { turn_id, text, .. }
                if turn_id == handle.turn_id =>
            {
                let _ = event_tx.send(CockpitEvent::StreamToken(text));
            }
            OperatorStreamFrame::Durable(row)
                if row.event.turn_id.as_deref() == Some(handle.turn_id.as_str()) =>
            {
                match row.event.event_type {
                    heiwa_evidence::OperatorEventType::RouteCompleted => {
                        if let Some(provider) =
                            row.event.payload.get("provider").and_then(Value::as_str)
                        {
                            pins.current_provider = provider.to_string();
                        }
                        if let Some(model) = row.event.payload.get("model").and_then(Value::as_str)
                        {
                            pins.current_model = model.to_string();
                        }
                        let _ = event_tx.send(CockpitEvent::RoutingUpdate(RoutingState {
                            current_provider: pins.current_provider.clone(),
                            current_model: pins.current_model.clone(),
                            mode: pins.cockpit_mode.label().to_string(),
                            explanation: Some(row.event.payload.to_string()),
                        }));
                    }
                    heiwa_evidence::OperatorEventType::AssistantCompleted => {
                        if let Some(text) = row.event.payload.get("text").and_then(Value::as_str) {
                            transcript.push(TranscriptBlock::Assistant(text.to_string()));
                        }
                    }
                    heiwa_evidence::OperatorEventType::TurnCompleted => {
                        send_done_event(event_tx, None);
                        return Ok(());
                    }
                    heiwa_evidence::OperatorEventType::TurnInterrupted
                    | heiwa_evidence::OperatorEventType::Blocker => {
                        return Err(row
                            .event
                            .payload
                            .get("message")
                            .or_else(|| row.event.payload.get("reason"))
                            .and_then(Value::as_str)
                            .unwrap_or("operator turn interrupted")
                            .to_string());
                    }
                    _ => {}
                }
            }
            OperatorStreamFrame::Error {
                turn_id, message, ..
            } if turn_id == handle.turn_id => return Err(message),
            _ => {}
        }
    }
    Err("cockpit operator turn ended without durable terminal event".to_string())
}

/// Prepare model work only after the runner has made intake durable.
async fn prepare_operator_turn_work(
    context: OperatorPreparationContext,
    mut start_request: heiwa_session::operator::StartTurnRequest,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    tool_scope: Option<ExecutionScope>,
) -> Result<OperatorTurnWork, String> {
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err("operator preparation cancelled before routing".to_string());
    }
    let thread_id = context.thread_id.clone();
    let prompt = start_request.prompt.clone();
    let persisted = heiwa_session::load_transcript(&thread_id)
        .unwrap_or_else(|_| heiwa_session::PersistedTranscript::empty(&thread_id));
    let transcript_blocks = persisted.blocks();
    let mut pins = SessionPins::new();
    pins.pinned_provider = start_request.route_policy.preferred_provider.clone();
    pins.pinned_model = start_request.route_policy.preferred_model.clone();
    pins.route_preference = match start_request.route_policy.mode {
        heiwa_session::operator::RouteMode::Auto | heiwa_session::operator::RouteMode::Explicit => {
            RoutePreference::Auto
        }
        heiwa_session::operator::RouteMode::LocalOnly => RoutePreference::LocalOnly,
        heiwa_session::operator::RouteMode::RemoteOnly => RoutePreference::RemoteOnly,
    };

    // Deterministic intents do not need provider discovery. Model-bearing
    // turns still discover the current provider/account inventory per call.
    let route_outcome = match route_task(&prompt, &pins, &[]) {
        Ok(RouteOutcome::Deterministic(response)) => RouteOutcome::Deterministic(response),
        _ => {
            let mut registry = heiwa_provider::AccountRegistry::load();
            heiwa_provider::detect::auto_discover(&mut registry).await;
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err("operator preparation cancelled during discovery".to_string());
            }
            let model_tiers = get_live_model_tiers(&registry);
            route_task(&prompt, &pins, &model_tiers)?
        }
    };

    let work = match route_outcome {
        RouteOutcome::Deterministic(response) => {
            let route = route_event_payload("deterministic", None);
            let done = repl_trace_payload("deterministic", None, None, None, None);
            OperatorTurnWork::Deterministic {
                response,
                route,
                done,
            }
        }
        RouteOutcome::Routed(route) => {
            let prepared =
                prepare_outbound_prompt_for_route_cancellable(&context, &route, &prompt, cancelled)
                    .await?;
            let messages =
                build_messages_from_transcript(&transcript_blocks, &prepared.model_prompt, &pins);
            if start_request.route_policy.privacy == "standard" && route.privacy != "standard" {
                start_request.route_policy.privacy = route.privacy.clone();
            }
            let privacy = PrivacyClass::parse(&route.privacy).map_err(str::to_string)?;
            let route_for_done = route.clone();
            let compression = prepared.compression.clone();
            let done_payload = Arc::new(move |result: &ModelCallResult| {
                let resolved = resolved_route_after_model_call(&route_for_done, result);
                let mode = if is_local_provider(&resolved.provider) {
                    "local_model"
                } else {
                    "remote_model"
                };
                let usage = usage_for_model_call(result);
                repl_trace_payload(
                    mode,
                    Some(&resolved),
                    Some(&usage),
                    Some(result),
                    compression.as_ref(),
                )
            });
            OperatorTurnWork::Model(Box::new(OperatorModelTurn {
                request: ModelCallRequest {
                    thread_id: String::new(),
                    turn_id: String::new(),
                    work_id: start_request.work_id.clone(),
                    call_id: format!("call-{}", uuid::Uuid::new_v4()),
                    intent: route.intent_key.clone(),
                    stage: ModelCallStage::Execution,
                    raw_text: prompt,
                    privacy,
                    risk: CallRisk::Low,
                    safety: SafetyClass::low_risk_auto_approval(&CallRisk::Low),
                    required_capabilities: vec![],
                    required_context_tokens: 1,
                    minimum_quality_class: start_request.route_policy.minimum_quality_class,
                    minimum_success_rate: 0.0,
                    maximum_marginal_cost_usd: start_request.route_policy.maximum_marginal_cost_usd,
                    preferred_provider: start_request.route_policy.preferred_provider.clone(),
                    preferred_model: start_request.route_policy.preferred_model.clone(),
                    allowed_models: start_request.route_policy.allowed_models.clone(),
                    excluded_models: start_request.route_policy.excluded_models.clone(),
                },
                candidates: route.candidates.clone(),
                messages,
                remaining_budget_usd: start_request.route_policy.turn_budget_usd,
                max_attempts: 3,
                tool_scope,
                done_payload,
            }))
        }
    };
    Ok(work)
}

/// Persist caller intake immediately, then let the process-wide runner own
/// deferred discovery, routing, compression, execution, and termination.
async fn submit_operator_turn_with_route(
    thread_id: &str,
    start_request: heiwa_session::operator::StartTurnRequest,
) -> std::result::Result<
    heiwa_shell::operator::OperatorTurnHandle,
    heiwa_shell::operator::OperatorSubmissionError,
> {
    submit_operator_turn_with_scope(thread_id, start_request, None).await
}

async fn submit_operator_turn_with_scope(
    thread_id: &str,
    start_request: heiwa_session::operator::StartTurnRequest,
    tool_scope: Option<ExecutionScope>,
) -> std::result::Result<
    heiwa_shell::operator::OperatorTurnHandle,
    heiwa_shell::operator::OperatorSubmissionError,
> {
    let runner = default_model_call_runtime()
        .map_err(|error| heiwa_shell::operator::OperatorSubmissionError::Runtime(anyhow!(error)))?
        .runner;
    let preparation_request = start_request.clone();
    let preparation =
        OperatorTurnPreparation::cancellable_with_context(move |context, cancelled| async move {
            prepare_operator_turn_work(context, preparation_request, cancelled, tool_scope)
                .await
                .map_err(anyhow::Error::msg)
        });
    runner.submit(thread_id, start_request, preparation)
}

pub(crate) async fn submit_operator_turn(
    thread_id: &str,
    request: heiwa_session::operator::StartTurnRequest,
) -> std::result::Result<
    heiwa_shell::operator::OperatorTurnHandle,
    heiwa_shell::operator::OperatorSubmissionError,
> {
    submit_operator_turn_with_route(thread_id, request).await
}

fn automation_terminal_outcome(
    event_type: &heiwa_evidence::OperatorEventType,
    payload: &Value,
    assistant_text: Option<&str>,
    automation_name: &str,
    thread_id: &str,
    turn_id: &str,
) -> Option<heiwa_automations::ExecutionOutcome> {
    match event_type {
        heiwa_evidence::OperatorEventType::TurnCompleted => {
            let text = assistant_text.unwrap_or_default();
            Some(heiwa_automations::ExecutionOutcome::Completed {
                summary: if text.is_empty() {
                    format!("automation {automation_name} completed")
                } else {
                    text.to_string()
                },
                output: Some(serde_json::json!({
                    "thread_id": thread_id,
                    "turn_id": turn_id,
                    "text": text,
                })),
            })
        }
        heiwa_evidence::OperatorEventType::ApprovalRequested => {
            let request_id = payload.get("request_id").and_then(Value::as_str)?;
            let summary = payload
                .get("message")
                .or_else(|| payload.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    payload
                        .get("tool")
                        .and_then(Value::as_str)
                        .map(|tool| format!("automation requires approval for {tool}"))
                })
                .unwrap_or_else(|| "automation requires operator confirmation".to_string());
            Some(heiwa_automations::ExecutionOutcome::AwaitingConfirmation {
                request_id: request_id.to_string(),
                summary,
            })
        }
        heiwa_evidence::OperatorEventType::Blocker => {
            let summary = payload
                .get("message")
                .or_else(|| payload.get("reason"))
                .and_then(Value::as_str)
                .unwrap_or("automation requires operator confirmation")
                .to_string();
            let request_id = payload
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or(turn_id)
                .to_string();
            Some(heiwa_automations::ExecutionOutcome::AwaitingConfirmation {
                request_id,
                summary,
            })
        }
        heiwa_evidence::OperatorEventType::TurnInterrupted => {
            let message = payload
                .get("message")
                .or_else(|| payload.get("reason"))
                .and_then(Value::as_str)
                .unwrap_or("automation turn interrupted")
                .to_string();
            Some(heiwa_automations::ExecutionOutcome::Failed { message })
        }
        _ => None,
    }
}

pub(crate) async fn execute_automation_prompt(
    automation: &heiwa_automations::Automation,
    execution: &heiwa_automations::Execution,
) -> Result<heiwa_automations::ExecutionOutcome> {
    let thread_id = format!("automation-{}", automation.id);
    let request = heiwa_session::operator::StartTurnRequest::auto(
        format!("automation-{}", execution.id),
        &automation.prompt,
    );
    let mut handle = submit_operator_turn_with_route(&thread_id, request)
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    let mut assistant_text = None;

    while let Ok(frame) = handle.recv().await {
        match frame {
            OperatorStreamFrame::Durable(row)
                if row.event.turn_id.as_deref() == Some(handle.turn_id.as_str()) =>
            {
                match row.event.event_type {
                    heiwa_evidence::OperatorEventType::AssistantCompleted => {
                        assistant_text = row
                            .event
                            .payload
                            .get("text")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    ref event_type => {
                        if let Some(outcome) = automation_terminal_outcome(
                            event_type,
                            &row.event.payload,
                            assistant_text.as_deref(),
                            &automation.name,
                            &thread_id,
                            &handle.turn_id,
                        ) {
                            return Ok(outcome);
                        }
                    }
                }
            }
            OperatorStreamFrame::Error {
                turn_id, message, ..
            } if turn_id == handle.turn_id => {
                return Ok(heiwa_automations::ExecutionOutcome::Failed { message });
            }
            _ => {}
        }
    }

    Ok(heiwa_automations::ExecutionOutcome::Failed {
        message: "automation operator turn ended without a durable terminal event".to_string(),
    })
}

/// Streaming REPL compatibility surface over the durable operator runner.
/// The runner owns operator-event persistence; this wrapper only maps its
/// route/delta/terminal frames into the established SSE response shape.
pub(crate) async fn execute_repl_turn_streaming(
    prompt: &str,
    events: tokio::sync::mpsc::Sender<ReplStreamEvent>,
) {
    let client_request_id = format!("repl-{}", uuid::Uuid::new_v4());
    let start_request = heiwa_session::operator::StartTurnRequest::auto(client_request_id, prompt);
    let mut handle = match submit_operator_turn_with_route(DEFAULT_SESSION_ID, start_request).await
    {
        Ok(handle) => handle,
        Err(error) => {
            let _ = events.send(ReplStreamEvent::Error(error.to_string())).await;
            return;
        }
    };
    while let Ok(frame) = handle.recv().await {
        match frame {
            OperatorStreamFrame::Error {
                turn_id, message, ..
            } if turn_id == handle.turn_id => {
                let _ = events.send(ReplStreamEvent::Error(message)).await;
                break;
            }
            OperatorStreamFrame::AssistantDelta { turn_id, text, .. }
                if turn_id == handle.turn_id =>
            {
                let _ = events.send(ReplStreamEvent::Token(text)).await;
            }
            OperatorStreamFrame::Durable(row)
                if row.event.turn_id.as_deref() == Some(handle.turn_id.as_str()) =>
            {
                match row.event.event_type {
                    heiwa_evidence::OperatorEventType::RoutePlanned => {
                        let payload = operator_repl_route_payload(&row.event.payload);
                        let _ = events.send(ReplStreamEvent::Route(payload)).await;
                    }
                    heiwa_evidence::OperatorEventType::TurnCompleted => {
                        let done = row
                            .event
                            .payload
                            .get("trace")
                            .cloned()
                            .unwrap_or_else(|| row.event.payload.clone());
                        let _ = events.send(ReplStreamEvent::Done(done)).await;
                        break;
                    }
                    heiwa_evidence::OperatorEventType::TurnInterrupted
                    | heiwa_evidence::OperatorEventType::Blocker => {
                        let message = row
                            .event
                            .payload
                            .get("message")
                            .or_else(|| row.event.payload.get("reason"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("operator turn interrupted")
                            .to_string();
                        let _ = events.send(ReplStreamEvent::Error(message)).await;
                        break;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn operator_repl_route_payload(payload: &serde_json::Value) -> serde_json::Value {
    let provider = payload.get("provider").and_then(serde_json::Value::as_str);
    let model = payload.get("model").and_then(serde_json::Value::as_str);
    let mode = payload
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| match provider {
            None => "deterministic",
            Some(provider) if is_local_provider(provider) => "local_model",
            Some(_) => "remote_model",
        });
    serde_json::json!({
        "mode": mode,
        "intent": payload.get("intent").cloned(),
        "provider": provider,
        "model": model,
        "provider_model": payload.get("provider_model").cloned(),
        "rate_group": payload.get("rate_group").cloned(),
        "privacy": payload.get("privacy").cloned(),
        "request_id": payload.get("request_id").cloned(),
    })
}

/// Blocking REPL turn used by /api/v1/repl: collects the streaming pipeline
/// into a single response plus structured trace.
pub(crate) async fn execute_repl_turn(prompt: &str) -> Result<(String, serde_json::Value), String> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let prompt_owned = prompt.to_string();
    tokio::spawn(async move {
        execute_repl_turn_streaming(&prompt_owned, tx).await;
    });

    let mut response = String::new();
    let mut trace = serde_json::Value::Null;
    while let Some(event) = rx.recv().await {
        match event {
            ReplStreamEvent::Route(_) => {}
            ReplStreamEvent::Token(text) => response.push_str(&text),
            ReplStreamEvent::Done(value) => {
                trace = value;
                break;
            }
            ReplStreamEvent::Error(message) => return Err(message),
        }
    }
    Ok((response, trace))
}

/// JSON route preview for POST /api/v1/route/preview: full auto-discovery,
/// quota admission, and the DREX decision without executing anything.
pub(crate) async fn preview_route_payload(prompt: &str) -> serde_json::Value {
    let mut registry = heiwa_provider::AccountRegistry::load();
    heiwa_provider::detect::auto_discover(&mut registry).await;
    let model_tiers = get_live_model_tiers(&registry);
    let pins = SessionPins::new();
    let now_unix = Utc::now().timestamp();
    let quota_ledger = open_default_quota_ledger();
    let quota = quota_budget_preview_lines(&model_tiers, quota_ledger.as_ref(), now_unix);
    let privacy = privacy_for_task(prompt);

    match route_task_with_quota(prompt, &pins, &model_tiers, quota_ledger.as_ref(), now_unix) {
        Ok(RouteOutcome::Deterministic(response)) => serde_json::json!({
            "mode": "deterministic",
            "privacy": privacy,
            "response": response,
            "quota": quota,
        }),
        Ok(RouteOutcome::Routed(route)) => {
            let metadata = serde_json::from_str::<serde_json::Value>(&route.routing_metadata)
                .unwrap_or(serde_json::Value::String(route.routing_metadata.clone()));
            serde_json::json!({
                "mode": if is_local_provider(&route.provider) { "local_model" } else { "remote_model" },
                "intent": route.intent_key,
                "provider": route.provider,
                "model": route.model_id,
                "provider_model": route.provider_model_id,
                "rate_group": route.rate_group,
                "privacy": route.privacy,
                "metadata": metadata,
                "quota": quota,
            })
        }
        Err(error) => serde_json::json!({
            "mode": "unavailable",
            "privacy": privacy,
            "error": error,
            "quota": quota,
        }),
    }
}

#[cfg(test)]
mod tests {
    use heiwa_protocol::{parse_turn_intent, Intent};

    /// Executor present. Tier projection filters on whether the account's
    /// executor exists on the host, so a test that does not state the answer
    /// asserts against the machine's installed tooling: a `claude` seat or an
    /// `ollama` runtime is routable on a developer laptop and filtered out on
    /// a CI runner.
    const INSTALLED: fn(&str) -> bool = |_| true;

    /// The BYOK path must survive the turn path, not just the adapter.
    ///
    /// Direct-API accounts are registered under the vendor name the
    /// credential belongs to ("anthropic"), while DREX routes are named after
    /// the surface ("claude"). A second alias table in this binary once
    /// failed to map between them, so every direct-API model was filtered out
    /// before routing ever saw it and a user with a valid key got "no models
    /// with working adapters". Alias mapping now has exactly one owner.
    #[test]
    fn direct_api_accounts_survive_the_live_model_tier_filter() {
        use heiwa_provider::registry::{
            AccountRegistry, AccountStatus, Credential, DetectedModel, InventoryTruth,
            ProviderAccount,
        };

        fn account(vendor: &str, model_id: &str) -> ProviderAccount {
            ProviderAccount {
                account_id: format!("{vendor}-api-1"),
                provider: vendor.to_string(),
                credential: Credential::ApiKey,
                rate_group: format!("{vendor}_api"),
                status: AccountStatus::Connected,
                models: vec![DetectedModel {
                    model_id: model_id.to_string(),
                    provider_model_id: model_id.to_string(),
                    provider: vendor.to_string(),
                    account_id: format!("{vendor}-api-1"),
                    rate_group: format!("{vendor}_api"),
                    capability_class: 5,
                    context_window: 200_000,
                    supports_streaming: true,
                    supports_tools: true,
                    supports_vision: true,
                    supports_audio: false,
                    cost_per_1k_input: 0.0,
                    cost_per_1k_output: 0.0,
                    inventory_truth: InventoryTruth::Verified,
                }],
            }
        }

        for (vendor, model_id) in [
            ("anthropic", "claude-opus-5"),
            ("openai", "gpt-5"),
            ("google", "gemini-3-pro"),
        ] {
            let registry = AccountRegistry {
                accounts: vec![account(vendor, model_id)],
            };
            let tiers = super::get_live_model_tiers(&registry);
            assert!(
                !tiers.is_empty(),
                "a {vendor} API-key account produced no routable tier — the BYOK \
                 path is unreachable from a turn"
            );
            assert!(
                tiers.iter().any(|tier| tier.model_id == model_id),
                "{vendor} tier list is missing {model_id}: {:?}",
                tiers.iter().map(|t| &t.model_id).collect::<Vec<_>>()
            );
            assert!(super::has_adapter(vendor), "{vendor} must have an adapter");
        }
    }

    #[test]
    fn automation_approval_request_becomes_awaiting_confirmation() {
        let outcome = super::automation_terminal_outcome(
            &heiwa_evidence::OperatorEventType::ApprovalRequested,
            &serde_json::json!({
                "request_id": "approval-42",
                "message": "calendar write needs approval",
            }),
            None,
            "daily brief",
            "automation-thread",
            "turn-42",
        );

        assert_eq!(
            outcome,
            Some(heiwa_automations::ExecutionOutcome::AwaitingConfirmation {
                request_id: "approval-42".to_string(),
                summary: "calendar write needs approval".to_string(),
            })
        );
    }

    #[test]
    fn operator_route_frame_keeps_repl_route_shape() {
        let payload = super::operator_repl_route_payload(&serde_json::json!({
            "intent": "chat",
            "provider": "ollama",
            "model": "qwen3.5:9b",
            "provider_model": "qwen3.5:9b",
            "rate_group": "local",
            "privacy": "standard",
            "request_id": "request-1",
        }));
        assert_eq!(payload["mode"], "local_model");
        assert_eq!(payload["intent"], "chat");
        assert_eq!(payload["provider"], "ollama");
        assert_eq!(payload["model"], "qwen3.5:9b");
        assert_eq!(payload["rate_group"], "local");
        assert_eq!(payload["privacy"], "standard");
        assert_eq!(payload["request_id"], "request-1");
    }

    #[test]
    fn privacy_cues_force_sovereign_lane() {
        assert_eq!(
            super::privacy_for_task("summarize my priority mail privately"),
            "sovereign"
        );
        assert_eq!(
            super::privacy_for_task("draft a CONFIDENTIAL reply"),
            "sovereign"
        );
        assert_eq!(
            super::privacy_for_task("this is sensitive — do not share"),
            "sovereign"
        );
        assert_eq!(
            super::privacy_for_task("review my personal finances plan"),
            "sovereign"
        );
    }

    #[test]
    fn privacy_defaults_to_standard() {
        assert_eq!(
            super::privacy_for_task("refactor the auth module and add tests"),
            "standard"
        );
        assert_eq!(super::privacy_for_task("hi"), "standard");
    }

    #[test]
    fn greeting_input_defaults_to_chat_intent() {
        assert_eq!(parse_turn_intent("hi").intent, Intent::Chat);
        assert_eq!(parse_turn_intent("hello there").intent, Intent::Chat);
    }

    #[test]
    fn coding_input_uses_build_intent() {
        assert_eq!(
            parse_turn_intent("refactor this Rust function").intent,
            Intent::Build
        );
        assert_eq!(
            parse_turn_intent("fix the failing cargo test").intent,
            Intent::Build
        );
    }

    #[test]
    fn research_input_uses_research_intent() {
        assert_eq!(
            parse_turn_intent("explain how DREX routing works").intent,
            Intent::Research
        );
        assert_eq!(
            parse_turn_intent("what is the weather like").intent,
            Intent::Research
        );
    }

    #[test]
    fn deploy_input_uses_deploy_intent() {
        assert_eq!(
            parse_turn_intent("deploy this to cloudflare").intent,
            Intent::Deploy
        );
        assert_eq!(
            parse_turn_intent("ship the new release").intent,
            Intent::Deploy
        );
    }

    #[test]
    fn strategy_input_uses_strategy_intent() {
        assert_eq!(
            parse_turn_intent("plan the roadmap for Q3").intent,
            Intent::Strategy
        );
        assert_eq!(
            parse_turn_intent("design the architecture").intent,
            Intent::Strategy
        );
    }

    #[test]
    fn audit_input_uses_audit_intent() {
        assert_eq!(parse_turn_intent("review the PR").intent, Intent::Audit);
        assert_eq!(parse_turn_intent("lint the codebase").intent, Intent::Audit);
    }

    #[test]
    fn openrouter_passes_every_adapter_gate() {
        // Three gates stand between an accounts.json entry and DREX routing;
        // a provider missing from any one of them silently drops out.
        assert!(super::has_adapter("openrouter"));
        assert!(super::provider_supports_loop_adapter("openrouter"));
        assert!(
            !super::is_local_provider("openrouter"),
            "openrouter is a remote tier — must not slip into the sovereign lane"
        );
    }

    #[test]
    fn math_question_does_not_false_positive_to_code() {
        // "what is 3/4?" should not match code just because of `/`
        assert_eq!(parse_turn_intent("what is 3/4?").intent, Intent::Research);
    }

    #[test]
    fn provider_pin_with_using_keyword() {
        let req = parse_turn_intent("using ollama explain the code");
        assert_eq!(req.provider_pin.as_deref(), Some("ollama"));
        assert!(req.model_pin.is_none()); // "explain" is a task starter word
    }

    #[test]
    fn provider_pin_with_keyword() {
        let req = parse_turn_intent("with claude sonnet-4 fix the bug");
        assert_eq!(req.provider_pin.as_deref(), Some("claude"));
        assert_eq!(req.model_pin.as_deref(), Some("sonnet-4"));
    }

    #[test]
    fn has_adapter_filters_known_providers() {
        assert!(super::has_adapter("ollama"));
        assert!(super::has_adapter("claude"));
        assert!(super::has_adapter("codex"));
        assert!(super::has_adapter("gemini"));
        assert!(!super::has_adapter("mystery-provider"));
    }

    #[test]
    fn vendor_names_resolve_to_the_same_adapters_as_route_names() {
        // The registry names accounts after the vendor whose key it holds;
        // DREX names routes after the surface. When the shell kept its own
        // alias table, these disagreed and every direct-API model was dropped
        // before routing — a user with a valid key saw "no working adapters".
        for vendor in ["anthropic", "openai", "google"] {
            assert!(
                super::has_adapter(vendor),
                "vendor name `{vendor}` must resolve to an adapter"
            );
        }
    }

    #[test]
    fn provider_adapter_checks_accept_cli_provider_ids() {
        assert!(super::provider_supports_loop_adapter("claude-code"));
        assert!(super::provider_supports_loop_adapter("google-gemini-cli"));
        assert!(super::has_adapter("claude-code"));
        assert!(super::has_adapter("google-gemini-cli"));
    }

    #[test]
    fn a_cli_seat_whose_binary_is_gone_offers_no_route() {
        // Health said NotInstalled and nothing asked: the tier filter read
        // stored status only, so a turn was routed to an adapter that could
        // not start and died on a raw OS error instead of routing elsewhere.
        let registry = heiwa_provider::AccountRegistry {
            accounts: vec![heiwa_provider::ProviderAccount {
                account_id: "anthropic-cli".to_string(),
                provider: "claude-code".to_string(),
                credential: heiwa_provider::Credential::OauthCli {
                    binary: "definitely-not-installed-xyz".to_string(),
                },
                rate_group: "claude_code".to_string(),
                status: heiwa_provider::AccountStatus::Connected,
                models: vec![heiwa_provider::DetectedModel {
                    model_id: "claude/sonnet-4-6".to_string(),
                    provider_model_id: "claude-sonnet-4-6".to_string(),
                    provider: "claude-code".to_string(),
                    account_id: "anthropic-cli".to_string(),
                    rate_group: "claude_code".to_string(),
                    capability_class: 4,
                    context_window: 200_000,
                    supports_streaming: true,
                    supports_tools: true,
                    supports_vision: false,
                    supports_audio: false,
                    cost_per_1k_input: 0.003,
                    cost_per_1k_output: 0.015,
                    inventory_truth: heiwa_provider::InventoryTruth::Inferred,
                }],
            }],
        };

        assert!(
            super::get_live_model_tiers(&registry).is_empty(),
            "an account whose executor is missing must not be offered as a route"
        );
    }

    #[test]
    fn live_model_tiers_canonicalize_cli_provider_ids() {
        let registry = heiwa_provider::AccountRegistry {
            accounts: vec![heiwa_provider::ProviderAccount {
                account_id: "anthropic-cli".to_string(),
                provider: "claude-code".to_string(),
                credential: heiwa_provider::Credential::OauthCli {
                    binary: "claude".to_string(),
                },
                rate_group: "claude_code".to_string(),
                status: heiwa_provider::AccountStatus::Connected,
                models: vec![heiwa_provider::DetectedModel {
                    model_id: "claude/sonnet-4-6".to_string(),
                    provider_model_id: "claude-sonnet-4-6".to_string(),
                    provider: "claude-code".to_string(),
                    account_id: "anthropic-cli".to_string(),
                    rate_group: "claude_code".to_string(),
                    capability_class: 4,
                    context_window: 200_000,
                    supports_streaming: true,
                    supports_tools: true,
                    supports_vision: false,
                    supports_audio: false,
                    cost_per_1k_input: 0.003,
                    cost_per_1k_output: 0.015,
                    inventory_truth: heiwa_provider::InventoryTruth::Inferred,
                }],
            }],
        };

        let tiers = super::get_live_model_tiers_with(&registry, INSTALLED);

        assert_eq!(tiers.len(), 1);
        assert_eq!(tiers[0].provider, "claude");
    }

    #[test]
    fn live_inventory_ids_are_stable_nonzero_and_unique_per_provider_model() {
        let model = |name: &str| heiwa_provider::DetectedModel {
            model_id: name.to_string(),
            provider_model_id: name.to_string(),
            provider: "ollama".to_string(),
            account_id: "ollama-local".to_string(),
            rate_group: "local".to_string(),
            capability_class: 3,
            context_window: 32_768,
            supports_streaming: true,
            supports_tools: true,
            supports_vision: false,
            supports_audio: false,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            inventory_truth: heiwa_provider::InventoryTruth::Verified,
        };
        let mut registry = heiwa_provider::AccountRegistry {
            accounts: vec![heiwa_provider::ProviderAccount {
                account_id: "ollama-local".to_string(),
                provider: "ollama".to_string(),
                credential: heiwa_provider::Credential::LocalRuntime {
                    endpoint: "http://127.0.0.1:11434".to_string(),
                },
                rate_group: "local".to_string(),
                status: heiwa_provider::AccountStatus::Connected,
                models: vec![model("gemma4"), model("qwen3.5:9b")],
            }],
        };

        let first = super::get_live_model_tiers_with(&registry, INSTALLED);
        registry.accounts[0].models.reverse();
        let second = super::get_live_model_tiers_with(&registry, INSTALLED);
        let first_ids = first.iter().map(|tier| tier.id).collect::<Vec<_>>();
        let second_ids = second.iter().map(|tier| tier.id).collect::<Vec<_>>();

        assert!(first_ids.iter().all(|id| *id != 0));
        assert_ne!(first_ids[0], first_ids[1]);
        assert_eq!(first_ids, second_ids);
    }

    #[test]
    fn discovered_ollama_models_are_zero_cost_on_device_budget_candidates() {
        let model = |name: &str| heiwa_provider::DetectedModel {
            model_id: name.to_string(),
            provider_model_id: name.to_string(),
            provider: "ollama".to_string(),
            account_id: "ollama-local".to_string(),
            rate_group: "local".to_string(),
            capability_class: 3,
            context_window: 32_768,
            supports_streaming: true,
            supports_tools: true,
            supports_vision: false,
            supports_audio: false,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            inventory_truth: heiwa_provider::InventoryTruth::Verified,
        };
        let registry = heiwa_provider::AccountRegistry {
            accounts: vec![heiwa_provider::ProviderAccount {
                account_id: "ollama-local".to_string(),
                provider: "ollama".to_string(),
                credential: heiwa_provider::Credential::LocalRuntime {
                    endpoint: "http://127.0.0.1:11434".to_string(),
                },
                rate_group: "local".to_string(),
                status: heiwa_provider::AccountStatus::Connected,
                models: vec![model("gemma4"), model("qwen3.5:9b")],
            }],
        };
        let tiers = super::get_live_model_tiers_with(&registry, INSTALLED);
        let candidates = tiers
            .iter()
            .map(super::model_call_candidate)
            .collect::<Vec<_>>();

        assert!(candidates.iter().all(|candidate| {
            candidate.locality == heiwa_core::drex::ExecutionLocality::OnDevice
                && candidate.cost_truth == heiwa_core::drex::CostTruth::LocalZeroCost
                && candidate.marginal_cost_usd == Some(0.0)
                && candidate.adapter_capable
        }));
        let plan = heiwa_core::drex::plan_model_call(
            &heiwa_core::drex::ModelCallRequest {
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
                work_id: None,
                call_id: "call".to_string(),
                intent: "code".to_string(),
                stage: heiwa_core::drex::ModelCallStage::Execution,
                raw_text: "local work".to_string(),
                privacy: heiwa_core::drex::PrivacyClass::Sovereign,
                risk: heiwa_core::drex::CallRisk::Low,
                safety: heiwa_core::drex::SafetyClass::low_risk_auto_approval(
                    &heiwa_core::drex::CallRisk::Low,
                ),
                required_capabilities: vec![],
                required_context_tokens: 1,
                minimum_quality_class: 1,
                minimum_success_rate: 0.0,
                maximum_marginal_cost_usd: Some(0.0),
                preferred_provider: None,
                preferred_model: None,
                allowed_models: vec![],
                excluded_models: vec![],
            },
            &candidates,
            &heiwa_core::drex::default_policy(),
        )
        .unwrap();
        assert!(plan.selected.is_some());
        assert_eq!(plan.admitted_ids.len(), 2);
    }

    #[test]
    fn route_task_handles_greeting_without_models() {
        let pins = super::SessionPins::new();
        let outcome = super::route_task("hi", &pins, &[]).expect("greeting should route");

        match outcome {
            super::RouteOutcome::Deterministic(response) => {
                assert!(
                    response.contains("Ready"),
                    "unexpected response: {response}"
                );
            }
            super::RouteOutcome::Routed(_) => panic!("greeting should not route to a model"),
        }
    }

    #[test]
    fn route_task_skips_remote_group_when_quota_exhausted() {
        let ledger = heiwa_quota::QuotaLedger::open_in_memory().expect("ledger");
        let now = 1_777_000_000;
        ledger
            .record_use(
                "claude",
                "anthropic",
                super::QUOTA_ADMISSION_WINDOW_SECONDS,
                super::REMOTE_RATE_GROUP_TOKEN_LIMIT,
                1,
                now,
            )
            .expect("seed quota");
        let pins = super::SessionPins::new();
        let tiers = vec![
            test_model_tier("claude", "claude-sonnet", "anthropic", 4, 0.20),
            test_model_tier("ollama", "qwen3.5:9b", "local_ollama", 3, 0.0),
        ];

        let outcome = super::route_task_with_quota(
            "explain the product strategy tradeoff",
            &pins,
            &tiers,
            Some(&ledger),
            now + 10,
        )
        .expect("route should fall back");

        match outcome {
            super::RouteOutcome::Routed(route) => {
                assert_eq!(route.provider, "ollama");
                assert_eq!(route.rate_group, "local_ollama");
            }
            super::RouteOutcome::Deterministic(_) => panic!("strategy task should route"),
        }
    }

    #[test]
    fn route_task_private_prompt_uses_local_model_when_available() {
        let pins = super::SessionPins::new();
        let tiers = vec![
            test_model_tier("claude", "claude-sonnet", "anthropic", 4, 0.20),
            test_model_tier("ollama", "qwen3.5:9b", "local_ollama", 3, 0.0),
        ];

        let outcome = super::route_task_with_quota(
            "summarize my priority mail privately",
            &pins,
            &tiers,
            None,
            1_777_000_000,
        )
        .expect("private prompt should route when a local model is available");

        match outcome {
            super::RouteOutcome::Routed(route) => {
                assert_eq!(route.privacy, "sovereign");
                assert_eq!(route.provider, "ollama");
                assert_eq!(route.rate_group, "local_ollama");
            }
            super::RouteOutcome::Deterministic(_) => {
                panic!("private prompt should route to a local model")
            }
        }
    }

    #[test]
    fn auto_route_preserves_remote_candidate_for_per_call_quality_floor() {
        let ledger = heiwa_quota::QuotaLedger::open_in_memory().expect("ledger");
        let pins = super::SessionPins::new();
        let mut tiers = vec![
            test_model_tier("ollama", "qwen3.5:4b", "local_ollama", 2, 0.0),
            test_model_tier("claude", "claude-sonnet", "anthropic", 4, 0.20),
        ];
        tiers[0].id = 1;
        tiers[1].id = 2;
        let outcome = super::route_task_with_quota(
            "compare these deployment approaches in one concise paragraph",
            &pins,
            &tiers,
            Some(&ledger),
            1_777_000_000,
        )
        .expect("auto route should preserve the admitted inventory");
        let super::RouteOutcome::Routed(route) = outcome else {
            panic!("comparison task should route to a model")
        };

        assert_eq!(
            route.candidates.len(),
            2,
            "legacy local-first preflight must not discard the opposite lane"
        );
        let plan = heiwa_core::drex::plan_model_call(
            &heiwa_core::drex::ModelCallRequest {
                thread_id: "thread".to_string(),
                turn_id: "turn".to_string(),
                work_id: None,
                call_id: "call".to_string(),
                intent: "chat".to_string(),
                stage: heiwa_core::drex::ModelCallStage::Execution,
                raw_text: "compare these deployment approaches".to_string(),
                privacy: heiwa_core::drex::PrivacyClass::Standard,
                risk: heiwa_core::drex::CallRisk::Low,
                safety: heiwa_core::drex::SafetyClass::low_risk_auto_approval(
                    &heiwa_core::drex::CallRisk::Low,
                ),
                required_capabilities: vec![],
                required_context_tokens: 1,
                minimum_quality_class: 4,
                minimum_success_rate: 0.0,
                maximum_marginal_cost_usd: Some(0.20),
                preferred_provider: None,
                preferred_model: None,
                allowed_models: vec![],
                excluded_models: vec![],
            },
            &route.candidates,
            &heiwa_core::drex::default_policy(),
        )
        .unwrap();
        let selected = plan
            .selected
            .expect("quality floor should admit remote model");
        assert_eq!(selected.tier.provider, "claude");
        assert_eq!(selected.tier.model_id, "claude-sonnet");
    }

    #[test]
    fn remote_only_main_route_preserves_local_auxiliary_call_candidates() {
        let ledger = heiwa_quota::QuotaLedger::open_in_memory().expect("ledger");
        let mut pins = super::SessionPins::new();
        pins.route_preference = super::RoutePreference::RemoteOnly;
        let mut tiers = vec![
            test_model_tier("ollama", "qwen3.5:4b", "local_ollama", 2, 0.0),
            test_model_tier("claude", "claude-sonnet", "anthropic", 4, 0.20),
        ];
        tiers[0].id = 1;
        tiers[1].id = 2;

        let outcome = super::route_task_with_quota(
            "compare deployment approaches with enough detail to require a model",
            &pins,
            &tiers,
            Some(&ledger),
            1_777_000_000,
        )
        .expect("remote-only main call should route");
        let super::RouteOutcome::Routed(route) = outcome else {
            panic!("comparison task should route to a model")
        };

        assert_eq!(
            route
                .candidates
                .iter()
                .map(|candidate| candidate.tier.provider.as_str())
                .collect::<Vec<_>>(),
            vec!["claude"],
            "main-call candidates must still honor remote-only"
        );
        assert_eq!(
            route
                .local_auxiliary_candidates
                .iter()
                .map(|candidate| candidate.tier.provider.as_str())
                .collect::<Vec<_>>(),
            vec!["ollama"],
            "local auxiliary calls need inventory independent of main route"
        );
    }

    #[test]
    fn remote_large_chat_prompt_is_compressed_before_model_send() {
        let route = super::RouteResult {
            candidates: vec![],
            local_auxiliary_candidates: vec![],
            model_id: "claude-sonnet".to_string(),
            provider: "claude".to_string(),
            provider_model_id: "claude-sonnet".to_string(),
            rate_group: "anthropic".to_string(),
            routing_metadata: "{}".to_string(),
            intent_key: "chat".to_string(),
            privacy: "standard".to_string(),
            request_id: "req-compress".to_string(),
            turn_started_at: "2026-05-26T00:00:00Z".to_string(),
        };
        let input = "x".repeat(super::ROUTE_COMPRESSION_BYTE_THRESHOLD + 1);

        let prepared =
            super::prepare_outbound_prompt_for_route_with(&route, &input, |body, source| {
                assert_eq!(body, input);
                assert_eq!(source, "route:req-compress:claude:chat");
                Ok(super::RouteCompressionResult {
                    compressed: "compressed payload".to_string(),
                    receipt_path: "/tmp/cmp.json".to_string(),
                    input_chars: body.chars().count(),
                    output_chars: 18,
                    ratio: 18.0 / body.chars().count() as f64,
                    input_tokens: 1024,
                    output_tokens: 6,
                    estimated_usd_saved: 0.003054,
                })
            });

        assert_eq!(prepared.model_prompt, "compressed payload");
        let compression = prepared.compression.expect("compression metadata");
        assert!(compression.applied);
        assert_eq!(compression.receipt_path.as_deref(), Some("/tmp/cmp.json"));
    }

    #[test]
    fn fallback_result_replaces_primary_route_attribution() {
        let route = super::RouteResult {
            candidates: vec![],
            local_auxiliary_candidates: vec![],
            model_id: "primary-model".to_string(),
            provider: "primary".to_string(),
            provider_model_id: "primary-model".to_string(),
            rate_group: "primary-rate".to_string(),
            routing_metadata: "{}".to_string(),
            intent_key: "chat".to_string(),
            privacy: "standard".to_string(),
            request_id: "req-fallback".to_string(),
            turn_started_at: "2026-05-26T00:00:00Z".to_string(),
        };
        let result = heiwa_shell::model_calls::ModelCallResult {
            route_receipt_ref: "test-route-receipt".to_string(),
            provider: "secondary".to_string(),
            model_id: "secondary-model".to_string(),
            provider_model_id: "secondary-provider-model".to_string(),
            rate_group: "secondary-rate".to_string(),
            text: "done".to_string(),
            usage: heiwa_provider::adapter::TokenUsage::default(),
            attempts: 2,
            failed_models: vec!["primary/primary-model".to_string()],
            cost_usd: 0.03,
            cost_truth: heiwa_core::drex::CostTruth::ProxyEstimate,
            attempt_records: vec![],
        };

        let resolved = super::resolved_route_after_model_call(&route, &result);

        assert_eq!(resolved.provider, "secondary");
        assert_eq!(resolved.model_id, "secondary-model");
        assert_eq!(resolved.provider_model_id, "secondary-provider-model");
        assert_eq!(resolved.rate_group, "secondary-rate");
        let usage = super::usage_for_model_call(&result);
        let trace = super::repl_trace_payload(
            "remote_model",
            Some(&resolved),
            Some(&usage),
            Some(&result),
            None,
        );
        assert_eq!(trace["provider"], "secondary");
        assert_eq!(trace["model"], "secondary-model");
        assert_eq!(trace["cost_usd"], 0.03);
        assert_eq!(trace["attempts"], 2);
    }

    #[test]
    fn fallback_receipt_persists_executor_usd_truth_and_failed_spend() {
        let receipts = heiwa_receipts::ReceiptStore::open_in_memory().unwrap();
        let rates = heiwa_receipts::RateTable::default();
        let result = heiwa_shell::model_calls::ModelCallResult {
            route_receipt_ref: "test-route-receipt".to_string(),
            provider: "secondary".to_string(),
            model_id: "secondary-model".to_string(),
            provider_model_id: "secondary-provider-model".to_string(),
            rate_group: "secondary-rate".to_string(),
            text: "done".to_string(),
            usage: heiwa_provider::adapter::TokenUsage {
                input_tokens: 5,
                output_tokens: 2,
                cost_usd: 0.02,
                ..Default::default()
            },
            attempts: 2,
            failed_models: vec!["primary/primary-model".to_string()],
            cost_usd: 0.03,
            cost_truth: heiwa_core::drex::CostTruth::ProxyEstimate,
            attempt_records: vec![
                heiwa_shell::model_calls::ModelCallAttemptRecord {
                    candidate_id: 1,
                    provider: "primary".to_string(),
                    model_id: "primary-model".to_string(),
                    outcome: heiwa_shell::model_calls::ModelCallAttemptOutcome::Failed,
                    failure_class: Some(
                        heiwa_shell::model_calls::ProviderFailureClass::RateLimited,
                    ),
                    provider_invoked: true,
                    cost_usd: Some(0.01),
                    cost_truth: heiwa_core::drex::CostTruth::TargetOnly,
                },
                heiwa_shell::model_calls::ModelCallAttemptRecord {
                    candidate_id: 2,
                    provider: "secondary".to_string(),
                    model_id: "secondary-model".to_string(),
                    outcome: heiwa_shell::model_calls::ModelCallAttemptOutcome::Completed,
                    failure_class: None,
                    provider_invoked: true,
                    cost_usd: Some(0.02),
                    cost_truth: heiwa_core::drex::CostTruth::TargetOnly,
                },
            ],
        };
        let usage = super::usage_for_model_call(&result);

        super::record_call_receipt(
            &receipts,
            &rates,
            super::CallReceiptInput {
                result: &result,
                usage: Some(&usage),
                session_id: "session",
                input_text: "input",
                output_text: "output",
                latency_ms: 10,
            },
        );

        let rows = receipts.list(0, i64::MAX).unwrap();
        assert_eq!(rows.len(), 1);
        let receipt = &rows[0];
        assert_eq!(receipt.provider, "secondary");
        assert_eq!(receipt.model, "secondary-model");
        assert_eq!(receipt.actual_cost_cad, 0.0);
        assert_eq!(receipt.model_call_cost_usd, Some(0.03));
        assert_eq!(
            receipt.model_call_cost_truth.as_deref(),
            Some("proxy_estimate")
        );
        assert_eq!(receipt.model_call_attempts, Some(2));
        assert_eq!(receipt.failed_attempt_cost_usd, Some(0.01));
    }

    #[test]
    fn compression_trace_suffix_includes_tokens_and_usd_when_applied() {
        let meta = super::RouteCompressionMetadata {
            applied: true,
            reason: "compressed".to_string(),
            receipt_path: Some("/tmp/cmp_test.json".to_string()),
            input_chars: 4096,
            output_chars: 1024,
            ratio: 0.25,
            input_tokens: 1024,
            output_tokens: 256,
            estimated_usd_saved: 0.002304,
        };
        let suffix = super::compression_trace_suffix(Some(&meta));
        assert!(suffix.contains("compression=applied"), "got: {suffix}");
        assert!(suffix.contains("tokens=1024->256"), "got: {suffix}");
        assert!(suffix.contains("saved=768"), "got: {suffix}");
        assert!(suffix.contains("usd_saved=0.002304"), "got: {suffix}");
        assert!(suffix.contains("ratio=0.250"), "got: {suffix}");
    }

    #[test]
    fn compression_trace_suffix_is_empty_when_none() {
        assert_eq!(super::compression_trace_suffix(None), "");
    }

    #[test]
    fn compression_trace_suffix_reports_reason_when_skipped() {
        let meta = super::RouteCompressionMetadata {
            applied: false,
            reason: "empty_output".to_string(),
            receipt_path: None,
            input_chars: 100,
            output_chars: 100,
            ratio: 1.0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_usd_saved: 0.0,
        };
        let suffix = super::compression_trace_suffix(Some(&meta));
        assert!(suffix.contains("compression=skipped"), "got: {suffix}");
        assert!(suffix.contains("empty_output"), "got: {suffix}");
    }

    #[test]
    fn pricing_for_provider_known_providers() {
        assert_eq!(
            super::pricing_for_provider("claude").usd_per_million_input_tokens,
            3.0
        );
        assert_eq!(
            super::pricing_for_provider("gemini").usd_per_million_input_tokens,
            1.25
        );
        assert_eq!(
            super::pricing_for_provider("ollama").usd_per_million_input_tokens,
            0.0
        );
        // Unknown providers fall back to conservative middle.
        assert_eq!(
            super::pricing_for_provider("mystery").usd_per_million_input_tokens,
            5.0
        );
    }

    #[test]
    fn pricing_for_provider_labels_token_count_basis() {
        let claude = super::pricing_for_provider("claude");
        assert_eq!(claude.tokenizer_id, "cl100k_base");
        assert_eq!(claude.token_count_kind, "proxy_estimate");
        assert_eq!(
            claude.exact_count_source.as_deref(),
            Some("anthropic_messages_count_tokens_api")
        );

        let ollama = super::pricing_for_provider("ollama");
        assert_eq!(ollama.token_count_kind, "local_zero_cost");
        assert_eq!(ollama.exact_count_source, None);
    }

    #[test]
    fn local_or_small_prompts_skip_route_compression() {
        let mut route = super::RouteResult {
            candidates: vec![],
            local_auxiliary_candidates: vec![],
            model_id: "qwen3.5:4b".to_string(),
            provider: "ollama".to_string(),
            provider_model_id: "qwen3.5:4b".to_string(),
            rate_group: "local".to_string(),
            routing_metadata: "{}".to_string(),
            intent_key: "chat".to_string(),
            privacy: "standard".to_string(),
            request_id: "req-local".to_string(),
            turn_started_at: "2026-05-26T00:00:00Z".to_string(),
        };
        let large_input = "x".repeat(super::ROUTE_COMPRESSION_BYTE_THRESHOLD + 1);

        let local = super::prepare_outbound_prompt_for_route_with(&route, &large_input, |_, _| {
            panic!("local routes must not compress");
        });
        assert_eq!(local.model_prompt, large_input);
        assert!(local.compression.is_none());

        route.provider = "claude".to_string();
        route.rate_group = "anthropic".to_string();
        let small = super::prepare_outbound_prompt_for_route_with(&route, "small", |_, _| {
            panic!("small prompts must not compress");
        });
        assert_eq!(small.model_prompt, "small");
        assert!(small.compression.is_none());
    }

    #[test]
    fn quota_admission_fails_remote_closed_without_ledger() {
        let tiers = vec![
            test_model_tier("claude", "claude-sonnet", "anthropic", 4, 0.20),
            test_model_tier("ollama", "qwen3.5:9b", "local", 3, 0.0),
        ];

        let admission = super::quota_admitted_model_tiers(&tiers, None, 1_777_000_000);

        assert_eq!(admission.admitted.len(), 1);
        assert_eq!(admission.admitted[0].provider, "ollama");
        assert!(admission
            .exhausted_groups
            .iter()
            .any(|group| group.contains("claude/anthropic")));
    }

    #[test]
    fn local_quota_record_persists_usage_by_rate_group() {
        let ledger = heiwa_quota::QuotaLedger::open_in_memory().expect("ledger");
        let route = super::RouteResult {
            candidates: vec![],
            local_auxiliary_candidates: vec![],
            model_id: "gemma4".to_string(),
            provider: "ollama".to_string(),
            provider_model_id: "gemma4".to_string(),
            rate_group: "local".to_string(),
            routing_metadata: "{\"reason\":\"test\"}".to_string(),
            intent_key: "chat".to_string(),
            privacy: "standard".to_string(),
            request_id: "req-test".to_string(),
            turn_started_at: "2026-05-07T00:00:00Z".to_string(),
        };
        let usage = heiwa_provider::adapter::TokenUsage {
            input_tokens: 12,
            output_tokens: 7,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.03,
        };

        super::record_local_quota_run(
            &ledger,
            "run-test",
            &route,
            Some(&usage),
            None,
            1_777_000_000,
        )
        .expect("quota write");

        let quota = ledger
            .get_quota("ollama", "local")
            .expect("quota read")
            .expect("quota row");
        assert_eq!(quota.tokens_used, 19);
        assert_eq!(quota.requests, 1);

        let runs = ledger.recent_runs(1).expect("runs");
        assert_eq!(runs[0].id, "run-test");
        assert_eq!(runs[0].tokens_input, 12);
        assert_eq!(runs[0].tokens_output, 7);
        assert_eq!(runs[0].cost, 0.03);
        assert_eq!(runs[0].meta["request_id"], "req-test");
    }

    fn test_model_tier(
        provider: &str,
        model_id: &str,
        rate_group: &str,
        capability_class: u8,
        cost_per_turn: f64,
    ) -> heiwa_protocol::ModelTier {
        heiwa_protocol::ModelTier {
            id: 0,
            model_id: model_id.to_string(),
            provider_model_id: model_id.to_string(),
            provider: provider.to_string(),
            rate_group: rate_group.to_string(),
            capability_class,
            effort_knob: "default".to_string(),
            effort_level: 1,
            cost_per_turn,
            max_context_tokens: 32_768,
            vram_requirement_mb: if rate_group == "local_ollama" {
                4096
            } else {
                0
            },
            quantization_type: "none".to_string(),
            kv_cache_strategy: "standard".to_string(),
            strengths_json: serde_json::json!(["chat", "advanced_coding"]).to_string(),
            enabled: true,
            last_success_rate: 1.0,
            avg_latency_ms: 100,
            latency_p_95_ms: 200,
            updated_at: "2026-05-08T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn exit_slash_returns_none() {
        let mut pins = super::SessionPins::new();
        assert!(super::handle_slash("exit", &[], &[], &mut pins).is_none());
        assert!(super::handle_slash("quit", &[], &[], &mut pins).is_none());
    }

    #[test]
    fn cwd_slash_tracks_current_working_directory() {
        let mut pins = super::SessionPins::new();
        let current = std::env::current_dir().unwrap().canonicalize().unwrap();

        let response = super::handle_slash("cwd", &[], &[], &mut pins).unwrap();
        assert!(response.contains(&current.display().to_string()));

        let response = super::handle_slash("cwd", &[".".to_string()], &[], &mut pins).unwrap();
        assert_eq!(pins.scope.working_dir, current);
        assert!(response.contains(&current.display().to_string()));
        assert!(pins.scope.allowed_dirs.iter().any(|path| path == &current));
    }

    #[test]
    fn add_dir_expands_home_children_glob() {
        let mut pins = super::SessionPins::new();
        let response = super::handle_slash("add-dir", &["~/*".to_string()], &[], &mut pins)
            .expect("add-dir should respond");

        assert!(
            response.contains("added dirs") || response.contains("no new dirs"),
            "unexpected response: {response}"
        );
        assert!(!pins.scope.allowed_dirs.is_empty());
    }

    #[test]
    fn model_context_includes_working_dirs() {
        let pins = super::SessionPins::new();
        let messages = super::build_messages_from_transcript(
            &[heiwa_protocol::TranscriptBlock::User("prior".into())],
            "status",
            &pins,
        );
        assert!(matches!(
            messages.first().unwrap().role,
            heiwa_provider::adapter::Role::System
        ));
        assert!(messages
            .first()
            .unwrap()
            .content
            .contains("current directory:"));
        assert_eq!(messages.last().unwrap().content, "status");
    }

    #[test]
    fn the_current_prompt_is_not_repeated_when_the_transcript_already_holds_it() {
        // The turn is persisted to the transcript before the prompt is built,
        // so the transcript's last entry is the message being sent. Appending
        // it again billed the user twice for the newest message on every
        // turn — the one carrying pasted context.
        use heiwa_protocol::TranscriptBlock;
        let pins = super::SessionPins::new();

        let messages = super::build_messages_from_transcript(
            &[
                TranscriptBlock::User("First question.".into()),
                TranscriptBlock::Assistant("An answer.".into()),
                TranscriptBlock::User("Second question.".into()),
            ],
            "Second question.",
            &pins,
        );

        let asked: Vec<&str> = messages
            .iter()
            .filter(|message| {
                matches!(message.role, heiwa_provider::adapter::Role::User)
                    && message.content == "Second question."
            })
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(
            asked.len(),
            1,
            "prompt sent {} times: {messages:?}",
            asked.len()
        );
        assert_eq!(messages.last().unwrap().content, "Second question.");
    }

    #[test]
    fn the_current_prompt_is_appended_when_the_transcript_does_not_hold_it() {
        use heiwa_protocol::TranscriptBlock;
        let pins = super::SessionPins::new();

        let messages = super::build_messages_from_transcript(
            &[TranscriptBlock::Assistant("An answer.".into())],
            "A new question.",
            &pins,
        );

        assert_eq!(messages.last().unwrap().content, "A new question.");
        assert!(matches!(
            messages.last().unwrap().role,
            heiwa_provider::adapter::Role::User
        ));
    }

    #[test]
    fn mode_slash_switches_between_direct_and_agentic() {
        let mut pins = super::SessionPins::new();
        assert_eq!(pins.cockpit_mode, super::CockpitMode::Direct);

        let response = super::handle_slash("mode", &["agentic".to_string()], &[], &mut pins)
            .expect("mode response");
        assert_eq!(response, "mode: agentic");
        assert_eq!(pins.cockpit_mode, super::CockpitMode::Agentic);

        let response = super::handle_slash("mode", &["direct".to_string()], &[], &mut pins)
            .expect("mode response");
        assert_eq!(response, "mode: direct");
        assert_eq!(pins.cockpit_mode, super::CockpitMode::Direct);
    }

    #[test]
    fn scoped_shell_blocks_paths_outside_execution_scope() {
        let pins = super::SessionPins::new();
        let error =
            super::run_scoped_shell("cat /etc/passwd", &pins.scope, &pins.principal).unwrap_err();
        assert!(error.contains("outside execution scope"));
    }

    #[test]
    fn scoped_shell_denies_viewer_even_with_shell_lease() {
        let mut pins = super::SessionPins::new();
        pins.principal = heiwa_protocol::SessionPrincipal::new(
            "viewer",
            heiwa_protocol::PrincipalKind::HumanUser,
            heiwa_protocol::ExecutionRole::Viewer,
        );

        let error = super::run_scoped_shell("echo ok", &pins.scope, &pins.principal).unwrap_err();
        assert!(error.contains("lacks permission"));
    }
}
