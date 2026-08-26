use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use heiwa_protocol::{ExecutionScope, RiskClass, ToolLease};
use heiwa_resource::{ResourcePolicy, ResourceSnapshot, ThermalPressure, WorkClass};
use serde::Serialize;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::{self, Duration};

pub(crate) const DEFAULT_PORT: u16 = 7474;
const HEARTBEAT_TTL_SECS: i64 = 120;
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const BROWSER_BOOTSTRAP_TTL_SECONDS: i64 = 60;
const BROWSER_SESSION_TTL_SECONDS: i64 = 8 * 60 * 60;
const BROWSER_SESSION_COOKIE_PREFIX: &str = "heiwa_local_operator_";
const APPLE_ARM64_LINKER_ENV: &str = "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER";
const MACH_O_LINKER_FLAVOR: &str = "-C linker-flavor=ld64.lld";

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LocalAppProbe {
    pub port: u16,
    pub url: String,
    pub reachable: bool,
    pub latency_ms: Option<u64>,
}

pub(crate) fn probe_local_app(port: u16) -> LocalAppProbe {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let start = Instant::now();
    let reachable = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok();
    LocalAppProbe {
        port,
        url: format!("http://127.0.0.1:{port}/"),
        reachable,
        latency_ms: reachable.then(|| start.elapsed().as_millis() as u64),
    }
}

pub async fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("start") => start(&args[1..]).await,
        Some("update") => update(&args[1..]),
        Some("runtime") => runtime(&args[1..]),
        Some("api") => api(&args[1..]).await,
        Some("status") => runtime_status(args),
        Some("--help") | Some("-h") | None => {
            if args.iter().any(|arg| arg == "--json") {
                runtime_status(args)
            } else {
                print_help();
                Ok(())
            }
        }
        Some(flag) if flag.starts_with("--") => runtime_status(args),
        Some(other) => Err(anyhow!("unknown app command: {other}")),
    }
}

fn update(args: &[String]) -> Result<()> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        print_update_help();
        return Ok(());
    }

    let dry_run = has_flag(args, "--dry-run");
    let json_output = has_flag(args, "--json");
    let source = flag_value(args, "--source").unwrap_or_else(|| "github".to_string());

    match source.as_str() {
        "github" => update_from_github_release(dry_run, json_output),
        "checkout" => update_from_checkout(dry_run, json_output),
        other => Err(anyhow!(
            "invalid --source value: {other} (expected github or checkout)"
        )),
    }
}

fn update_from_github_release(dry_run: bool, json_output: bool) -> Result<()> {
    let install_root = heiwa_install::get_heiwa_dir();
    // `update` is reached from an async command dispatcher, and the release
    // update performs blocking HTTP, so it has to leave the async worker.
    tokio::task::block_in_place(|| {
        super::release_update::run(
            install_root,
            github_release_platform(),
            env!("CARGO_PKG_VERSION"),
            dry_run,
            json_output,
        )
    })
}

fn update_from_checkout(dry_run: bool, json_output: bool) -> Result<()> {
    let repo_root = find_repo_root(env::current_dir()?)
        .ok_or_else(|| anyhow!("heiwa app update must run from a heiwa-universe checkout"))?;
    let shell_manifest = repo_root
        .join("apps")
        .join("heiwa_shell")
        .join("Cargo.toml");
    if !shell_manifest.is_file() {
        return Err(anyhow!(
            "heiwa app update could not find apps/heiwa_shell/Cargo.toml under {}",
            repo_root.display()
        ));
    }

    let install_root = heiwa_install::get_heiwa_dir();
    let installed_bin = install_root.join("bin").join("heiwa");
    let installed_app = install_root.join("app").join("Heiwa.app");
    let cargo_environment = checkout_cargo_environment()?;
    let install_command = vec![
        "cargo".to_string(),
        "install".to_string(),
        "--path".to_string(),
        repo_root
            .join("apps")
            .join("heiwa_shell")
            .display()
            .to_string(),
        "--root".to_string(),
        install_root.display().to_string(),
        "--locked".to_string(),
        "--force".to_string(),
    ];
    let plan = checkout_update_plan(
        &repo_root,
        &installed_bin,
        &installed_app,
        dry_run,
        &install_command,
        &cargo_environment,
    );

    if json_output {
        println!("{}", serde_json::to_string(&plan)?);
    } else {
        println!("heiwa app update");
        println!("  source_mode: checkout-dev");
        println!("  source: {}", repo_root.display());
        println!(
            "  source_branch: {}",
            plan["source_branch"].as_str().unwrap_or("unknown")
        );
        println!(
            "  source_commit: {}",
            plan["source_commit"].as_str().unwrap_or("unknown")
        );
        println!(
            "  source_dirty: {}",
            plan["source_dirty"].as_bool().unwrap_or(false)
        );
        println!("  official_source: GitHub Releases");
        println!("  target: {}", installed_bin.display());
        println!("  cargo_environment: {}", cargo_environment.strategy);
        println!("  restart_policy: prompt-before-restart");
        println!(
            "  command: cargo install --path apps/heiwa_shell --root ~/.heiwa --locked --force"
        );
        if dry_run {
            println!("  dry_run: true");
            return Ok(());
        }
    }

    if dry_run {
        return Ok(());
    }

    let mut cargo = Command::new("cargo");
    cargo
        .arg("install")
        .arg("--path")
        .arg(repo_root.join("apps").join("heiwa_shell"))
        .arg("--root")
        .arg(&install_root)
        .arg("--locked")
        .arg("--force");
    cargo_environment.apply(&mut cargo);
    let status = cargo.status()?;
    if !status.success() {
        return Err(anyhow!("cargo install failed with status {status}"));
    }

    let desktop_bundle = plan
        .get("desktop_bundle_source")
        .ok_or_else(|| anyhow!("desktop bundle source missing from update plan"))?;
    let desktop_bundle_present = desktop_bundle
        .get("present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if desktop_bundle_present {
        let desktop_bundle_path = desktop_bundle
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("desktop bundle path missing from update plan"))?;
        heiwa_install::install_desktop_app_bundle(&install_root, Path::new(desktop_bundle_path))?;
    }
    let receipt_path = write_promotion_receipt(&plan)?;
    if !json_output {
        println!("  status: updated");
        println!(
            "  app_bundle: {}",
            if desktop_bundle_present {
                "updated"
            } else {
                "not built; CLI only"
            }
        );
        println!("  promotion_receipt: {}", receipt_path.display());
    }
    Ok(())
}

fn github_release_platform() -> &'static str {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => "macos-aarch64",
        ("linux", "x86_64") => "linux-x86_64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => "unsupported",
    }
}

fn checkout_update_plan(
    repo_root: &Path,
    installed_bin: &Path,
    installed_app: &Path,
    dry_run: bool,
    install_command: &[String],
    cargo_environment: &CheckoutCargoEnvironment,
) -> Value {
    let state = RuntimeStatus::detect();
    let pending_approvals = state
        .approvals_summary
        .get("pending")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let live_workers = state
        .workers_summary
        .get("live")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let runtime_workers = state
        .workers_summary
        .get("runtime_live")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let task_workers = state
        .workers_summary
        .get("task_live")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let blocking = pending_approvals > 0 || task_workers > 0;
    let receipt_id = format!(
        "heiwa-app-update-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    );
    let source_branch = git_output(repo_root, &["branch", "--show-current"])
        .unwrap_or_else(|| "unknown".to_string());
    let source_commit = git_output(repo_root, &["rev-parse", "--short", "HEAD"])
        .unwrap_or_else(|| "unknown".to_string());
    let source_dirty = git_is_dirty(repo_root);
    let installed_version = installed_heiwa_version(installed_bin);
    let desktop_bundle = desktop_bundle_source(repo_root);
    let promotion_receipt = promotion_receipt_plan(
        &receipt_id,
        repo_root,
        &source_branch,
        &source_commit,
        source_dirty,
        installed_bin,
        &installed_version,
        installed_app,
        &desktop_bundle,
        &cargo_environment.receipt(),
        dry_run,
    );

    json!({
        "command": "app update",
        "source_mode": "checkout-dev",
        "source": repo_root.display().to_string(),
        "source_branch": source_branch,
        "source_commit": source_commit,
        "source_dirty": source_dirty,
        "official_source": "GitHub Releases",
        "installed_bin": installed_bin.display().to_string(),
        "installed_version": installed_version,
        "installed_app": installed_app.display().to_string(),
        "installed_app_present": installed_app.join("Contents").join("MacOS").join("Heiwa").is_file(),
        "desktop_bundle_source": desktop_bundle,
        "app_bundle_update": app_bundle_update_plan(&desktop_bundle, dry_run),
        "install_command": install_command,
        "cargo_environment": cargo_environment.receipt(),
        "restart_policy": "prompt-before-restart",
        "restart_required": true,
        "dry_run": dry_run,
        "active_work": {
            "pending_approvals": pending_approvals,
            "live_workers": live_workers,
            "runtime_workers": runtime_workers,
            "task_workers": task_workers,
            "blocking_restart": blocking,
            "classification": if blocking { "blocking" } else { "none_or_pausable" },
        },
        "verification_commands": [
            "heiwa doctor",
            "heiwa app runtime status --json",
            "curl -fsS http://127.0.0.1:7474/status/health",
            "curl -fsS http://127.0.0.1:7474/api/v1/capabilities",
        ],
        "promotion_receipt": promotion_receipt,
    })
}

fn desktop_bundle_source(repo_root: &Path) -> Value {
    let bundle = repo_root
        .join("target")
        .join("release")
        .join("bundle")
        .join("macos")
        .join("Heiwa.app");
    let executable = bundle.join("Contents").join("MacOS").join("Heiwa");
    json!({
        "path": bundle.display().to_string(),
        "present": executable.is_file(),
        "executable": executable.display().to_string(),
    })
}

fn app_bundle_update_plan(desktop_bundle: &Value, dry_run: bool) -> Value {
    let source_present = desktop_bundle
        .get("present")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    json!({
        "wired": true,
        "status": if source_present { "ready" } else { "not_built" },
        "source_present": source_present,
        "would_install": source_present,
        "will_install": source_present && !dry_run,
        "blocker": if source_present {
            Value::Null
        } else {
            json!("build target/release/bundle/macos/Heiwa.app before checkout promotion to update the desktop surface")
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn promotion_receipt_plan(
    receipt_id: &str,
    repo_root: &Path,
    source_branch: &str,
    source_commit: &str,
    source_dirty: bool,
    installed_bin: &Path,
    installed_version: &Value,
    installed_app: &Path,
    desktop_bundle: &Value,
    cargo_environment: &Value,
    dry_run: bool,
) -> Value {
    let receipt_path = heiwa_install::get_heiwa_dir()
        .join("state")
        .join("evidence")
        .join("promotion")
        .join(format!("{receipt_id}.json"));

    json!({
        "schema_version": "heiwa_promotion_receipt_v1",
        "receipt_id": receipt_id,
        "event": "heiwa.app.update.checkout",
        "plane": "evidence",
        "created_at": chrono::Utc::now().to_rfc3339(),
        "would_write": !dry_run,
        "receipt_path": receipt_path.display().to_string(),
        "source": {
            "kind": "checkout",
            "path": repo_root.display().to_string(),
            "branch": source_branch,
            "commit": source_commit,
            "dirty": source_dirty,
        },
        "target": {
            "installed_bin": installed_bin.display().to_string(),
            "installed_version_before": installed_version,
            "installed_app": installed_app.display().to_string(),
            "desktop_bundle_source": desktop_bundle,
            "desktop_bundle_would_install": desktop_bundle.get("present").and_then(Value::as_bool).unwrap_or(false),
            "desktop_bundle_installed": !dry_run && desktop_bundle.get("present").and_then(Value::as_bool).unwrap_or(false),
        },
        "codesign": codesign_probe(desktop_bundle),
        "cargo_environment": cargo_environment,
        "runtime_probes": runtime_probe_contracts(),
        "evidence_plane": {
            "backend": "local-jsonl",
            "truth": "local text (JSONL/markdown); GitHub sync planned, redaction-gated",
            "index": "lance (derived, rebuildable)",
            "status": "local_only",
        },
        "restart_policy": "prompt-before-restart",
    })
}

#[derive(Debug)]
struct CheckoutCargoEnvironment {
    strategy: &'static str,
    linker: Option<PathBuf>,
    rustflags: Option<OsString>,
}

impl CheckoutCargoEnvironment {
    fn apply(&self, command: &mut Command) {
        if let Some(linker) = &self.linker {
            command.env(APPLE_ARM64_LINKER_ENV, linker);
        }
        if let Some(rustflags) = &self.rustflags {
            command.env("RUSTFLAGS", rustflags);
        }
    }

    fn receipt(&self) -> Value {
        json!({
            "strategy": self.strategy,
            "operator_override": self.strategy == "operator_override",
            "linker": self.linker.as_ref().map(|path| path.display().to_string()),
            "rustflags_append": if self.strategy == "rust_bundled_macho_linker" {
                Some(MACH_O_LINKER_FLAVOR)
            } else {
                None
            },
        })
    }
}

fn checkout_cargo_environment() -> Result<CheckoutCargoEnvironment> {
    let operator_linker = env::var_os(APPLE_ARM64_LINKER_ENV).filter(|value| !value.is_empty());
    if env::consts::OS != "macos" || env::consts::ARCH != "aarch64" || operator_linker.is_some() {
        return checkout_cargo_environment_from(
            env::consts::OS,
            env::consts::ARCH,
            operator_linker,
            env::var_os("RUSTFLAGS"),
            None,
            false,
        );
    }

    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .context("resolve the pinned Rust sysroot for checkout promotion")?;
    if !output.status.success() {
        return Err(anyhow!(
            "rustc --print sysroot failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let sysroot = String::from_utf8(output.stdout)
        .context("rustc --print sysroot returned non-UTF-8 output")?;
    let sysroot = PathBuf::from(sysroot.trim());
    let linker = bundled_macho_linker(&sysroot);
    checkout_cargo_environment_from(
        env::consts::OS,
        env::consts::ARCH,
        None,
        env::var_os("RUSTFLAGS"),
        Some(&sysroot),
        linker.is_file(),
    )
}

fn checkout_cargo_environment_from(
    os: &str,
    arch: &str,
    operator_linker: Option<OsString>,
    existing_rustflags: Option<OsString>,
    rust_sysroot: Option<&Path>,
    bundled_linker_exists: bool,
) -> Result<CheckoutCargoEnvironment> {
    if os != "macos" || arch != "aarch64" {
        return Ok(CheckoutCargoEnvironment {
            strategy: "host_default",
            linker: None,
            rustflags: None,
        });
    }
    if operator_linker.is_some_and(|value| !value.is_empty()) {
        return Ok(CheckoutCargoEnvironment {
            strategy: "operator_override",
            linker: None,
            rustflags: None,
        });
    }

    let sysroot = rust_sysroot.ok_or_else(|| anyhow!("pinned Rust sysroot is unavailable"))?;
    let linker = bundled_macho_linker(sysroot);
    if !bundled_linker_exists {
        return Err(anyhow!(
            "Rust's bundled rust-lld is missing at {}; reinstall the pinned Rust toolchain before checkout promotion",
            linker.display()
        ));
    }

    let mut rustflags = existing_rustflags.unwrap_or_default();
    if !rustflags
        .to_string_lossy()
        .contains("linker-flavor=ld64.lld")
    {
        if !rustflags.is_empty() {
            rustflags.push(" ");
        }
        rustflags.push(MACH_O_LINKER_FLAVOR);
    }
    Ok(CheckoutCargoEnvironment {
        strategy: "rust_bundled_macho_linker",
        linker: Some(linker),
        rustflags: Some(rustflags),
    })
}

fn bundled_macho_linker(rust_sysroot: &Path) -> PathBuf {
    rust_sysroot
        .join("lib")
        .join("rustlib")
        .join("aarch64-apple-darwin")
        .join("bin")
        .join("rust-lld")
}

fn runtime_probe_contracts() -> Value {
    json!([
        {
            "name": "health",
            "method": "GET",
            "endpoint": "/status/health",
            "expected_content_type": "application/json",
            "expected_json": true,
        },
        {
            "name": "capabilities_contract",
            "method": "GET",
            "endpoint": "/api/v1/capabilities",
            "expected_content_type": "application/json",
            "expected_json": true,
        },
        {
            "name": "runtime_snapshot",
            "method": "GET",
            "endpoint": "/api/v1/runtime/snapshot",
            "expected_content_type": "application/json",
            "expected_json": true,
        },
    ])
}

fn codesign_probe(desktop_bundle: &Value) -> Value {
    let Some(path) = desktop_bundle.get("path").and_then(Value::as_str) else {
        return json!({"status":"unknown","reason":"desktop bundle path missing"});
    };
    if !Path::new(path).exists() {
        return json!({"status":"not_present","path":path});
    }
    if env::consts::OS != "macos" {
        return json!({"status":"skipped","path":path,"reason":"codesign probe is macOS-only"});
    }
    match Command::new("codesign")
        .args(["--verify", "--deep", "--strict", path])
        .output()
    {
        Ok(output) => json!({
            "status": if output.status.success() { "verified" } else { "failed" },
            "path": path,
            "stdout": String::from_utf8_lossy(&output.stdout).trim(),
            "stderr": String::from_utf8_lossy(&output.stderr).trim(),
        }),
        Err(error) => json!({
            "status": "unavailable",
            "path": path,
            "error": error.to_string(),
        }),
    }
}

fn write_promotion_receipt(plan: &Value) -> Result<PathBuf> {
    let receipt = plan
        .get("promotion_receipt")
        .ok_or_else(|| anyhow!("promotion receipt missing from update plan"))?;
    let path = receipt
        .get("receipt_path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("promotion receipt path missing from update plan"))?;
    let path = PathBuf::from(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_vec_pretty(receipt)?)?;
    Ok(path)
}

fn git_output(repo_root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn git_is_dirty(repo_root: &Path) -> bool {
    git_output(repo_root, &["status", "--porcelain=v1"])
        .is_some_and(|status| !status.trim().is_empty())
}

fn installed_heiwa_version(installed_bin: &Path) -> Value {
    let output = Command::new(installed_bin).arg("--version").output();
    match output {
        Ok(output) if output.status.success() => {
            json!(String::from_utf8_lossy(&output.stdout).trim())
        }
        Ok(output) => json!({
            "status": "error",
            "stderr": String::from_utf8_lossy(&output.stderr).trim(),
        }),
        Err(err) => json!({
            "status": "unavailable",
            "error": err.to_string(),
        }),
    }
}

fn find_repo_root(start: PathBuf) -> Option<PathBuf> {
    for candidate in start.ancestors() {
        if candidate.join("HEIWA.md").is_file()
            && candidate
                .join("apps")
                .join("heiwa_shell")
                .join("Cargo.toml")
                .is_file()
        {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

fn runtime(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("status") | None => runtime_status(args),
        Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(anyhow!("unknown app runtime command: {other}")),
    }
}

async fn api(args: &[String]) -> Result<()> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        print_api_help();
        return Ok(());
    }
    let method = args
        .first()
        .map(|raw| raw.to_ascii_uppercase())
        .ok_or_else(|| anyhow!("usage: heiwa app api get|post <path> [--port N] [--body JSON]"))?;
    if method != "GET" && method != "POST" {
        return Err(anyhow!(
            "unknown app api method: {method} (expected get or post)"
        ));
    }
    let path = args
        .get(1)
        .filter(|path| path.starts_with('/'))
        .ok_or_else(|| anyhow!("app api path must start with /, e.g. /api/v1/session"))?;
    let port = parse_port(args)?;
    let dry_run = has_flag(args, "--dry-run");
    let json_output = has_flag(args, "--json");
    let body_raw = flag_value(args, "--body");
    let body_value = match body_raw.as_deref() {
        Some(raw) => read_api_body(raw)?,
        None => Value::Null,
    };
    let body_payload = if method == "POST" {
        Some(serde_json::to_string(&body_value)?)
    } else {
        None
    };
    let url = format!("http://127.0.0.1:{port}{path}");
    let machine_auth_token = heiwa_core::config::RuntimeConfig::from_env().machine_auth_token;
    let auth_state = if machine_auth_token.trim().is_empty() {
        "missing"
    } else {
        "machine_token_configured"
    };

    if dry_run {
        let payload = json!({
            "command": "app api",
            "method": method,
            "path": path,
            "url": url,
            "dry_run": true,
            "auth": auth_state,
            "body": body_value,
            "next": "drop --dry-run to call the running local Heiwa.app runtime",
        });
        if json_output {
            println!("{payload}");
        } else {
            println!("heiwa app api");
            println!("  method: {}", payload["method"].as_str().unwrap_or("?"));
            println!("  url: {url}");
            println!("  dry_run: true");
            println!("  auth: {auth_state}");
        }
        return Ok(());
    }

    if machine_auth_token.trim().is_empty() {
        return Err(anyhow!(
            "auth_not_configured: set HEIWA_MACHINE_AUTH_TOKEN before calling the local app API"
        ));
    }
    let response = call_local_app_api(
        &method,
        path,
        port,
        body_payload.as_deref(),
        &machine_auth_token,
    )
    .await?;
    if response.status >= 400 {
        return Err(anyhow!(
            "app api {} {} returned HTTP {}: {}",
            method,
            path,
            response.status,
            response.body
        ));
    }
    println!("{}", response.body);
    Ok(())
}

fn read_api_body(raw: &str) -> Result<Value> {
    let body = if let Some(path) = raw.strip_prefix('@') {
        fs::read_to_string(path).map_err(|error| anyhow!("cannot read --body {raw}: {error}"))?
    } else {
        raw.to_string()
    };
    serde_json::from_str(&body).map_err(|error| anyhow!("--body must be JSON: {error}"))
}

struct ApiResponse {
    status: u16,
    body: String,
}

#[derive(Clone, Copy)]
struct LocalAppApiTransportPolicy {
    connect_timeout: Duration,
    write_timeout: Duration,
    read_timeout: Duration,
    max_response_bytes: usize,
}

// Local JSON API responses are operational control payloads, not bulk data.
// Two MiB leaves ample room for bounded event pages while limiting a hostile
// or malfunctioning loopback peer.
const LOCAL_APP_API_TRANSPORT_POLICY: LocalAppApiTransportPolicy = LocalAppApiTransportPolicy {
    connect_timeout: Duration::from_secs(3),
    write_timeout: Duration::from_secs(5),
    read_timeout: Duration::from_secs(15),
    max_response_bytes: 2 * 1024 * 1024,
};

async fn call_local_app_api(
    method: &str,
    path: &str,
    port: u16,
    body: Option<&str>,
    machine_auth_token: &str,
) -> Result<ApiResponse> {
    call_local_app_api_with_policy(
        method,
        path,
        port,
        body,
        machine_auth_token,
        LOCAL_APP_API_TRANSPORT_POLICY,
    )
    .await
}

async fn call_local_app_api_with_policy(
    method: &str,
    path: &str,
    port: u16,
    body: Option<&str>,
    machine_auth_token: &str,
    policy: LocalAppApiTransportPolicy,
) -> Result<ApiResponse> {
    let body = body.unwrap_or("");
    let signed = heiwa_core::auth::sign_local_request(
        heiwa_core::auth::LocalRequestParts {
            method,
            port,
            target: path,
            body: body.as_bytes(),
        },
        chrono::Utc::now().timestamp(),
        &uuid::Uuid::new_v4().simple().to_string(),
        machine_auth_token,
    )
    .map_err(|_| anyhow!("cannot sign local app API request"))?;
    let mut stream = time::timeout(
        policy.connect_timeout,
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map_err(|_| anyhow!("local app API connect timed out"))?
    .map_err(|error| anyhow!("cannot connect to Heiwa.app runtime on 127.0.0.1:{port}: {error}"))?;
    let request = if method == "POST" {
        format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nX-Heiwa-Local-Auth-Version: {}\r\nX-Heiwa-Local-Auth-Timestamp: {}\r\nX-Heiwa-Local-Auth-Nonce: {}\r\nX-Heiwa-Local-Auth-Signature: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            signed.version,
            signed.timestamp,
            signed.nonce,
            signed.signature,
            body.len()
        )
    } else {
        format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nX-Heiwa-Local-Auth-Version: {}\r\nX-Heiwa-Local-Auth-Timestamp: {}\r\nX-Heiwa-Local-Auth-Nonce: {}\r\nX-Heiwa-Local-Auth-Signature: {}\r\nConnection: close\r\n\r\n",
            signed.version, signed.timestamp, signed.nonce, signed.signature,
        )
    };
    time::timeout(policy.write_timeout, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| anyhow!("local app API write timed out"))?
        .map_err(|_| anyhow!("local app API write failed"))?;
    let raw = time::timeout(
        policy.read_timeout,
        read_capped_api_response(&mut stream, policy.max_response_bytes),
    )
    .await
    .map_err(|_| anyhow!("local app API read timed out"))??;
    parse_api_response(&raw)
}

async fn read_capped_api_response<R>(reader: &mut R, max_bytes: usize) -> Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|_| anyhow!("local app API read failed"))?;
        if read == 0 {
            return Ok(response);
        }
        if response.len().saturating_add(read) > max_bytes {
            return Err(anyhow!("local app API response too large"));
        }
        response.extend_from_slice(&buffer[..read]);
    }
}

fn parse_api_response(raw: &[u8]) -> Result<ApiResponse> {
    let Some(split) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err(anyhow!("app api response missing HTTP header separator"));
    };
    let head = String::from_utf8_lossy(&raw[..split]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("app api response missing HTTP status"))?;
    let body = String::from_utf8_lossy(&raw[split + 4..]).to_string();
    Ok(ApiResponse { status, body })
}

async fn start(args: &[String]) -> Result<()> {
    if has_flag(args, "--help") || has_flag(args, "-h") {
        print_start_help();
        return Ok(());
    }

    let port = parse_port(args)?;
    let no_open = has_flag(args, "--no-open");
    let install_path = env::current_exe().context("resolve current heiwa runtime path")?;
    heiwa_install::refresh_machine_manifest_for_runtime(heiwa_install::MachineRuntime {
        version: env!("CARGO_PKG_VERSION").to_string(),
        channel: runtime_channel(),
        install_path,
    })?;
    let worker_id = format!("heiwa-app-{}", std::process::id());
    let started_at = Arc::new(chrono::Utc::now().to_rfc3339());
    let runtime_state_dir = state_dir();
    let local_request_replays = Arc::new(Mutex::new(LocalRequestReplayCache::default()));
    let browser_sessions = Arc::new(Mutex::new(BrowserSessionStore::default()));

    // The port identifies this app instance; the runtime lease prevents two
    // app servers from sharing one evidence root. Session-service activity
    // leases separately prove exclusive recovery ownership across app, CLI,
    // REPL, and loop writers.
    let evidence_root = heiwa_evidence::journal_root()?;
    let _operator_runtime_lease =
        heiwa_session::operator::OperatorAppRuntimeLease::acquire(evidence_root)
            .map_err(|error| anyhow!(error))?;
    let sessions = crate::default_model_call_runtime()
        .map_err(anyhow::Error::msg)?
        .sessions;
    sessions
        .recover_interrupted()
        .map_err(|error| anyhow!("operator restart recovery failed: {error}"))?;

    // Port reachability is the readiness boundary used by launchers and
    // tests. Bind only after ownership and recovery finish so a successful
    // TCP connect cannot race the exclusive recovery lease.
    let bind_addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    let url = format!("http://127.0.0.1:{}/", local_addr.port());

    write_app_heartbeat(&runtime_state_dir, &worker_id)?;
    let mut caffeinate = spawn_caffeinate();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(heartbeat_loop(
        runtime_state_dir.clone(),
        worker_id.clone(),
        shutdown_rx.clone(),
    ));
    tokio::spawn(automation_loop(runtime_state_dir, shutdown_rx));

    if !no_open {
        let config = heiwa_core::config::RuntimeConfig::from_env();
        if config.machine_auth_token.trim().is_empty() {
            open_url(&url)?;
        } else {
            let bootstrap = browser_sessions
                .lock()
                .map_err(|_| anyhow!("browser session mutex poisoned"))?
                .issue_bootstrap_at(
                    chrono::Utc::now().timestamp(),
                    &format!("http://127.0.0.1:{}", local_addr.port()),
                );
            open_url(&format!("{url}?heiwa_bootstrap={bootstrap}"))?;
        }
    }

    println!("heiwa app start");
    println!("  url: {url}");
    println!("  worker_id: {worker_id}");
    println!(
        "  caffeinate: {}",
        caffeinate
            .as_ref()
            .map(|child| child.id().to_string())
            .unwrap_or_else(|| "not-started".to_string())
    );
    println!("  static: {}", cockpit_static_root().display());
    println!("  stop: SIGINT/SIGTERM");

    let signal = shutdown_signal();
    tokio::pin!(signal);

    loop {
        tokio::select! {
            _ = &mut signal => {
                break;
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let started_at = started_at.clone();
                let local_request_replays = local_request_replays.clone();
                let browser_sessions = browser_sessions.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(
                        stream,
                        started_at,
                        local_request_replays,
                        browser_sessions,
                    )
                    .await
                    {
                        eprintln!("heiwa app connection error: {err}");
                    }
                });
            }
        }
    }

    let _ = shutdown_tx.send(true);
    stop_caffeinate(&mut caffeinate);
    println!("heiwa app stopped");
    Ok(())
}

async fn heartbeat_loop(
    runtime_state_dir: PathBuf,
    worker_id: String,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ticker = time::interval(Duration::from_secs(60));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let _ = write_app_heartbeat(&runtime_state_dir, &worker_id);
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

async fn automation_loop(runtime_state_dir: PathBuf, mut shutdown: watch::Receiver<bool>) {
    let mut ticker = time::interval(Duration::from_secs(60));
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(error) = crate::cmd::auto::tick_and_execute_state_dir(
                    &runtime_state_dir,
                    chrono::Utc::now(),
                ).await {
                    eprintln!("heiwa automation tick error: {error:#}");
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn runtime_status(args: &[String]) -> Result<()> {
    let status = RuntimeStatus::detect();
    if has_flag(args, "--json") {
        println!(
            "{}",
            json!({
                "command": "app runtime status",
                "state": status.state,
                "node": status.node,
                "cli_path": status.cli_path.display().to_string(),
                "state_dir": status.state_dir.display().to_string(),
                "transport": status.transport,
                "sidecar": status.sidecar,
                "keep_awake": status.keep_awake,
                "policy": status.policy,
                "hooks": status.hooks_summary,
                "workers": status.workers_summary,
                "approvals": status.approvals_summary,
                "mail": status.mail_summary,
                "local_app": status.local_app,
                "next": status.next,
            })
        );
        return Ok(());
    }
    println!("heiwa app");
    println!("  command: app runtime status");
    println!("  state: {}", status.state);
    println!("  node: {}", status.node);
    println!("  cli: {}", status.cli_path.display());
    println!("  state_dir: {}", status.state_dir.display());
    println!("  transport: {}", status.transport);
    println!("  sidecar: {}", status.sidecar);
    println!("  keep_awake: {}", status.keep_awake);
    println!("  policy: {}", status.policy);
    println!(
        "  hooks: {} active / {} degraded / {} unconfigured",
        status
            .hooks_summary
            .get("active")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        status
            .hooks_summary
            .get("degraded")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        status
            .hooks_summary
            .get("unconfigured")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    );
    println!(
        "  workers: {} live / {} stale",
        status
            .workers_summary
            .get("live")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        status
            .workers_summary
            .get("stale")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    );
    println!(
        "  approvals: {} pending",
        status
            .approvals_summary
            .get("pending")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    );
    println!(
        "  mail: {} (policy: {})",
        status
            .mail_summary
            .get("bridge_state")
            .and_then(Value::as_str)
            .unwrap_or("unknown"),
        status
            .mail_summary
            .get("policy")
            .and_then(Value::as_str)
            .unwrap_or("metadata-only-no-body"),
    );
    println!(
        "  local_app: {} on {} ({})",
        if status.local_app.reachable {
            "reachable"
        } else {
            "unreachable"
        },
        status.local_app.url,
        status
            .local_app
            .latency_ms
            .map(|ms| format!("{ms}ms"))
            .unwrap_or_else(|| "not running".to_string()),
    );
    println!("  next: {}", status.next);
    Ok(())
}

struct RuntimeStatus {
    state: &'static str,
    node: String,
    cli_path: PathBuf,
    state_dir: PathBuf,
    transport: &'static str,
    sidecar: &'static str,
    keep_awake: String,
    policy: &'static str,
    next: &'static str,
    hooks_summary: Value,
    workers_summary: Value,
    approvals_summary: Value,
    mail_summary: Value,
    local_app: LocalAppProbe,
}

impl RuntimeStatus {
    fn detect() -> Self {
        let state_dir = state_dir();
        Self {
            state: "local_probe",
            node: hostname_string(),
            cli_path: env::current_exe().unwrap_or_else(|_| PathBuf::from("heiwa")),
            state_dir: state_dir.clone(),
            transport: "localhost-http-websocket-ready",
            sidecar: "start-with-heiwa-app-start",
            keep_awake: detect_keep_awake(),
            policy: "local-only-no-side-effects",
            next: "run heiwa app start --port 7474",
            hooks_summary: hooks_summary(),
            workers_summary: workers_summary(&state_dir),
            approvals_summary: approvals_summary(&state_dir),
            mail_summary: mail_summary(),
            local_app: probe_local_app(DEFAULT_PORT),
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    started_at: Arc<String>,
    local_request_replays: Arc<Mutex<LocalRequestReplayCache>>,
    browser_sessions: Arc<Mutex<BrowserSessionStore>>,
) -> Result<()> {
    let local_port = stream
        .local_addr()
        .map(|addr| addr.port())
        .unwrap_or(DEFAULT_PORT);
    let (request, body_bytes) = read_http_request_and_body(&mut stream).await?;
    if request.is_empty() {
        return Ok(());
    }

    if is_websocket_request(&request) {
        let target = request_target(&request).unwrap_or("/").to_string();
        let path = request_path(&request).unwrap_or("/").to_string();
        if path == "/ws/v1/operator" {
            if let Err(error) = operator_http_auth_subject(
                &request,
                "GET",
                &target,
                b"",
                local_port,
                &local_request_replays,
                &browser_sessions,
            ) {
                let (status, code) = operator_auth_response(error);
                return write_response(
                    &mut stream,
                    status,
                    "application/json",
                    json!({"ok": false, "error": {"code": code}})
                        .to_string()
                        .into_bytes(),
                    false,
                )
                .await;
            }
        }
        return handle_websocket(stream, &request, started_at, &path, &target).await;
    }

    let method = request_method(&request).unwrap_or("GET");
    let target = request_target(&request).unwrap_or("/");
    let path = request_path(&request).unwrap_or("/");
    let body = String::from_utf8_lossy(&body_bytes).to_string();
    if method == "GET" && path == "/" {
        if let Some(bootstrap) = query_param(target, "heiwa_bootstrap") {
            let origin = exact_loopback_origin(&request, local_port)
                .map_err(|_| anyhow!("invalid browser bootstrap origin"))?;
            let session = browser_sessions
                .lock()
                .map_err(|_| anyhow!("browser session mutex poisoned"))?
                .consume_bootstrap_at(&bootstrap, chrono::Utc::now().timestamp(), &origin);
            return match session {
                Some(session) => {
                    write_browser_session_redirect(&mut stream, &session, local_port).await
                }
                None => {
                    write_response(
                        &mut stream,
                        401,
                        "application/json",
                        json!({"ok": false, "error": {"code": "invalid_browser_bootstrap"}})
                            .to_string()
                            .into_bytes(),
                        false,
                    )
                    .await
                }
            };
        }
    }
    if method == "OPTIONS" {
        return write_response(&mut stream, 204, "text/plain", Vec::new(), false).await;
    }
    let head_only = method == "HEAD";

    if is_runtime_authenticated_request(method, path) {
        if let Err(error) = operator_http_auth_subject(
            &request,
            method,
            target,
            &body_bytes,
            local_port,
            &local_request_replays,
            &browser_sessions,
        ) {
            let (status, code) = operator_auth_response(error);
            return write_response(
                &mut stream,
                status,
                "application/json",
                json!({"ok": false, "error": {"code": code}})
                    .to_string()
                    .into_bytes(),
                false,
            )
            .await;
        }
    }

    if is_operator_api_path(path) {
        let (status, payload) = operator_http_response(method, target, path, &body).await;
        return write_response(
            &mut stream,
            status,
            "application/json",
            payload.to_string().into_bytes(),
            head_only,
        )
        .await;
    }

    if method == "POST" && path == "/api/v1/repl" {
        let parsed_body: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let prompt = parsed_body
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let payload = match crate::execute_repl_turn(&prompt).await {
            Ok((response, trace)) => {
                json!({
                    "ok": true,
                    "data": {
                        "response": response,
                        "trace": trace,
                    }
                })
            }
            Err(err) => {
                json!({
                    "ok": false,
                    "error": {
                        "code": "execution_failed",
                        "message": err,
                    }
                })
            }
        };

        return write_response(
            &mut stream,
            200,
            "application/json",
            payload.to_string().into_bytes(),
            false,
        )
        .await;
    }

    if method == "POST" && path == "/api/v1/repl/stream" {
        let parsed_body: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let prompt = parsed_body
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if prompt.trim().is_empty() {
            return write_response(
                &mut stream,
                400,
                "application/json",
                json!({"ok": false, "error": {"code": "empty_prompt"}})
                    .to_string()
                    .into_bytes(),
                false,
            )
            .await;
        }
        return serve_repl_stream(stream, prompt).await;
    }

    if method == "POST"
        && matches!(
            path,
            "/api/v1/connectors/apple_calendar/connect"
                | "/api/v1/connectors/apple_calendar/disconnect"
        )
    {
        let result = if path.ends_with("/connect") {
            crate::cmd::connectors::connect_apple_calendar()
        } else {
            crate::cmd::connectors::disconnect_apple_calendar()
        };
        let (status, payload) = match result {
            Ok(data) => (200, json!({"ok": true, "data": data})),
            Err(error) => (
                400,
                json!({"ok": false, "error": {"code": "connector_action_failed", "message": error.to_string()}}),
            ),
        };
        return write_response(
            &mut stream,
            status,
            "application/json",
            payload.to_string().into_bytes(),
            false,
        )
        .await;
    }

    if method == "POST" {
        let segments = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
        if let ["api", "v1", "approvals", request_id, action] = segments.as_slice() {
            let approve = match *action {
                "approve" | "grant" => Some(true),
                "deny" => Some(false),
                _ => None,
            };
            if let Some(approve) = approve {
                let parsed_body: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
                let note = parsed_body
                    .get("note")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let (status, payload) = match crate::cmd::approvals::decide_request(
                    request_id,
                    approve,
                    note,
                    "Heiwa.app",
                ) {
                    Ok(data) => (200, json!({"ok": true, "data": data})),
                    Err(error) => {
                        let message = error.to_string();
                        let status = if message.contains("already approved")
                            || message.contains("already denied")
                            || message.contains("already in progress")
                        {
                            409
                        } else {
                            400
                        };
                        (
                            status,
                            json!({"ok": false, "error": {"code": "approval_decision_failed", "message": message}}),
                        )
                    }
                };
                return write_response(
                    &mut stream,
                    status,
                    "application/json",
                    payload.to_string().into_bytes(),
                    false,
                )
                .await;
            }
        }
    }

    if method == "POST" && path == "/api/v1/calendar/holds" {
        let parsed_body: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let (status, payload) = match crate::cmd::calendar::create_hold_from_app(&parsed_body) {
            Ok(data) => (201, json!({"ok": true, "data": data})),
            Err(error) => (
                400,
                json!({"ok": false, "error": {"code": "invalid_hold", "message": error.to_string()}}),
            ),
        };
        return write_response(
            &mut stream,
            status,
            "application/json",
            payload.to_string().into_bytes(),
            false,
        )
        .await;
    }

    if method == "POST" && path == "/api/v1/route/preview" {
        let parsed_body: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let prompt = parsed_body
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if prompt.is_empty() {
            return write_response(
                &mut stream,
                400,
                "application/json",
                json!({"ok": false, "error": {"code": "empty_prompt"}})
                    .to_string()
                    .into_bytes(),
                false,
            )
            .await;
        }
        let payload = crate::preview_route_payload(&prompt).await;
        return write_response(
            &mut stream,
            200,
            "application/json",
            json!({"ok": true, "data": payload})
                .to_string()
                .into_bytes(),
            false,
        )
        .await;
    }

    if method == "POST" && path == "/api/v1/agents/dispatch" {
        let parsed_body: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
        let task_prompt = parsed_body
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if task_prompt.is_empty() {
            return write_response(
                &mut stream,
                400,
                "application/json",
                json!({"ok": false, "error": {"code": "empty_task"}})
                    .to_string()
                    .into_bytes(),
                false,
            )
            .await;
        }
        let provider = parsed_body
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .to_string();
        let model = parsed_body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("router-selected")
            .to_string();
        let task_id = format!("sa-{}", std::process::id());

        // Spawn background task
        tokio::spawn(async move {
            let _ = crate::execute_repl_turn(&task_prompt).await;
        });

        return write_response(
            &mut stream,
            202,
            "application/json",
            json!( {
                "ok": true,
                "data": {
                    "task_id": task_id,
                    "provider": provider,
                    "model": model,
                    "status": "accepted",
                }
            })
            .to_string()
            .into_bytes(),
            false,
        )
        .await;
    }

    if method != "GET" && !head_only {
        return write_response(
            &mut stream,
            405,
            "application/json",
            json!({"ok": false, "error": {"code": "method_not_allowed"}})
                .to_string()
                .into_bytes(),
            false,
        )
        .await;
    }

    if path == "/api/v1/files/tree" {
        let payload = match files_tree_payload_from_target(target) {
            Ok(data) => json!({"ok": true, "data": data}),
            Err(error) => {
                json!({"ok": false, "error": {"code": "files_tree_failed", "message": error.to_string()}})
            }
        };
        return write_response(
            &mut stream,
            if payload.get("ok").and_then(Value::as_bool) == Some(true) {
                200
            } else {
                400
            },
            "application/json",
            payload.to_string().into_bytes(),
            head_only,
        )
        .await;
    }

    if path == "/api/v1/files/preview" {
        let payload = match file_preview_payload_from_target(target) {
            Ok(data) => json!({"ok": true, "data": data}),
            Err(error) => {
                json!({"ok": false, "error": {"code": "file_preview_failed", "message": error.to_string()}})
            }
        };
        return write_response(
            &mut stream,
            if payload.get("ok").and_then(Value::as_bool) == Some(true) {
                200
            } else {
                400
            },
            "application/json",
            payload.to_string().into_bytes(),
            head_only,
        )
        .await;
    }

    if path == "/api/v1/browser/probe" {
        let payload = match browser_probe_payload_from_target(target) {
            Ok(data) => json!({"ok": true, "data": data}),
            Err(error) => {
                json!({"ok": false, "error": {"code": "browser_probe_failed", "message": error.to_string()}})
            }
        };
        return write_response(
            &mut stream,
            if payload.get("ok").and_then(Value::as_bool) == Some(true) {
                200
            } else {
                400
            },
            "application/json",
            payload.to_string().into_bytes(),
            head_only,
        )
        .await;
    }

    let payload_path = path.to_string();
    let payload_started_at = started_at.as_str().to_string();
    let payload = tokio::task::spawn_blocking(move || {
        api_payload_for_port(&payload_path, &payload_started_at, local_port)
    })
    .await
    .map_err(|error| anyhow!("local app payload task failed: {error}"))?;
    if let Some(payload) = payload {
        return write_response(
            &mut stream,
            200,
            "application/json",
            payload.to_string().into_bytes(),
            head_only,
        )
        .await;
    }

    serve_static(&mut stream, path, head_only).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperatorAuthError {
    NotConfigured,
    Unauthorized,
}

fn operator_auth_response(error: OperatorAuthError) -> (u16, &'static str) {
    match error {
        OperatorAuthError::NotConfigured => (500, "auth_not_configured"),
        OperatorAuthError::Unauthorized => (401, "unauthorized"),
    }
}

fn is_operator_api_path(path: &str) -> bool {
    path == "/api/v1/operator" || path.starts_with("/api/v1/operator/")
}

fn is_runtime_authenticated_request(method: &str, path: &str) -> bool {
    if is_operator_api_path(path) || matches!(path, "/api/v1/repl" | "/api/v1/repl/stream") {
        return true;
    }
    path.starts_with("/api/v1/")
        && matches!(method, "POST" | "PUT" | "PATCH" | "DELETE")
        && path != "/api/v1/route/preview"
}

const MAX_LOCAL_REQUEST_NONCES: usize = 4096;

#[derive(Default)]
struct BrowserSessionStore {
    bootstraps: HashMap<String, BrowserSessionGrant>,
    sessions: HashMap<String, BrowserSessionGrant>,
}

#[derive(Clone)]
struct BrowserSessionGrant {
    expires_at: i64,
    origin: String,
}

impl BrowserSessionStore {
    fn issue_bootstrap_at(&mut self, now: i64, origin: &str) -> String {
        self.retain_fresh(now);
        loop {
            let token = uuid::Uuid::new_v4().simple().to_string();
            if self
                .bootstraps
                .insert(
                    token.clone(),
                    BrowserSessionGrant {
                        expires_at: now.saturating_add(BROWSER_BOOTSTRAP_TTL_SECONDS),
                        origin: origin.to_string(),
                    },
                )
                .is_none()
            {
                return token;
            }
        }
    }

    fn consume_bootstrap_at(&mut self, bootstrap: &str, now: i64, origin: &str) -> Option<String> {
        self.retain_fresh(now);
        let grant = self.bootstraps.remove(bootstrap)?;
        if grant.expires_at < now || grant.origin != origin {
            return None;
        }
        loop {
            let session = uuid::Uuid::new_v4().simple().to_string();
            if self
                .sessions
                .insert(
                    session.clone(),
                    BrowserSessionGrant {
                        expires_at: now.saturating_add(BROWSER_SESSION_TTL_SECONDS),
                        origin: origin.to_string(),
                    },
                )
                .is_none()
            {
                return Some(session);
            }
        }
    }

    fn authenticates_cookie_at(
        &mut self,
        cookie_header: &str,
        now: i64,
        port: u16,
        origin: &str,
    ) -> bool {
        self.retain_fresh(now);
        let expected_name = browser_session_cookie_name(port);
        cookie_header.split(';').map(str::trim).any(|segment| {
            let Some((name, value)) = segment.split_once('=') else {
                return false;
            };
            name == expected_name
                && self
                    .sessions
                    .get(value)
                    .is_some_and(|grant| grant.expires_at >= now && grant.origin == origin)
        })
    }

    fn retain_fresh(&mut self, now: i64) {
        self.bootstraps.retain(|_, grant| grant.expires_at >= now);
        self.sessions.retain(|_, grant| grant.expires_at >= now);
    }
}

fn browser_session_cookie_name(port: u16) -> String {
    format!("{BROWSER_SESSION_COOKIE_PREFIX}{port}")
}

#[derive(Default)]
struct LocalRequestReplayCache {
    nonces: HashMap<String, i64>,
}

impl LocalRequestReplayCache {
    fn consume(
        &mut self,
        verified: heiwa_core::auth::VerifiedLocalRequest,
        now: i64,
    ) -> std::result::Result<(), ()> {
        self.nonces.retain(|_, timestamp| {
            timestamp.abs_diff(now) <= heiwa_core::auth::LOCAL_REQUEST_MAX_SKEW_SECONDS
        });
        if self.nonces.contains_key(&verified.nonce)
            || self.nonces.len() >= MAX_LOCAL_REQUEST_NONCES
        {
            return Err(());
        }
        self.nonces.insert(verified.nonce, verified.timestamp);
        Ok(())
    }
}

fn operator_http_auth_subject(
    request: &str,
    method: &str,
    target: &str,
    body: &[u8],
    local_port: u16,
    local_request_replays: &Mutex<LocalRequestReplayCache>,
    browser_sessions: &Mutex<BrowserSessionStore>,
) -> std::result::Result<heiwa_core::auth::AuthSubject, OperatorAuthError> {
    let config = heiwa_core::config::RuntimeConfig::from_env();
    if config.machine_auth_token.trim().is_empty() && config.jwt_signing_secret.trim().is_empty() {
        return Err(OperatorAuthError::NotConfigured);
    }
    let expected_origin = exact_loopback_origin(request, local_port)?;
    let supplied_origin =
        strict_header_value(request, "origin").map_err(|_| OperatorAuthError::Unauthorized)?;
    if supplied_origin
        .as_deref()
        .is_some_and(|origin| origin != expected_origin)
    {
        return Err(OperatorAuthError::Unauthorized);
    }
    let cookie =
        strict_header_value(request, "cookie").map_err(|_| OperatorAuthError::Unauthorized)?;
    if cookie.is_some_and(|cookie| {
        supplied_origin.as_deref() == Some(expected_origin.as_str())
            && browser_sessions.lock().ok().is_some_and(|mut sessions| {
                sessions.authenticates_cookie_at(
                    &cookie,
                    chrono::Utc::now().timestamp(),
                    local_port,
                    &expected_origin,
                )
            })
    }) {
        return Ok(heiwa_core::auth::AuthSubject::Operator);
    }
    match local_request_signature_headers(request) {
        Ok(Some(signed)) => {
            if config.machine_auth_token.trim().is_empty() {
                return Err(OperatorAuthError::Unauthorized);
            }
            let now = chrono::Utc::now().timestamp();
            let verified = heiwa_core::auth::verify_local_request(
                heiwa_core::auth::LocalRequestParts {
                    method,
                    port: local_port,
                    target,
                    body,
                },
                &signed,
                &config.machine_auth_token,
                now,
            )
            .map_err(|_| OperatorAuthError::Unauthorized)?;
            local_request_replays
                .lock()
                .map_err(|_| OperatorAuthError::Unauthorized)?
                .consume(verified, now)
                .map_err(|_| OperatorAuthError::Unauthorized)?;
            return Ok(heiwa_core::auth::AuthSubject::Operator);
        }
        Ok(None) => {}
        Err(()) => return Err(OperatorAuthError::Unauthorized),
    }

    let cookie = header_value(request, "cookie");
    let authorization = header_value(request, "authorization");
    heiwa_core::auth::extract_auth_subject(cookie.as_deref(), authorization.as_deref(), &config)
        .map_err(|_| OperatorAuthError::Unauthorized)
}

fn exact_loopback_origin(
    request: &str,
    local_port: u16,
) -> std::result::Result<String, OperatorAuthError> {
    let host = strict_header_value(request, "host")
        .map_err(|_| OperatorAuthError::Unauthorized)?
        .ok_or(OperatorAuthError::Unauthorized)?;
    let expected_host = format!("127.0.0.1:{local_port}");
    if host != expected_host {
        return Err(OperatorAuthError::Unauthorized);
    }
    Ok(format!("http://{expected_host}"))
}

fn local_request_signature_headers(
    request: &str,
) -> std::result::Result<Option<heiwa_core::auth::LocalRequestSignature>, ()> {
    let names = [
        heiwa_core::auth::LOCAL_REQUEST_AUTH_VERSION_HEADER,
        heiwa_core::auth::LOCAL_REQUEST_AUTH_TIMESTAMP_HEADER,
        heiwa_core::auth::LOCAL_REQUEST_AUTH_NONCE_HEADER,
        heiwa_core::auth::LOCAL_REQUEST_AUTH_SIGNATURE_HEADER,
    ];
    let values = names
        .iter()
        .map(|name| strict_header_value(request, name))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    if values.iter().any(Option::is_none) {
        return Err(());
    }
    Ok(Some(heiwa_core::auth::LocalRequestSignature {
        version: values[0].clone().ok_or(())?,
        timestamp: values[1].clone().ok_or(())?,
        nonce: values[2].clone().ok_or(())?,
        signature: values[3].clone().ok_or(())?,
    }))
}

fn strict_header_value(request: &str, name: &str) -> std::result::Result<Option<String>, ()> {
    let values = request
        .lines()
        .filter_map(|line| {
            let (header_name, value) = line.split_once(':')?;
            header_name
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
        .collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [value] => Ok(Some(value.clone())),
        _ => Err(()),
    }
}

enum OperatorHttpRoute {
    Threads,
    Thread(String),
    Events(String),
    Turns(String),
    Cancel(String),
}

async fn operator_http_response(
    method: &str,
    target: &str,
    path: &str,
    body: &str,
) -> (u16, Value) {
    let route = match parse_operator_route(path) {
        Ok(Some(route)) => route,
        Ok(None) => return operator_error(404, "not_found"),
        Err(()) => return operator_error(400, "invalid_id"),
    };
    let runtime = match crate::default_model_call_runtime() {
        Ok(runtime) => runtime,
        Err(_) => return operator_error(503, "operator_unavailable"),
    };
    let sessions = runtime.sessions;
    let runner = runtime.runner;

    match (method, route) {
        ("GET", OperatorHttpRoute::Threads) => match sessions.list_threads(100) {
            Ok(threads) => (200, json!({"ok": true, "data": {"threads": threads}})),
            Err(_) => operator_error(503, "operator_unavailable"),
        },
        ("POST", OperatorHttpRoute::Threads) => {
            let parsed = match parse_json_body(body) {
                Ok(parsed) => parsed,
                Err(()) => return operator_error(400, "invalid_request"),
            };
            let raw_id = match parsed {
                Value::Null => format!("thread-{}", uuid::Uuid::new_v4()),
                Value::Object(object) => match object.get("thread_id") {
                    None => format!("thread-{}", uuid::Uuid::new_v4()),
                    Some(Value::String(thread_id)) => thread_id.clone(),
                    Some(_) => return operator_error(400, "invalid_request"),
                },
                _ => return operator_error(400, "invalid_request"),
            };
            let thread_id = match validate_operator_identifier(&raw_id) {
                Ok(thread_id) => thread_id,
                Err(()) => return operator_error(400, "invalid_id"),
            };
            let created = match sessions.ensure_thread(&thread_id) {
                Ok(created) => created,
                Err(_) => return operator_error(503, "operator_unavailable"),
            };
            match sessions.thread(&thread_id) {
                Ok(thread) => (
                    200,
                    json!({"ok": true, "data": {"thread_id": thread_id, "created": created, "thread": thread}}),
                ),
                Err(_) => operator_error(503, "operator_unavailable"),
            }
        }
        ("GET", OperatorHttpRoute::Thread(thread_id)) => match sessions.thread(&thread_id) {
            Ok(thread) => (200, json!({"ok": true, "data": {"thread": thread}})),
            Err(_) => operator_error(503, "operator_unavailable"),
        },
        ("GET", OperatorHttpRoute::Events(thread_id)) => {
            let limit = match query_param(target, "limit") {
                Some(raw) => match raw.parse::<usize>() {
                    Ok(limit) if (1..=500).contains(&limit) => limit,
                    _ => return operator_error(400, "invalid_request"),
                },
                None => 100,
            };
            let after = query_param(target, "after").filter(|cursor| !cursor.is_empty());
            match sessions.events_after(&thread_id, after.as_deref(), limit) {
                Ok(page) => {
                    let events = page
                        .events
                        .into_iter()
                        .map(|row| json!({"cursor": row.cursor, "event": row.event}))
                        .collect::<Vec<_>>();
                    (
                        200,
                        json!({
                            "ok": true,
                            "data": {
                                "events": events,
                                "next_cursor": page.next_cursor,
                                "skipped_lines": page.skipped_lines,
                            }
                        }),
                    )
                }
                Err(heiwa_evidence::CursorError::InvalidCursor { .. }) => {
                    operator_error(400, "invalid_cursor")
                }
                Err(
                    heiwa_evidence::CursorError::UnstableLineage { .. }
                    | heiwa_evidence::CursorError::Storage(_),
                ) => operator_error(503, "operator_unavailable"),
            }
        }
        ("POST", OperatorHttpRoute::Turns(thread_id)) => {
            let request = match parse_turn_request(body) {
                Ok(request) => request,
                Err(()) => return operator_error(400, "invalid_request"),
            };
            match crate::submit_operator_turn(&thread_id, request).await {
                Ok(handle) => {
                    let stream_url = format!(
                        "/ws/v1/operator?thread_id={}&after={}",
                        percent_encode_query_component(&handle.thread_id),
                        percent_encode_query_component(&handle.cursor)
                    );
                    (
                        202,
                        json!({
                            "ok": true,
                            "data": {
                                "thread_id": handle.thread_id,
                                "turn_id": handle.turn_id,
                                "cursor": handle.cursor,
                                "duplicate": handle.duplicate,
                                "stream_url": stream_url,
                            }
                        }),
                    )
                }
                Err(heiwa_shell::operator::OperatorSubmissionError::Rejected(
                    heiwa_session::operator::TurnSubmissionError::IdempotencyConflict { .. },
                )) => operator_error(409, "idempotency_conflict"),
                Err(heiwa_shell::operator::OperatorSubmissionError::Rejected(
                    heiwa_session::operator::TurnSubmissionError::SensitiveMaterial { .. },
                )) => operator_error(400, "sensitive_material"),
                Err(heiwa_shell::operator::OperatorSubmissionError::Rejected(
                    heiwa_session::operator::TurnSubmissionError::InvalidWorkScope { .. },
                )) => operator_error(409, "invalid_work_scope"),
                Err(heiwa_shell::operator::OperatorSubmissionError::Rejected(
                    heiwa_session::operator::TurnSubmissionError::Runtime(_),
                ))
                | Err(heiwa_shell::operator::OperatorSubmissionError::Runtime(_)) => {
                    operator_error(503, "operator_unavailable")
                }
            }
        }
        ("POST", OperatorHttpRoute::Cancel(turn_id)) => match runner.request_cancel(&turn_id) {
            Ok(true) => (
                202,
                json!({"ok": true, "data": {"turn_id": turn_id, "cancel_requested": true}}),
            ),
            Ok(false) => (
                200,
                json!({"ok": true, "data": {"turn_id": turn_id, "cancel_requested": false}}),
            ),
            Err(_) => operator_error(503, "operator_unavailable"),
        },
        _ => operator_error(405, "method_not_allowed"),
    }
}

fn operator_error(status: u16, code: &str) -> (u16, Value) {
    (status, json!({"ok": false, "error": {"code": code}}))
}

fn parse_operator_route(path: &str) -> std::result::Result<Option<OperatorHttpRoute>, ()> {
    let segments = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
    if segments.get(..3) != Some(&["api", "v1", "operator"][..]) {
        return Ok(None);
    }
    match segments.as_slice() {
        ["api", "v1", "operator", "threads"] => Ok(Some(OperatorHttpRoute::Threads)),
        ["api", "v1", "operator", "threads", thread_id] => Ok(Some(OperatorHttpRoute::Thread(
            decode_operator_path_id(thread_id)?,
        ))),
        ["api", "v1", "operator", "threads", thread_id, "events"] => Ok(Some(
            OperatorHttpRoute::Events(decode_operator_path_id(thread_id)?),
        )),
        ["api", "v1", "operator", "threads", thread_id, "turns"] => Ok(Some(
            OperatorHttpRoute::Turns(decode_operator_path_id(thread_id)?),
        )),
        ["api", "v1", "operator", "turns", turn_id, "cancel"] => Ok(Some(
            OperatorHttpRoute::Cancel(decode_operator_path_id(turn_id)?),
        )),
        _ if segments.iter().any(|segment| segment.is_empty()) => Err(()),
        _ => Ok(None),
    }
}

fn decode_operator_path_id(raw: &str) -> std::result::Result<String, ()> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(());
            }
            let hi = hex_value(bytes[index + 1]).ok_or(())?;
            let lo = hex_value(bytes[index + 2]).ok_or(())?;
            decoded.push((hi << 4) | lo);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(decoded).map_err(|_| ())?;
    validate_operator_identifier(&decoded)
}

fn percent_encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

fn validate_operator_identifier(raw: &str) -> std::result::Result<String, ()> {
    let id = validate_operator_identifier_shape(raw)?;
    if heiwa_evidence::find_sensitive(&Value::String(id.clone())).is_some() {
        return Err(());
    }
    Ok(id)
}

fn validate_operator_identifier_shape(raw: &str) -> std::result::Result<String, ()> {
    let id = raw.trim();
    if id.is_empty()
        || id.len() > 128
        || id.contains("..")
        || id.contains('/')
        || id.contains('\\')
        || id.chars().any(char::is_control)
    {
        return Err(());
    }
    Ok(id.to_string())
}

fn parse_json_body(body: &str) -> std::result::Result<Value, ()> {
    if body.trim().is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_str(body).map_err(|_| ())
    }
}

fn parse_turn_request(
    body: &str,
) -> std::result::Result<heiwa_session::operator::StartTurnRequest, ()> {
    let value = parse_json_body(body)?;
    let object = value.as_object().ok_or(())?;
    let client_request_id = object
        .get("client_request_id")
        .and_then(Value::as_str)
        .ok_or(())?;
    // Client request IDs are not path components. Validate their shape here,
    // then let the typed session admission gate classify sensitive material.
    let client_request_id = validate_operator_identifier_shape(client_request_id)?;
    let prompt = object.get("prompt").and_then(Value::as_str).ok_or(())?;
    if prompt.trim().is_empty() || prompt.len() > 64 * 1024 {
        return Err(());
    }

    let mut request = heiwa_session::operator::StartTurnRequest::auto(client_request_id, prompt);
    if let Some(work_id) = object.get("work_id").filter(|value| !value.is_null()) {
        let work_id = work_id.as_str().ok_or(())?;
        request.work_id = Some(validate_operator_identifier(work_id)?);
    }
    if let Some(policy) = object.get("route_policy").filter(|value| !value.is_null()) {
        request.route_policy = parse_route_policy(policy)?;
    }
    Ok(request)
}

fn parse_route_policy(
    value: &Value,
) -> std::result::Result<heiwa_session::operator::TurnRoutePolicy, ()> {
    use heiwa_session::operator::{RouteMode, TurnRoutePolicy};

    let object = value.as_object().ok_or(())?;
    let mode = match object.get("mode") {
        None => RouteMode::Auto,
        Some(Value::String(mode)) => match mode.as_str() {
            "auto" => RouteMode::Auto,
            "local_only" => RouteMode::LocalOnly,
            "remote_only" => RouteMode::RemoteOnly,
            "explicit" => RouteMode::Explicit,
            _ => return Err(()),
        },
        Some(_) => return Err(()),
    };
    let preferred_provider = parse_optional_policy_string(object.get("preferred_provider"))?;
    let preferred_model = parse_optional_policy_string(object.get("preferred_model"))?;
    let allowed_models = parse_policy_string_list(object.get("allowed_models"))?;
    let excluded_models = parse_policy_string_list(object.get("excluded_models"))?;
    let minimum_quality_class = match object.get("minimum_quality_class") {
        Some(value) => {
            let quality = value.as_u64().ok_or(())?;
            if !(1..=5).contains(&quality) {
                return Err(());
            }
            quality as u8
        }
        None => 1,
    };
    let maximum_marginal_cost_usd =
        parse_nonnegative_budget(object.get("maximum_marginal_cost_usd"))?;
    let turn_budget_usd = parse_nonnegative_budget(object.get("turn_budget_usd"))?;
    let privacy = match object.get("privacy") {
        None => "standard",
        Some(Value::String(privacy)) => privacy.as_str(),
        Some(_) => return Err(()),
    };
    heiwa_core::drex::PrivacyClass::parse(privacy).map_err(|_| ())?;
    if mode == RouteMode::Explicit && preferred_provider.is_none() && preferred_model.is_none() {
        return Err(());
    }
    Ok(TurnRoutePolicy {
        mode,
        preferred_provider,
        preferred_model,
        allowed_models,
        excluded_models,
        minimum_quality_class,
        maximum_marginal_cost_usd,
        turn_budget_usd,
        privacy: privacy.to_string(),
    })
}

fn parse_optional_policy_string(value: Option<&Value>) -> std::result::Result<Option<String>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() && value.len() <= 256 => {
            Ok(Some(value.trim().to_string()))
        }
        _ => Err(()),
    }
}

fn parse_policy_string_list(value: Option<&Value>) -> std::result::Result<Vec<String>, ()> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or(())?;
    if values.len() > 64 {
        return Err(());
    }
    values
        .iter()
        .map(|value| {
            let value = value.as_str().ok_or(())?;
            if value.trim().is_empty() || value.len() > 256 {
                Err(())
            } else {
                Ok(value.trim().to_string())
            }
        })
        .collect()
}

fn parse_nonnegative_budget(value: Option<&Value>) -> std::result::Result<Option<f64>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let value = value.as_f64().ok_or(())?;
            if value.is_finite() && value >= 0.0 {
                Ok(Some(value))
            } else {
                Err(())
            }
        }
    }
}

async fn read_http_request_and_body(stream: &mut TcpStream) -> Result<(String, Vec<u8>)> {
    const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
    const MAX_HTTP_BODY_BYTES: usize = 10 * 1024 * 1024;

    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    let mut headers_len = None;

    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let end = pos + 4;
            if end > MAX_HTTP_HEADER_BYTES {
                return Err(anyhow!("request headers too large"));
            }
            headers_len = Some(end);
            break;
        }
        if data.len() > MAX_HTTP_HEADER_BYTES {
            return Err(anyhow!("request headers too large"));
        }
    }

    let headers_len = match headers_len {
        Some(len) => len,
        None => return Err(anyhow!("missing http headers separator")),
    };

    let headers_str = String::from_utf8_lossy(&data[..headers_len]).to_string();

    let content_lengths = headers_str
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim())
        })
        .collect::<Vec<_>>();
    let content_length = match content_lengths.as_slice() {
        [] => 0,
        [raw] if !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()) => raw
            .parse::<usize>()
            .map_err(|_| anyhow!("invalid content-length"))?,
        [_] => return Err(anyhow!("invalid content-length")),
        _ => return Err(anyhow!("multiple content-length headers")),
    };
    let total_len = headers_len
        .checked_add(content_length)
        .ok_or_else(|| anyhow!("request length overflow"))?;
    if content_length > MAX_HTTP_BODY_BYTES {
        return Err(anyhow!("request body too large"));
    }
    while data.len() < total_len {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(anyhow!(
                "truncated request body: expected {content_length} bytes"
            ));
        }
        data.extend_from_slice(&buf[..n]);
    }

    let body = data
        .get(headers_len..total_len)
        .ok_or_else(|| anyhow!("truncated request body"))?;
    Ok((headers_str, body.to_vec()))
}

async fn handle_websocket(
    mut stream: TcpStream,
    request: &str,
    started_at: Arc<String>,
    path: &str,
    target: &str,
) -> Result<()> {
    let key = header_value(request, "sec-websocket-key")
        .ok_or_else(|| anyhow!("missing websocket key"))?;
    let accept = websocket_accept_key(&key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\
         \r\n"
    );
    stream.write_all(response.as_bytes()).await?;

    if path == "/ws/v1/events" {
        return events_loop(stream).await;
    }

    if path == "/ws/v1/operator" {
        let params = match parse_operator_websocket_request(target) {
            Ok(params) => params,
            Err(()) => {
                let payload = json!({"type":"error","code":"invalid_request"});
                let _ = write_ws_text(&mut stream, &payload.to_string()).await;
                return Ok(());
            }
        };
        let runtime = match crate::default_model_call_runtime() {
            Ok(runtime) => runtime,
            Err(_) => {
                let payload = json!({"type":"error","code":"operator_unavailable"});
                let _ = write_ws_text(&mut stream, &payload.to_string()).await;
                return Ok(());
            }
        };
        let sessions = runtime.sessions;
        let runner = runtime.runner;
        return operator_events_loop(
            stream,
            sessions,
            runner.subscribe(),
            params.thread_id,
            params.after,
            OperatorWebsocketIntervals::default(),
        )
        .await;
    }

    let mut ticker = time::interval(Duration::from_secs(5));
    loop {
        ticker.tick().await;
        let payload = json!({
            "type": "runtime_snapshot",
            "data": snapshot(&started_at),
        });
        if write_ws_text(&mut stream, &payload.to_string())
            .await
            .is_err()
        {
            break;
        }
    }
    Ok(())
}

struct OperatorWebsocketRequest {
    thread_id: String,
    after: Option<String>,
}

#[derive(Clone, Copy)]
struct OperatorWebsocketIntervals {
    poll: Duration,
    heartbeat: Duration,
    write_timeout: Duration,
}

impl Default for OperatorWebsocketIntervals {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(200),
            heartbeat: Duration::from_secs(30),
            write_timeout: Duration::from_secs(10),
        }
    }
}

fn parse_operator_websocket_request(
    target: &str,
) -> std::result::Result<OperatorWebsocketRequest, ()> {
    let (path, query) = target.split_once('?').ok_or(())?;
    if path != "/ws/v1/operator" {
        return Err(());
    }
    let mut thread_id = None;
    let mut after = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = strict_percent_decode_query(raw_key)?;
        let value = strict_percent_decode_query(raw_value)?;
        match key.as_str() {
            "thread_id" if thread_id.is_none() => thread_id = Some(value),
            "after" if after.is_none() => after = Some(value),
            _ => return Err(()),
        }
    }
    let thread_id = validate_operator_identifier(&thread_id.ok_or(())?)?;
    let after = after.filter(|cursor| !cursor.is_empty());
    if after
        .as_ref()
        .is_some_and(|cursor| cursor.len() > 8 * 1024 || cursor.chars().any(char::is_control))
    {
        return Err(());
    }
    Ok(OperatorWebsocketRequest { thread_id, after })
}

fn strict_percent_decode_query(raw: &str) -> std::result::Result<String, ()> {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(());
                }
                let hi = hex_value(bytes[index + 1]).ok_or(())?;
                let lo = hex_value(bytes[index + 2]).ok_or(())?;
                decoded.push((hi << 4) | lo);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).map_err(|_| ())
}

async fn operator_events_loop(
    stream: TcpStream,
    sessions: Arc<heiwa_session::operator::OperatorSessionService>,
    mut transient: broadcast::Receiver<heiwa_shell::operator::OperatorStreamFrame>,
    thread_id: String,
    mut cursor: Option<String>,
    intervals: OperatorWebsocketIntervals,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let (control_tx, mut control_rx) = mpsc::channel(8);
    let _reader = AbortTask(tokio::spawn(read_operator_websocket_controls(
        reader, control_tx,
    )));
    let mut poll = time::interval(intervals.poll);
    poll.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut heartbeat = time::interval(intervals.heartbeat);
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    // `interval` ticks immediately. Consume that heartbeat tick so the first
    // heartbeat observes the configured delay while replay starts at once.
    heartbeat.tick().await;
    let mut caught_up = false;
    let mut transient_open = true;

    loop {
        tokio::select! {
            _ = poll.tick() => {
                let page = match sessions.events_after(&thread_id, cursor.as_deref(), 100) {
                    Ok(page) => page,
                    Err(heiwa_evidence::CursorError::InvalidCursor { .. }) => {
                        let payload = json!({
                            "type": "invalid_cursor",
                            "code": "invalid_cursor",
                            "action": "replay_from_start",
                        });
                        let _ = write_operator_ws_text(
                            &mut writer,
                            &payload.to_string(),
                            intervals.write_timeout,
                        )
                        .await;
                        return Ok(());
                    }
                    Err(
                        heiwa_evidence::CursorError::UnstableLineage { .. }
                        | heiwa_evidence::CursorError::Storage(_),
                    ) => {
                        let payload = json!({"type":"error","code":"operator_unavailable"});
                        let _ = write_operator_ws_text(
                            &mut writer,
                            &payload.to_string(),
                            intervals.write_timeout,
                        )
                        .await;
                        return Ok(());
                    }
                };
                let event_count = page.events.len();
                for row in page.events {
                    cursor = Some(row.cursor.clone());
                    let payload = json!({
                        "type": "event",
                        "cursor": row.cursor,
                        "event": row.event,
                    });
                    if write_operator_ws_text(&mut writer, &payload.to_string(), intervals.write_timeout).await.is_err() {
                        return Ok(());
                    }
                }
                if let Some(next_cursor) = page.next_cursor {
                    cursor = Some(next_cursor);
                }
                if !caught_up && event_count < 100 {
                    if write_operator_ws_text(&mut writer, &json!({"type":"caught_up"}).to_string(), intervals.write_timeout).await.is_err() {
                        return Ok(());
                    }
                    caught_up = true;
                }
            }
            frame = transient.recv(), if transient_open => {
                match frame {
                    Ok(heiwa_shell::operator::OperatorStreamFrame::AssistantDelta {
                        thread_id: frame_thread,
                        turn_id,
                        text,
                    }) if frame_thread == thread_id => {
                        let payload = json!({
                            "type":"assistant_delta",
                            "thread_id": frame_thread,
                            "turn_id": turn_id,
                            "text": text,
                        });
                        if write_operator_ws_text(&mut writer, &payload.to_string(), intervals.write_timeout).await.is_err() {
                            return Ok(());
                        }
                    }
                    Ok(heiwa_shell::operator::OperatorStreamFrame::Error {
                        thread_id: frame_thread,
                        turn_id,
                        ..
                    }) if frame_thread == thread_id => {
                        let payload = json!({
                            "type":"error",
                            "code":"execution_failed",
                            "thread_id": frame_thread,
                            "turn_id": turn_id,
                        });
                        if write_operator_ws_text(&mut writer, &payload.to_string(), intervals.write_timeout).await.is_err() {
                            return Ok(());
                        }
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => transient_open = false,
                }
            }
            _ = heartbeat.tick() => {
                let payload = json!({
                    "type":"heartbeat",
                    "occurred_at": chrono::Utc::now().to_rfc3339(),
                });
                if write_operator_ws_text(&mut writer, &payload.to_string(), intervals.write_timeout).await.is_err() {
                    return Ok(());
                }
            }
            control = control_rx.recv() => {
                match control {
                    Some(OperatorWebsocketControl::Ping(payload)) => {
                        if write_operator_ws_control(&mut writer, 0xA, &payload, intervals.write_timeout).await.is_err() {
                            return Ok(());
                        }
                    }
                    Some(OperatorWebsocketControl::Close(payload)) => {
                        let _ = write_operator_ws_control(&mut writer, 0x8, &payload, intervals.write_timeout).await;
                        return Ok(());
                    }
                    Some(OperatorWebsocketControl::ProtocolError) => {
                        let _ = write_operator_ws_control(
                            &mut writer,
                            0x8,
                            &1002_u16.to_be_bytes(),
                            intervals.write_timeout,
                        )
                        .await;
                        return Ok(());
                    }
                    None => return Ok(()),
                }
            }
        }
    }
}

enum OperatorWebsocketControl {
    Ping(Vec<u8>),
    Close(Vec<u8>),
    ProtocolError,
}

struct AbortTask(tokio::task::JoinHandle<()>);

impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn read_operator_websocket_controls(
    mut reader: tokio::net::tcp::OwnedReadHalf,
    controls: mpsc::Sender<OperatorWebsocketControl>,
) {
    const MAX_CLIENT_FRAME_BYTES: usize = 64 * 1024;

    loop {
        let mut header = [0_u8; 2];
        if reader.read_exact(&mut header).await.is_err() {
            let _ = controls
                .send(OperatorWebsocketControl::Close(Vec::new()))
                .await;
            return;
        }
        let fin = header[0] & 0x80 != 0;
        let reserved = header[0] & 0x70;
        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let short_len = header[1] & 0x7f;
        let is_control = opcode & 0x08 != 0;
        if reserved != 0 || !masked || (is_control && (!fin || short_len > 125)) {
            let _ = controls.send(OperatorWebsocketControl::ProtocolError).await;
            return;
        }

        let payload_len = match short_len {
            value @ 0..=125 => value as usize,
            126 => {
                let mut bytes = [0_u8; 2];
                if reader.read_exact(&mut bytes).await.is_err() {
                    return;
                }
                u16::from_be_bytes(bytes) as usize
            }
            127 => {
                let mut bytes = [0_u8; 8];
                if reader.read_exact(&mut bytes).await.is_err() {
                    return;
                }
                match usize::try_from(u64::from_be_bytes(bytes)) {
                    Ok(length) => length,
                    Err(_) => {
                        let _ = controls.send(OperatorWebsocketControl::ProtocolError).await;
                        return;
                    }
                }
            }
            _ => unreachable!(),
        };
        if payload_len > MAX_CLIENT_FRAME_BYTES {
            let _ = controls.send(OperatorWebsocketControl::ProtocolError).await;
            return;
        }

        let mut mask = [0_u8; 4];
        if reader.read_exact(&mut mask).await.is_err() {
            return;
        }
        let mut payload = vec![0_u8; payload_len];
        if reader.read_exact(&mut payload).await.is_err() {
            return;
        }
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % mask.len()];
        }

        let control = match opcode {
            0x8 if payload.len() != 1 => OperatorWebsocketControl::Close(payload),
            0x9 => OperatorWebsocketControl::Ping(payload),
            0xA | 0x0 | 0x1 | 0x2 => continue,
            _ => OperatorWebsocketControl::ProtocolError,
        };
        let terminal = matches!(
            control,
            OperatorWebsocketControl::Close(_) | OperatorWebsocketControl::ProtocolError
        );
        if controls.send(control).await.is_err() || terminal {
            return;
        }
    }
}

async fn events_loop(mut stream: TcpStream) -> Result<()> {
    let mut last_pending: HashSet<String> = HashSet::new();
    let mut last_decided: HashSet<String> = HashSet::new();
    let mut last_goals_fingerprint: HashSet<(String, u64)> = HashSet::new();
    let mut first = true;
    let mut heartbeat_counter: u32 = 0;
    let mut ticker = time::interval(Duration::from_secs(2));

    loop {
        ticker.tick().await;
        let pending = scan_dispatch_ids("requests");
        let decided = scan_dispatch_ids("approvals/decisions");
        let goals_fp = scan_goals_fingerprint();
        let ts = chrono::Utc::now().to_rfc3339();

        if first {
            let payload = json!({
                "event": "events_initial",
                "ts_utc": ts,
                "scope": "approvals",
                "payload": {
                    "pending_count": pending.len(),
                    "decided_count": decided.len(),
                    "goals_count": goals_fp.len(),
                }
            });
            if write_ws_text(&mut stream, &payload.to_string())
                .await
                .is_err()
            {
                return Ok(());
            }
            first = false;
        } else {
            let mut emitted = false;
            for id in pending.difference(&last_pending) {
                let payload = json!({
                    "event": "dispatch_request_appeared",
                    "ts_utc": ts,
                    "scope": "approvals",
                    "payload": { "id": id }
                });
                if write_ws_text(&mut stream, &payload.to_string())
                    .await
                    .is_err()
                {
                    return Ok(());
                }
                emitted = true;
            }
            for id in decided.difference(&last_decided) {
                let payload = json!({
                    "event": "dispatch_request_decided",
                    "ts_utc": ts,
                    "scope": "approvals",
                    "payload": { "id": id }
                });
                if write_ws_text(&mut stream, &payload.to_string())
                    .await
                    .is_err()
                {
                    return Ok(());
                }
                emitted = true;
            }
            if goals_fp != last_goals_fingerprint {
                let payload = json!({
                    "event": "goal_updated",
                    "ts_utc": ts,
                    "scope": "goals",
                    "payload": { "count": goals_fp.len() }
                });
                if write_ws_text(&mut stream, &payload.to_string())
                    .await
                    .is_err()
                {
                    return Ok(());
                }
                emitted = true;
            }
            heartbeat_counter += 1;
            if !emitted && heartbeat_counter >= 15 {
                let payload = json!({ "event": "heartbeat", "ts_utc": ts });
                if write_ws_text(&mut stream, &payload.to_string())
                    .await
                    .is_err()
                {
                    return Ok(());
                }
                heartbeat_counter = 0;
            } else if emitted {
                heartbeat_counter = 0;
            }
        }

        last_pending = pending;
        last_decided = decided;
        last_goals_fingerprint = goals_fp;
    }
}

fn scan_goals_fingerprint() -> HashSet<(String, u64)> {
    let dir = crate::cmd::goal::goals_dir();
    let mut out = HashSet::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let mtime = fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.insert((stem.to_string(), mtime));
    }
    out
}

fn scan_dispatch_ids(subdir: &str) -> HashSet<String> {
    let dir = crate::home::heiwa_state_dir().join("dispatch").join(subdir);
    scan_dispatch_ids_in(&dir)
}

fn scan_dispatch_ids_in(dir: &Path) -> HashSet<String> {
    let mut ids = HashSet::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return ids;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            ids.insert(stem.to_string());
        }
    }
    ids
}

async fn write_operator_ws_text<W>(
    stream: &mut W,
    text: &str,
    write_timeout: Duration,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    time::timeout(write_timeout, write_ws_text(stream, text))
        .await
        .map_err(|_| anyhow!("operator websocket write timed out"))?
}

async fn write_operator_ws_control<W>(
    stream: &mut W,
    opcode: u8,
    payload: &[u8],
    write_timeout: Duration,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    time::timeout(write_timeout, write_ws_control(stream, opcode, payload))
        .await
        .map_err(|_| anyhow!("operator websocket write timed out"))?
}

async fn write_ws_text<W>(stream: &mut W, text: &str) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    write_ws_frame(stream, 0x1, text.as_bytes()).await
}

async fn write_ws_control<W>(stream: &mut W, opcode: u8, payload: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > 125 || !matches!(opcode, 0x8..=0xA) {
        return Err(anyhow!("invalid websocket control frame"));
    }
    write_ws_frame(stream, opcode, payload).await
}

async fn write_ws_frame<W>(stream: &mut W, opcode: u8, bytes: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut frame = Vec::with_capacity(bytes.len() + 10);
    frame.push(0x80 | opcode);
    match bytes.len() {
        len if len < 126 => frame.push(len as u8),
        len if len <= u16::MAX as usize => {
            frame.push(126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            frame.push(127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(bytes);
    stream.write_all(&frame).await?;
    Ok(())
}

async fn serve_static(stream: &mut TcpStream, path: &str, head_only: bool) -> Result<()> {
    let root = cockpit_static_root();
    let file = static_file_for(&root, path);
    let Ok(bytes) = fs::read(&file) else {
        return write_response(
            stream,
            404,
            "application/json",
            json!({"ok": false, "error": {"code": "not_found"}})
                .to_string()
                .into_bytes(),
            head_only,
        )
        .await;
    };
    write_response(stream, 200, content_type(&file), bytes, head_only).await
}

/// Serve one REPL turn as Server-Sent Events over the raw socket.
///
/// The hand-rolled server closes every connection after one exchange, so the
/// body is EOF-terminated: no Content-Length, no chunked framing needed.
async fn serve_repl_stream(mut stream: TcpStream, prompt: String) -> Result<()> {
    let header = "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n";
    stream.write_all(header.as_bytes()).await?;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::ReplStreamEvent>(64);
    tokio::spawn(async move {
        crate::execute_repl_turn_streaming(&prompt, tx).await;
    });

    while let Some(event) = rx.recv().await {
        let (name, data, terminal) = match event {
            crate::ReplStreamEvent::Route(value) => ("route", value.to_string(), false),
            crate::ReplStreamEvent::Token(text) => {
                ("token", json!({ "text": text }).to_string(), false)
            }
            crate::ReplStreamEvent::Done(value) => ("done", value.to_string(), true),
            crate::ReplStreamEvent::Error(message) => {
                ("error", json!({ "message": message }).to_string(), true)
            }
        };
        let frame = format!("event: {name}\ndata: {data}\n\n");
        if stream.write_all(frame.as_bytes()).await.is_err() {
            break;
        }
        if terminal {
            break;
        }
    }
    let _ = stream.flush().await;
    Ok(())
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
    head_only: bool,
) -> Result<()> {
    write_response_with_headers(stream, status, content_type, body, head_only, &[]).await
}

async fn write_browser_session_redirect(
    stream: &mut TcpStream,
    session: &str,
    port: u16,
) -> Result<()> {
    let cookie = format!(
        "{}={session}; HttpOnly; SameSite=Strict; Path=/; Max-Age={BROWSER_SESSION_TTL_SECONDS}",
        browser_session_cookie_name(port),
    );
    write_response_with_headers(
        stream,
        303,
        "text/plain",
        Vec::new(),
        false,
        &[("Location", "/"), ("Set-Cookie", &cookie)],
    )
    .await
}

async fn write_response_with_headers(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
    head_only: bool,
    extra_headers: &[(&str, &str)],
) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        303 => "See Other",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let mut header = format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Length: {}\r\n\
         Content-Type: {content_type}\r\n\
         Cache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\n\
         Connection: close\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        header.push_str(name);
        header.push_str(": ");
        header.push_str(value);
        header.push_str("\r\n");
    }
    header.push_str("\r\n");
    stream.write_all(header.as_bytes()).await?;
    if !head_only {
        stream.write_all(&body).await?;
    }
    Ok(())
}

#[cfg(test)]
fn api_payload(path: &str, started_at: &str) -> Option<Value> {
    api_payload_for_port(path, started_at, DEFAULT_PORT)
}

fn api_payload_for_port(path: &str, started_at: &str, app_port: u16) -> Option<Value> {
    let data = match path {
        "/status/health" => json!({
            "status": "ok",
            "runtime_version": env!("CARGO_PKG_VERSION"),
            "started_at": started_at,
            "notes": ["heiwa-shell local app runtime"],
        }),
        "/api/runtime/snapshot" | "/api/v1/runtime/snapshot" => snapshot(started_at),
        "/api/v1/monitor" => monitor_payload(started_at, app_port),
        "/api/v1/resource" => resource_payload(),
        "/api/v1/session" => json!({
            "operator_id": env::var("USER").unwrap_or_else(|_| "local-operator".to_string()),
            "hostname": hostname_string(),
            "runtime_version": env!("CARGO_PKG_VERSION"),
            "channel": "stable",
            "default_route_role": "local_first",
            "app_url": format!("http://127.0.0.1:{app_port}/"),
        }),
        "/api/v1/providers" => json!({ "providers": provider_rows() }),
        "/api/v1/routes" => json!({ "routes": route_rows() }),
        "/api/v1/hooks" => json!({ "providers": hook_provider_rows(), "summary": hooks_summary() }),
        "/api/v1/missions" => json!({ "missions": [], "cursor": null }),
        "/api/v1/approvals" => json!({ "approvals": approval_rows() }),
        "/api/v1/approvals/summary" => crate::cmd::approvals::pending_approvals_summary_payload(),
        "/api/v1/life/today" => crate::cmd::life::today_payload(),
        "/api/v1/life/freshness" => crate::cmd::life::freshness_payload(),
        "/api/v1/life/social" => crate::cmd::life::social_payload(),
        "/api/v1/calendar/summary" => crate::cmd::calendar::summary_payload(),
        "/api/v1/calendar/resources" => {
            crate::cmd::calendar::apple_calendar_resources_payload_or_error()
        }
        "/api/v1/mail/summary" => crate::cmd::mail::summary_payload(),
        "/api/v1/automations" => crate::cmd::auto::automations_payload(),
        "/api/v1/receipts" => receipts_payload_for_state_dir(&state_dir()),
        "/api/v1/connectors" => crate::cmd::connectors::connectors_payload(),
        "/api/v1/goals" => crate::cmd::goal::goals_payload(),
        "/api/v1/compress/summary" => crate::cmd::compress::compress_summary_payload(),
        "/api/v1/rate-groups" => json!({ "rate_groups": rate_group_rows() }),
        "/api/v1/inbox" => {
            let state_dir = state_dir();
            let mut items = inbox_items_for_state_dir(&state_dir);
            items.extend(life_inbox_items());
            sort_values_by_time_desc(&mut items, "occurred_at");
            items.truncate(80);
            json!({ "items": items, "cursor": null })
        }
        "/api/v1/history" => {
            let state_dir = state_dir();
            history_summary_for_state_dir(&state_dir)
        }
        "/api/v1/traces" => json!({ "traces": [] }),
        "/api/v1/memory" => json!({ "entries": [] }),
        "/api/v1/agents" => json!({ "agents": worker_agent_rows() }),
        "/api/v1/agents/active" => json!({ "tasks": [] }),
        "/api/v1/capabilities" => capabilities_payload(),
        "/api/v1/crons" => json!({ "crons": [] }),
        "/api/v1/cells/catalog" => json!({ "cells": [] }),
        "/api/v1/providers/ollama/models" => ollama_models_payload(),
        _ => return None,
    };
    Some(json!({ "ok": true, "data": data }))
}

fn monitor_payload(started_at: &str, app_port: u16) -> Value {
    let state_dir = state_dir();
    let mut inbox_items = inbox_items_for_state_dir(&state_dir);
    inbox_items.extend(life_inbox_items());
    sort_values_by_time_desc(&mut inbox_items, "occurred_at");
    inbox_items.truncate(20);

    json!({
        "schema_version": "heiwa_monitor_v1",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "intent": "read-only operator and machine monitor for agents",
        "runtime": {
            "snapshot": snapshot(started_at),
            "session": {
                "operator_id": env::var("USER").unwrap_or_else(|_| "local-operator".to_string()),
                "hostname": hostname_string(),
                "runtime_version": env!("CARGO_PKG_VERSION"),
                "app_url": format!("http://127.0.0.1:{app_port}/"),
            },
        },
        "machine_ops": {
            "resource": resource_payload(),
            "providers": provider_rows(),
            "workers": worker_agent_rows(),
            "capabilities": capabilities_payload(),
            "rate_groups": rate_group_rows(),
        },
        "user_ops": {
            "today": crate::cmd::life::today_payload(),
            "freshness": crate::cmd::life::freshness_payload(),
            "calendar": crate::cmd::calendar::summary_payload(),
            "mail": crate::cmd::mail::summary_payload(),
            "approvals": crate::cmd::approvals::pending_approvals_summary_payload(),
            "connectors": crate::cmd::connectors::connectors_payload(),
            "goals": crate::cmd::goal::goals_payload(),
            "inbox": {
                "items": inbox_items,
                "truncated": true,
                "limit": 20,
            },
        },
        "receipts": receipts_payload_for_state_dir(&state_dir),
        "safety": {
            "mode": "read_only",
            "external_side_effects": false,
            "approval_required_for_writes": true,
        },
    })
}

const RECEIPT_SCAN_LIMIT: usize = 120;

fn receipts_payload_for_state_dir(state_dir: &Path) -> Value {
    let mut counts = serde_json::Map::new();
    let mut receipts = Vec::new();

    for (lane, dir) in receipt_lanes(state_dir) {
        let lane_receipts = scan_receipt_lane(state_dir, lane, &dir);
        counts.insert(lane.to_string(), json!(lane_receipts.len()));
        receipts.extend(lane_receipts);
    }

    receipts.sort_by(|a, b| {
        b.get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(a.get("created_at").and_then(Value::as_str).unwrap_or(""))
            .then_with(|| {
                b.get("modified_unix")
                    .and_then(Value::as_u64)
                    .cmp(&a.get("modified_unix").and_then(Value::as_u64))
            })
    });
    let total = receipts.len();
    let truncated = receipts.len() > RECEIPT_SCAN_LIMIT;
    receipts.truncate(RECEIPT_SCAN_LIMIT);
    counts.insert("total".to_string(), json!(total));

    json!({
        "command": "receipts summary",
        "state_dir": state_dir.display().to_string(),
        "counts": counts,
        "receipts": receipts,
        "truncated": truncated,
        "limit": RECEIPT_SCAN_LIMIT,
        "next": [
            "heiwa calendar status --json",
            "heiwa auto status --json",
            "heiwa mail status --json"
        ],
    })
}

fn receipt_lanes(state_dir: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![
        ("calendar", state_dir.join("calendar").join("receipts")),
        (
            "automations",
            state_dir.join("automations").join("receipts"),
        ),
        ("mail", state_dir.join("mail").join("receipts")),
        ("promotion", state_dir.join("evidence").join("promotion")),
        ("compress", state_dir.join("evidence").join("compress")),
        ("models", state_dir.join("models").join("receipts")),
        ("model", state_dir.join("model").join("receipts")),
    ]
}

fn scan_receipt_lane(state_dir: &Path, lane: &str, dir: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let metadata = entry.metadata().ok();
        let modified_unix = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());
        let modified_at = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339());
        let size_bytes = metadata.as_ref().map(|m| m.len());
        let relative_path = path
            .strip_prefix(state_dir)
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let raw = fs::read_to_string(&path);
        let (data, parse_error) = match raw {
            Ok(raw) => match serde_json::from_str::<Value>(&raw) {
                Ok(value) => (value, None),
                Err(error) => (json!({}), Some(error.to_string())),
            },
            Err(error) => (json!({}), Some(error.to_string())),
        };
        let created_at = receipt_created_at(&data)
            .or(modified_at.as_deref())
            .unwrap_or("unknown")
            .to_string();
        rows.push(json!({
            "lane": lane,
            "receipt_id": receipt_id_from_value(&data, &path),
            "kind": data.get("kind").and_then(Value::as_str)
                .or_else(|| data.get("schema_version").and_then(Value::as_str))
                .unwrap_or("unknown"),
            "event": data.get("event").and_then(Value::as_str),
            "created_at": created_at,
            "path": path.display().to_string(),
            "relative_path": relative_path,
            "size_bytes": size_bytes,
            "modified_unix": modified_unix,
            "parse_error": parse_error,
            "data": data,
        }));
    }
    rows
}

fn receipt_created_at(value: &Value) -> Option<&str> {
    [
        "created_at",
        "scanned_at",
        "completed_at",
        "started_at",
        "ts",
        "timestamp",
    ]
    .iter()
    .find_map(|field| value.get(*field).and_then(Value::as_str))
}

fn receipt_id_from_value(value: &Value, path: &Path) -> String {
    value
        .get("receipt_id")
        .and_then(Value::as_str)
        .or_else(|| value.get("id").and_then(Value::as_str))
        .or_else(|| value.get("execution_id").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("unknown-receipt")
                .to_string()
        })
}

const FILE_TREE_LIMIT: usize = 160;
const FILE_PREVIEW_LIMIT: usize = 96 * 1024;

fn files_tree_payload_from_target(target: &str) -> Result<Value> {
    let requested = query_param(target, "path").unwrap_or_else(default_workspace_path);
    let path = resolve_readonly_user_path(&requested)?;
    if !path.is_dir() {
        return Err(anyhow!("not a directory: {}", path.display()));
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(&path)? {
        let Ok(entry) = entry else { continue };
        let entry_path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        entries.push(json!({
            "name": name,
            "path": entry_path.display().to_string(),
            "kind": if metadata.is_dir() { "directory" } else if metadata.is_file() { "file" } else { "other" },
            "size_bytes": if metadata.is_file() { Some(metadata.len()) } else { None },
            "modified_unix": metadata.modified().ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs()),
            "hidden": name.starts_with('.'),
        }));
    }
    entries.sort_by(|a, b| {
        let a_dir = a.get("kind").and_then(Value::as_str) == Some("directory");
        let b_dir = b.get("kind").and_then(Value::as_str) == Some("directory");
        b_dir.cmp(&a_dir).then_with(|| {
            a.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase()
                .cmp(
                    &b.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_ascii_lowercase(),
                )
        })
    });
    let truncated = entries.len() > FILE_TREE_LIMIT;
    entries.truncate(FILE_TREE_LIMIT);

    Ok(json!({
        "command": "files tree",
        "root": default_workspace_path(),
        "path": path.display().to_string(),
        "parent": path.parent().map(|parent| parent.display().to_string()),
        "entries": entries,
        "truncated": truncated,
        "limit": FILE_TREE_LIMIT,
        "policy": "read_only_user_home_or_temp",
    }))
}

fn file_preview_payload_from_target(target: &str) -> Result<Value> {
    let requested = query_param(target, "path").ok_or_else(|| anyhow!("missing path"))?;
    let path = resolve_readonly_user_path(&requested)?;
    let metadata = fs::metadata(&path)?;
    if metadata.is_dir() {
        return Ok(json!({
            "command": "files preview",
            "path": path.display().to_string(),
            "kind": "directory",
            "size_bytes": null,
            "truncated": false,
            "content": null,
            "message": "Directory selected. Open it in the tree to inspect children.",
        }));
    }
    if !metadata.is_file() {
        return Err(anyhow!("not a regular file: {}", path.display()));
    }

    let bytes = fs::read(&path)?;
    let truncated = bytes.len() > FILE_PREVIEW_LIMIT;
    let sample = &bytes[..bytes.len().min(FILE_PREVIEW_LIMIT)];
    let (content, binary) = match std::str::from_utf8(sample) {
        Ok(text) => (Some(text.to_string()), false),
        Err(_) => (None, true),
    };

    Ok(json!({
        "command": "files preview",
        "path": path.display().to_string(),
        "name": path.file_name().and_then(|name| name.to_str()).unwrap_or(""),
        "extension": path.extension().and_then(|ext| ext.to_str()),
        "kind": "file",
        "size_bytes": metadata.len(),
        "modified_unix": metadata.modified().ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs()),
        "truncated": truncated,
        "limit": FILE_PREVIEW_LIMIT,
        "binary": binary,
        "content": content,
        "policy": "read_only_text_preview",
    }))
}

fn browser_probe_payload_from_target(target: &str) -> Result<Value> {
    let raw = query_param(target, "url")
        .unwrap_or_else(|| "https://www.google.com/search?q=Heiwa".to_string());
    let normalized = normalize_browser_url(&raw)?;
    let host = normalized
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(rest))
        .unwrap_or(&normalized)
        .to_string();
    Ok(json!({
        "command": "browser probe",
        "url": normalized,
        "host": host,
        "mode": "embedded_webview",
        "policy": "user_navigated_no_credentials_exfiltration",
        "notes": [
            "Some sites block iframe embedding with X-Frame-Options or CSP.",
            "Tauri builds render this as an app WebView surface, not a server-side fetch."
        ]
    }))
}

fn default_workspace_path() -> String {
    env::current_dir()
        .unwrap_or_else(|_| crate::home::heiwa_home().unwrap_or_else(|| PathBuf::from(".")))
        .display()
        .to_string()
}

fn resolve_readonly_user_path(raw: &str) -> Result<PathBuf> {
    let expanded = if raw == "~" {
        crate::home::heiwa_home().ok_or_else(|| anyhow!("home directory unavailable"))?
    } else if let Some(rest) = raw.strip_prefix("~/") {
        crate::home::heiwa_home()
            .ok_or_else(|| anyhow!("home directory unavailable"))?
            .join(rest)
    } else {
        PathBuf::from(raw)
    };
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        env::current_dir()?.join(expanded)
    };
    let canonical = candidate.canonicalize()?;
    let home = crate::home::heiwa_home()
        .and_then(|home| home.canonicalize().ok())
        .unwrap_or_else(|| PathBuf::from("/"));
    let temp = env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| env::temp_dir());
    if canonical.starts_with(&home) || canonical.starts_with(&temp) {
        Ok(canonical)
    } else {
        Err(anyhow!(
            "path outside read-only Heiwa scope: {}",
            canonical.display()
        ))
    }
}

fn normalize_browser_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("empty url"));
    }
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    if with_scheme.contains(char::is_whitespace) {
        return Err(anyhow!("url cannot contain whitespace"));
    }
    Ok(with_scheme)
}

fn snapshot(started_at: &str) -> Value {
    let status = RuntimeStatus::detect();
    json!({
        "runtime": {
            "status": "ok",
            "started_at": started_at,
            "version": env!("CARGO_PKG_VERSION"),
            "node": status.node,
            "transport": status.transport,
            "keep_awake": status.keep_awake,
        },
        "workers": status.workers_summary,
        "approvals": status.approvals_summary,
        "mail": status.mail_summary,
        "hooks": status.hooks_summary,
        "providers": provider_rows(),
        "resource": resource_payload(),
        "machine": machine_perspective_payload(),
    })
}

fn machine_perspective_payload() -> Value {
    let current_runtime = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "channel": runtime_channel(),
        "install_path": env::current_exe().ok().map(|path| path.display().to_string()),
    });
    let (mut machine, recognition_error) = match heiwa_install::load_machine_manifest() {
        Ok(Some(mut manifest)) => {
            if manifest.display_name.trim().is_empty() {
                manifest.display_name = manifest.hostname.clone();
            }
            if manifest.hardware.logical_cpu_count == 0 || manifest.hardware.memory_total_bytes == 0
            {
                manifest.hardware = heiwa_install::probe_machine_hardware();
            }
            if manifest.capabilities.host_surfaces.is_empty() {
                manifest.capabilities.host_surfaces =
                    vec!["terminal".to_string(), "desktop".to_string()];
            }
            if manifest.capabilities.display_surfaces.is_empty() {
                manifest.capabilities.display_surfaces = vec!["desktop".to_string()];
            }
            match serde_json::to_value(manifest) {
                Ok(machine) => (machine, None),
                Err(_) => (
                    unrecognized_machine_payload("manifest_error"),
                    Some(json!({
                        "code": "invalid_shape",
                        "message": "Machine identity file is incomplete.",
                    })),
                ),
            }
        }
        Ok(None) => (unrecognized_machine_payload("unregistered"), None),
        Err(error) => (
            unrecognized_machine_payload("manifest_error"),
            Some(json!({
                "code": machine_manifest_issue_code(error.issue()),
                "message": error.user_message(),
            })),
        ),
    };
    if let Some(object) = machine.as_object_mut() {
        object.insert("runtime".to_string(), current_runtime);
        if let Some(error) = recognition_error {
            object.insert("recognition_error".to_string(), error);
        }
        object.insert(
            "perspective".to_string(),
            machine_perspective(&crate::home::heiwa_runtime_dir()),
        );
    }
    machine
}

fn unrecognized_machine_payload(registration_status: &str) -> Value {
    let hardware = heiwa_install::probe_machine_hardware();
    json!({
                "schema_version": "heiwa_machine_v1",
                "device_id": null,
                "display_name": hostname_string(),
                "hostname": hostname_string(),
                "os": env::consts::OS,
                "arch": env::consts::ARCH,
                "device_class": "full_node",
                "hardware": hardware,
                "capabilities": {
                    "provider_clis": [],
                    "local_model_runtimes": [],
                    "host_surfaces": ["terminal", "desktop"],
                    "display_surfaces": ["desktop"],
                },
                "registration_status": registration_status,
    })
}

fn machine_manifest_issue_code(issue: heiwa_install::MachineManifestLoadIssue) -> &'static str {
    match issue {
        heiwa_install::MachineManifestLoadIssue::ReadFailed => "read_failed",
        heiwa_install::MachineManifestLoadIssue::InvalidJson => "invalid_json",
        heiwa_install::MachineManifestLoadIssue::UnsupportedSchema => "unsupported_schema",
        heiwa_install::MachineManifestLoadIssue::InvalidShape => "invalid_shape",
    }
}

/// This machine's place in the mesh, read from state rather than asserted.
///
/// Peer identity is the node fingerprint, never the pre-mesh local handle
/// (`device_id`). A mesh read that fails is reported as `unknown` and carried
/// in `mesh_errors`; rendering it as `local_only` would turn "I could not look"
/// into "there is nobody there".
fn machine_perspective(runtime_root: &Path) -> Value {
    let mesh = match crate::cmd::mesh::summarize(runtime_root) {
        Ok(summary) => summary,
        Err(error) => json!({
            "node": Value::Null,
            "enrolled_peer_ids": [],
            "sync_status": "unknown",
            "transport": "not_configured",
            "errors": [{ "code": "mesh_state_unreadable", "message": error.to_string() }],
        }),
    };
    let peer_ids = mesh["enrolled_peer_ids"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut perspective = json!({
        "locality": "local",
        "execution_scope": "this_device",
        "data_scope": "shared_user",
        "sync_status": mesh["sync_status"],
        "transport": mesh["transport"],
        "node_id": mesh["node"]["node_id"],
        "enrolled_peer_count": peer_ids.len(),
        "enrolled_peer_ids": peer_ids,
    });
    if let Some(errors) = mesh.get("errors") {
        perspective["mesh_errors"] = errors.clone();
    }
    perspective
}

fn resource_payload() -> Value {
    let policy = ResourcePolicy::default();
    let (free_memory_bytes, free_memory_source) = free_memory_bytes();
    let (load_1m, load_source) = load_1m();
    let power = power_state();
    let (thermal_pressure, thermal_source) = thermal_pressure();
    let snapshot = ResourceSnapshot {
        cpu_count: std::thread::available_parallelism()
            .map(|count| count.get() as u32)
            .unwrap_or(1),
        load_1m,
        free_memory_bytes,
        battery_percent: power.battery_percent,
        on_battery: power.on_battery,
        thermal_pressure,
    };
    let admissions = json!({
        "foreground_interactive": policy.admit(&snapshot, WorkClass::ForegroundInteractive),
        "background_watch": policy.admit(&snapshot, WorkClass::BackgroundWatch),
        "local_summary": policy.admit(&snapshot, WorkClass::LocalSummary),
        "local_model_small": policy.admit(&snapshot, WorkClass::LocalModelSmall),
        "local_model_large": policy.admit(&snapshot, WorkClass::LocalModelLarge),
        "provider_escalation": policy.admit(&snapshot, WorkClass::ProviderEscalation),
    });

    json!({
        "snapshot": snapshot,
        "policy": policy,
        "admissions": admissions,
        "sources": {
            "cpu_count": "std::thread::available_parallelism",
            "load_1m": load_source,
            "free_memory_bytes": free_memory_source,
            "battery_percent": power.source,
            "thermal_pressure": thermal_source,
        },
        "notes": [
            "read_only_local_probe",
            "resource policy gates local always-on work before provider routing"
        ],
    })
}

fn capabilities_payload() -> Value {
    capabilities_payload_for_state_dir(&state_dir())
}

fn capabilities_payload_for_state_dir(state_dir: &Path) -> Value {
    let capabilities_dir = state_dir.join("capabilities");
    let mut catalogs = Vec::new();
    let Ok(entries) = fs::read_dir(&capabilities_dir) else {
        return capabilities_payload_with_catalogs(&capabilities_dir, catalogs);
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let catalog_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("capability-catalog")
            .to_string();
        catalogs.push(json!({
            "catalog_id": catalog_id,
            "path": path.display().to_string(),
            "schema_version": value.get("schema_version").and_then(Value::as_str).unwrap_or("unknown"),
            "generated_at": value.get("generated_at").and_then(Value::as_str),
            "providers": value.get("providers").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "gemini_extensions": value.get("gemini_extensions").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "codex_plugins_observed": value.get("codex_plugins_observed").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "codex_mcp_servers": value.get("codex_mcp_servers").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "claude_plugins_observed": value.get("claude_plugins_observed").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "gemini_skills_observed": value.get("gemini_skills_observed").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "installed_apps_observed": value.get("installed_apps_observed").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "peer_handoff_findings": value.get("peer_handoff_findings").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "reference_sources": value.get("reference_sources").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "integration_families": value.get("integration_families").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "runtime_targets": value.get("runtime_targets").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "performance_targets": value.get("performance_targets").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            "next_runtime_targets": value.get("next_runtime_targets").cloned().unwrap_or_else(|| json!([])),
        }));
    }

    catalogs.sort_by(|a, b| {
        let a_id = a.get("catalog_id").and_then(Value::as_str).unwrap_or("");
        let b_id = b.get("catalog_id").and_then(Value::as_str).unwrap_or("");
        b_id.cmp(a_id)
    });
    capabilities_payload_with_catalogs(&capabilities_dir, catalogs)
}

fn capabilities_payload_with_catalogs(capabilities_dir: &Path, catalogs: Vec<Value>) -> Value {
    let latest = catalogs.first().cloned().unwrap_or(Value::Null);
    let tools = tool_call_contracts();
    let executable_tools = tools
        .iter()
        .filter(|tool| tool.get("execution_state").and_then(Value::as_str) == Some("executable"))
        .count();

    json!({
        "catalogs": catalogs,
        "latest": latest,
        "path": capabilities_dir.display().to_string(),
        "tool_call_contract_version": "heiwa_tool_call_contract_v1",
        "tools": tools,
        "tool_counts": {
            "total": tools.len(),
            "executable": executable_tools,
            "target_only": tools.len().saturating_sub(executable_tools),
        },
    })
}

fn tool_call_contracts() -> Vec<Value> {
    let mut scope =
        ExecutionScope::local_default(env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    for name in ["fs.list", "fs.read", "repo.grep"] {
        scope.tool_leases.push(ToolLease {
            name: name.to_string(),
            risk_class: RiskClass::HostSafeReadonly,
            allowed: true,
        });
    }

    let registry = heiwa_mcp::local_repo_registry(scope);
    let mut tools: Vec<Value> = registry
        .names()
        .into_iter()
        .map(|name| {
            json!({
                "id": name,
                "name": name,
                "plane": "evidence",
                "kind": "local_mcp_tool",
                "execution_state": "executable",
                "risk_class": "host_safe_readonly",
                "lease_required": true,
                "approval_class": "auto_allowed_readonly",
                "adapter": "heiwa_mcp::local_repo_registry",
                "description": registry.description(name).unwrap_or(""),
                "input_schema": registry
                    .schema(name)
                    .and_then(|schema| serde_json::to_value(schema).ok())
                    .unwrap_or_else(|| json!({})),
                "evidence": {
                    "receipt": "ToolCallReceipt",
                    "status_values": ["success", "failure", "denied"],
                },
            })
        })
        .collect();

    tools.extend([
        json!({
            "id": "shell.run",
            "name": "shell.run",
            "plane": "execution",
            "kind": "shell_tool",
            "execution_state": "declared_no_adapter",
            "risk_class": "host_mutating",
            "lease_required": true,
            "approval_class": "approval_required",
            "adapter": null,
            "description": "Shell work is a product target and a REPL lease exists, but no agentic local MCP shell adapter is wired yet.",
            "next": "Add bounded shell capability registry before exposing model-initiated shell calls.",
        }),
        json!({
            "id": "browser.isolated",
            "name": "browser.isolated",
            "plane": "intake",
            "kind": "browser_tool",
            "execution_state": "target_only",
            "risk_class": "sandbox_required",
            "lease_required": true,
            "approval_class": "approval_required_for_logged_in_or_form_submit",
            "adapter": null,
            "description": "Isolated browser task lane is required for product parity but is not wired in this runtime API yet.",
        }),
        json!({
            "id": "computer.use",
            "name": "computer.use",
            "plane": "execution",
            "kind": "computer_use_tool",
            "execution_state": "target_only",
            "risk_class": "sandbox_required",
            "lease_required": true,
            "approval_class": "approval_required",
            "adapter": null,
            "description": "Full computer use is target work and must stage side effects before execution.",
        }),
        json!({
            "id": "calendar.read",
            "name": "calendar.read",
            "plane": "intake",
            "kind": "connector_tool",
            "execution_state": "target_only",
            "risk_class": "host_safe_readonly",
            "lease_required": true,
            "approval_class": "connector_auth_required",
            "adapter": null,
            "description": "Calendar/scheduling is target Intake work; no product-grade connector adapter is wired here yet.",
        }),
    ]);

    tools
}

fn load_1m() -> (f32, &'static str) {
    #[cfg(unix)]
    {
        let mut loads = [0.0_f64; 3];
        let count = unsafe { libc::getloadavg(loads.as_mut_ptr(), 1) };
        if count == 1 {
            return (loads[0] as f32, "libc_getloadavg");
        }
    }
    (0.0, "unavailable_default_zero")
}

struct PowerState {
    battery_percent: Option<u8>,
    on_battery: bool,
    source: &'static str,
}

#[cfg(target_os = "macos")]
fn power_state() -> PowerState {
    let Ok(output) = Command::new("/usr/bin/pmset").args(["-g", "batt"]).output() else {
        return PowerState {
            battery_percent: None,
            on_battery: false,
            source: "macos_pmset_unavailable",
        };
    };
    let raw = String::from_utf8_lossy(&output.stdout);
    let battery_percent = raw.lines().find_map(|line| {
        let percent = line.split('%').next()?.split_whitespace().last()?;
        percent.parse::<u8>().ok()
    });
    PowerState {
        battery_percent,
        on_battery: raw.contains("'Battery Power'"),
        source: "macos_pmset_g_batt",
    }
}

#[cfg(target_os = "linux")]
fn power_state() -> PowerState {
    let batteries = fs::read_dir("/sys/class/power_supply").ok();
    let battery = batteries.and_then(|entries| {
        entries.filter_map(Result::ok).find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("BAT")
        })
    });
    let Some(path) = battery.map(|entry| entry.path()) else {
        return PowerState {
            battery_percent: None,
            on_battery: false,
            source: "linux_sysfs_no_battery",
        };
    };
    let battery_percent = fs::read_to_string(path.join("capacity"))
        .ok()
        .and_then(|value| value.trim().parse::<u8>().ok());
    let on_battery = fs::read_to_string(path.join("status"))
        .ok()
        .is_some_and(|status| status.trim().eq_ignore_ascii_case("discharging"));
    PowerState {
        battery_percent,
        on_battery,
        source: "linux_sysfs_power_supply",
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn power_state() -> PowerState {
    PowerState {
        battery_percent: None,
        on_battery: false,
        source: "platform_power_probe_unavailable",
    }
}

#[cfg(target_os = "macos")]
fn thermal_pressure() -> (ThermalPressure, &'static str) {
    let Ok(output) = Command::new("/usr/bin/pmset")
        .args(["-g", "therm"])
        .output()
    else {
        return (ThermalPressure::Unknown, "macos_pmset_unavailable");
    };
    let raw = String::from_utf8_lossy(&output.stdout);
    let pressure = if raw.contains("No thermal warning level has been recorded") {
        ThermalPressure::Nominal
    } else if raw.contains("CPU_Speed_Limit") || raw.contains("thermal warning") {
        ThermalPressure::Serious
    } else {
        ThermalPressure::Unknown
    };
    (pressure, "macos_pmset_g_therm")
}

#[cfg(not(target_os = "macos"))]
fn thermal_pressure() -> (ThermalPressure, &'static str) {
    (
        ThermalPressure::Unknown,
        "platform_thermal_probe_unavailable",
    )
}

fn runtime_channel() -> String {
    env::var("HEIWA_CHANNEL").unwrap_or_else(|_| {
        if cfg!(debug_assertions) {
            "dev".to_string()
        } else {
            "stable".to_string()
        }
    })
}

fn free_memory_bytes() -> (u64, &'static str) {
    if let Some(bytes) = linux_mem_available_bytes() {
        return (bytes, "linux_proc_meminfo_memavailable");
    }
    if let Some(bytes) = macos_memory_pressure_available_bytes() {
        return (bytes, "macos_memory_pressure_free_percentage");
    }
    if let Some(bytes) = macos_vm_stat_available_bytes() {
        return (bytes, "macos_vm_stat_free_inactive_speculative");
    }
    (u64::MAX, "unavailable_assumed_unconstrained")
}

#[cfg(target_os = "linux")]
fn linux_mem_available_bytes() -> Option<u64> {
    let raw = fs::read_to_string("/proc/meminfo").ok()?;
    raw.lines().find_map(|line| {
        let rest = line.strip_prefix("MemAvailable:")?;
        let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        Some(kb * 1024)
    })
}

#[cfg(not(target_os = "linux"))]
fn linux_mem_available_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn macos_memory_pressure_available_bytes() -> Option<u64> {
    let output = Command::new("/usr/bin/memory_pressure").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    parse_macos_memory_pressure_available_bytes(&raw)
}

#[cfg(not(target_os = "macos"))]
fn macos_memory_pressure_available_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn parse_macos_memory_pressure_available_bytes(raw: &str) -> Option<u64> {
    let total_bytes = raw.lines().find_map(|line| {
        let rest = line.strip_prefix("The system has ")?;
        rest.split_whitespace().next()?.parse::<u64>().ok()
    })?;
    let free_percent = raw.lines().find_map(|line| {
        let rest = line.strip_prefix("System-wide memory free percentage: ")?;
        rest.trim_end_matches('%').trim().parse::<u64>().ok()
    })?;
    Some(total_bytes.saturating_mul(free_percent) / 100)
}

#[cfg(target_os = "macos")]
fn macos_vm_stat_available_bytes() -> Option<u64> {
    let output = Command::new("/usr/bin/vm_stat").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let page_size = parse_vm_stat_page_size(&raw)?;
    let pages = parse_vm_stat_pages(&raw, "Pages free")
        + parse_vm_stat_pages(&raw, "Pages inactive")
        + parse_vm_stat_pages(&raw, "Pages speculative");
    Some(pages * page_size)
}

#[cfg(not(target_os = "macos"))]
fn macos_vm_stat_available_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn parse_vm_stat_page_size(raw: &str) -> Option<u64> {
    let marker = "page size of ";
    let (_, rest) = raw.lines().next()?.split_once(marker)?;
    let bytes = rest.split_whitespace().next()?.parse::<u64>().ok()?;
    Some(bytes)
}

#[cfg(target_os = "macos")]
fn parse_vm_stat_pages(raw: &str, label: &str) -> u64 {
    raw.lines()
        .find_map(|line| {
            let rest = line
                .trim()
                .strip_prefix(label)?
                .trim_start_matches(':')
                .trim();
            rest.trim_end_matches('.')
                .replace('.', "")
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
}

fn provider_rows() -> Vec<Value> {
    ["ollama", "gemini", "antigravity", "claude", "codex"]
        .iter()
        .filter_map(|provider| heiwa_provider::get_auth_status(provider))
        .map(|account| {
            json!({
                "provider_id": account.provider_id,
                "display_name": provider_display_name(&account.provider_id),
                "auth_kind": auth_kind_label(&account.auth_kind),
                "status": cockpit_status(&account.status),
                "rate_group": account.rate_group,
                "default_model": account.default_model,
                "last_validated_at": chrono::Utc::now().to_rfc3339(),
                "last_error": if cockpit_status(&account.status) == "connected" { Value::Null } else { Value::String(account.status.clone()) },
                "supported_lanes": supported_lanes(&account.provider_id),
            })
        })
        .collect()
}

/// Live route table: ask DREX what it would pick today for each intent,
/// using the cached account registry (no CLI probing on the GET path).
fn route_rows() -> Vec<Value> {
    use heiwa_core::drex::{default_policy, plan_route, DrexIngress};

    let registry = heiwa_provider::AccountRegistry::load();
    let tiers = crate::get_live_model_tiers(&registry);
    if tiers.is_empty() {
        return vec![json!({
            "role": "chat",
            "provider": Value::Null,
            "model": Value::Null,
            "source": "no_model_tiers",
            "fallbacks": [],
            "offline_capable": false,
        })];
    }

    let policy = default_policy();
    ["chat", "build", "research", "audit"]
        .iter()
        .map(|intent| {
            let ingress = DrexIngress {
                intent: (*intent).to_string(),
                risk: "low".to_string(),
                raw_text: format!("route table preview for {intent}"),
                privacy: "standard".to_string(),
                runtime: "any".to_string(),
                available_vram_mb: 8192,
                required_context_tokens: 1024,
            };
            match plan_route(&ingress, &tiers, &policy) {
                Ok(route) => match route.selected_model {
                    Some(selected) => {
                        let fallbacks: Vec<String> = {
                            let mut seen = vec![selected.provider.clone()];
                            tiers
                                .iter()
                                .filter_map(|tier| {
                                    if seen.contains(&tier.provider) {
                                        None
                                    } else {
                                        seen.push(tier.provider.clone());
                                        Some(tier.provider.clone())
                                    }
                                })
                                .collect()
                        };
                        json!({
                            "role": intent,
                            "provider": selected.provider,
                            "model": selected.model_id,
                            "rate_group": selected.rate_group,
                            "source": "drex_live",
                            "fallbacks": fallbacks,
                            "offline_capable": selected.provider == "ollama",
                        })
                    }
                    None => json!({
                        "role": intent,
                        "provider": Value::Null,
                        "model": Value::Null,
                        "source": "drex_no_match",
                        "fallbacks": [],
                        "offline_capable": false,
                    }),
                },
                Err(error) => json!({
                    "role": intent,
                    "provider": Value::Null,
                    "model": Value::Null,
                    "source": format!("drex_error: {error}"),
                    "fallbacks": [],
                    "offline_capable": false,
                }),
            }
        })
        .collect()
}

/// Calendar holds and priority mail as inbox items, merged with dispatch
/// receipts and event-log rows on /api/v1/inbox.
fn life_inbox_items() -> Vec<Value> {
    let mut items = Vec::new();
    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    for hold in crate::cmd::calendar::holds_for_date(&today) {
        let hold_id = hold.get("id").and_then(Value::as_str).unwrap_or("hold");
        let title = hold.get("title").and_then(Value::as_str).unwrap_or("Hold");
        let start = hold.get("start").and_then(Value::as_str).unwrap_or("--:--");
        items.push(json!({
            "item_id": format!("calendar:{hold_id}"),
            "kind": "calendar_hold",
            "plane": "intake",
            "priority": "normal",
            "pinned": false,
            "status": hold.get("status").and_then(Value::as_str).unwrap_or("draft"),
            "title": format!("{start} {title}"),
            "summary": hold.get("note").and_then(Value::as_str).unwrap_or("Local hold; external promotion is approval-gated."),
            "occurred_at": hold.get("created_at").and_then(Value::as_str).unwrap_or("unknown"),
            "source": {
                "source_id": hold_id,
                "source_type": "calendar_hold",
                "label": "Heiwa Calendar",
                "path": crate::home::heiwa_state_dir()
                    .join("calendar").join("holds").join(format!("{hold_id}.json"))
                    .display().to_string(),
            },
            "subject_ref": hold_id,
            "receipt_refs": [{ "kind": "receipt", "ref": format!("rcpt-{hold_id}") }],
        }));
    }

    for row in crate::cmd::mail::priority_rows() {
        let action = row
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("digest");
        if action == "digest" {
            continue;
        }
        let subject = row
            .get("subject")
            .and_then(Value::as_str)
            .unwrap_or("(no subject)");
        let sender = row
            .get("sender")
            .and_then(Value::as_str)
            .unwrap_or("unknown sender");
        items.push(json!({
            "item_id": format!("mail:{}", stable_hash(&format!("{sender}|{subject}"))),
            "kind": "mail_priority",
            "plane": "intake",
            "priority": if action == "draft" { "high" } else { "normal" },
            "pinned": false,
            "status": action,
            "title": subject,
            "summary": format!("{sender} · staged action: {action}"),
            "occurred_at": row.get("date").and_then(Value::as_str).unwrap_or("unknown"),
            "source": {
                "source_id": "mail_priority_scan",
                "source_type": "mail_metadata",
                "label": "Mail priority scan",
                "path": crate::home::heiwa_state_dir()
                    .join("mail").join("headers.jsonl")
                    .display().to_string(),
            },
            "subject_ref": subject,
            "receipt_refs": [],
        }));
    }

    items
}

fn stable_hash(input: &str) -> String {
    use sha1::{Digest, Sha1};
    let digest = Sha1::digest(input.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn approval_rows() -> Vec<Value> {
    crate::cmd::approvals::scan_pending_requests()
        .into_iter()
        .map(|value| {
        let approval_id = value
            .get("id")
            .or_else(|| value.get("request_id"))
            .or_else(|| value.get("task_id"))
            .and_then(Value::as_str)
            .unwrap_or("approval")
            .to_string();
        let summary = value
            .get("summary")
            .or_else(|| value.get("action"))
            .or_else(|| value.get("reason"))
            .and_then(Value::as_str)
            .unwrap_or("Local approval request")
            .to_string();
        json!({
            "approval_id": approval_id,
            "mission_id": value.get("mission_id").or_else(|| value.get("task_id")).and_then(Value::as_str).unwrap_or("local-dispatch"),
            "risk_level": value.get("risk_level").or_else(|| value.get("risk")).or_else(|| value.get("risk_tier")).and_then(Value::as_str).unwrap_or("unknown"),
            "summary": summary,
            "requested_at": value.get("requested_at").or_else(|| value.get("created_at")).and_then(Value::as_str).unwrap_or("unknown"),
            "expires_at": Value::Null,
            "requested_by": value.get("requested_by").or_else(|| value.get("from")).and_then(Value::as_str).unwrap_or("local-dispatch"),
        })
    })
    .collect()
}

fn history_summary_for_state_dir(state_dir: &Path) -> Value {
    let dispatch_results = dispatch_result_values_for_state_dir(state_dir);
    let mut recent_runs = dispatch_results
        .iter()
        .map(|(_, value)| {
            let request_id = string_field(value, &["request_id", "mission_id", "task_id"])
                .unwrap_or("local-dispatch");
            json!({
                "mission_id": request_id,
                "status": string_field(value, &["status"]).unwrap_or("unknown"),
                "updated_at": string_field(value, &["completed_at", "updated_at", "created_at"]).unwrap_or("unknown"),
                "summary": string_field(value, &["summary"]),
            })
        })
        .collect::<Vec<_>>();
    sort_values_by_time_desc(&mut recent_runs, "updated_at");
    recent_runs.truncate(40);

    let mut artifacts = Vec::new();
    for (_, result) in &dispatch_results {
        let updated_at = string_field(result, &["completed_at", "updated_at", "created_at"])
            .unwrap_or("unknown");
        if let Some(refs) = result.get("evidence_refs").and_then(Value::as_array) {
            for evidence_ref in refs.iter().filter_map(Value::as_str) {
                artifacts.push(json!({
                    "id": evidence_ref,
                    "kind": "evidence_ref",
                    "label": evidence_ref,
                    "updated_at": updated_at,
                }));
            }
        }
    }
    sort_values_by_time_desc(&mut artifacts, "updated_at");
    artifacts.truncate(80);

    json!({
        "sessions": [],
        "recent_runs": recent_runs,
        "artifacts": artifacts,
        "cursor": null,
    })
}

fn inbox_items_for_state_dir(state_dir: &Path) -> Vec<Value> {
    let mut items = Vec::new();
    items.extend(event_log_items_for_state_dir(state_dir));
    items.extend(
        dispatch_result_values_for_state_dir(state_dir)
            .into_iter()
            .map(|(path, value)| dispatch_result_inbox_item(&path, &value)),
    );
    sort_values_by_time_desc(&mut items, "occurred_at");
    items.truncate(80);
    items
}

fn dispatch_result_inbox_item(path: &Path, result: &Value) -> Value {
    let result_id = string_field(result, &["result_id"])
        .or_else(|| path.file_stem().and_then(|stem| stem.to_str()))
        .unwrap_or("dispatch-result");
    let request_id =
        string_field(result, &["request_id", "mission_id", "task_id"]).unwrap_or("local-dispatch");
    let occurred_at =
        string_field(result, &["completed_at", "updated_at", "created_at"]).unwrap_or("unknown");
    let adapter = string_field(result, &["adapter"]).unwrap_or("dispatch");
    let status = string_field(result, &["status"]).unwrap_or("unknown");
    let summary = string_field(result, &["summary"]).unwrap_or("Dispatch result recorded");
    let receipt_refs = result
        .get("evidence_refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|evidence_ref| evidence_ref.as_str().map(str::to_string))
        .map(|evidence_ref| {
            json!({
                "kind": "evidence_ref",
                "ref": evidence_ref,
            })
        })
        .collect::<Vec<_>>();

    json!({
        "item_id": format!("receipt:{result_id}"),
        "kind": "dispatch_result",
        "plane": "evidence",
        "priority": priority_for_status(status),
        "pinned": false,
        "status": status,
        "title": format!("{adapter} {status}"),
        "summary": summary,
        "occurred_at": occurred_at,
        "source": source_ref(result_id, "dispatch_result", adapter, path),
        "subject_ref": request_id,
        "receipt_refs": receipt_refs,
    })
}

fn event_log_items_for_state_dir(state_dir: &Path) -> Vec<Value> {
    let events_path = state_dir.join("events").join("events.jsonl");
    let Ok(raw) = fs::read_to_string(&events_path) else {
        return Vec::new();
    };
    raw.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .take(40)
        .map(|event| event_log_inbox_item(&events_path, &event))
        .collect()
}

fn event_log_inbox_item(path: &Path, event: &Value) -> Value {
    let event_id = string_field(event, &["event_id"]).unwrap_or("event");
    let event_type = string_field(event, &["event_type"]).unwrap_or("event");
    let source = string_field(event, &["source"]).unwrap_or("local-state");
    let subject = string_field(event, &["subject"]).unwrap_or(event_type);
    let occurred_at = string_field(event, &["ts", "created_at", "updated_at"]).unwrap_or("unknown");
    let payload_ref = string_field(event, &["payload_ref"]);
    let receipt_refs = payload_ref
        .map(|payload_ref| {
            vec![json!({
                "kind": "payload_ref",
                "ref": payload_ref,
            })]
        })
        .unwrap_or_default();

    json!({
        "item_id": format!("event:{event_id}"),
        "kind": "event",
        "plane": plane_for_event_type(event_type),
        "priority": priority_for_severity(string_field(event, &["severity"]).unwrap_or("info")),
        "pinned": false,
        "status": string_field(event, &["severity"]).unwrap_or("info"),
        "title": event_type,
        "summary": subject,
        "occurred_at": occurred_at,
        "source": source_ref(event_id, "event_log", source, path),
        "subject_ref": subject,
        "receipt_refs": receipt_refs,
    })
}

fn dispatch_result_values_for_state_dir(state_dir: &Path) -> Vec<(PathBuf, Value)> {
    let results = state_dir.join("dispatch").join("results");
    let Ok(entries) = fs::read_dir(results) else {
        return Vec::new();
    };
    let mut values = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                return None;
            }
            let value = fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())?;
            Some((path, value))
        })
        .collect::<Vec<_>>();
    values.sort_by(|(_, a), (_, b)| {
        string_field(b, &["completed_at", "updated_at", "created_at"])
            .unwrap_or("")
            .cmp(string_field(a, &["completed_at", "updated_at", "created_at"]).unwrap_or(""))
    });
    values.truncate(80);
    values
}

fn source_ref(source_id: &str, source_type: &str, label: &str, path: &Path) -> Value {
    json!({
        "source_id": source_id,
        "source_type": source_type,
        "label": label,
        "uri": path.display().to_string(),
    })
}

fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
}

fn sort_values_by_time_desc(values: &mut [Value], field: &str) {
    values.sort_by(|a, b| {
        b.get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(a.get(field).and_then(Value::as_str).unwrap_or(""))
    });
}

fn plane_for_event_type(event_type: &str) -> &'static str {
    if event_type.contains("result") || event_type.contains("evidence") {
        "evidence"
    } else if event_type.contains("request.created")
        || event_type.contains("message")
        || event_type.contains("mail")
        || event_type.contains("calendar")
        || event_type.contains("forum")
    {
        "intake"
    } else if event_type.contains("policy")
        || event_type.contains("worker")
        || event_type.contains("doctor")
        || event_type.contains("dispatch.")
    {
        "execution"
    } else {
        "intake"
    }
}

fn priority_for_status(status: &str) -> &'static str {
    match status {
        "failed" | "denied" | "error" => "high",
        "pending" | "running" => "normal",
        _ => "low",
    }
}

fn priority_for_severity(severity: &str) -> &'static str {
    match severity {
        "error" => "high",
        "warn" => "normal",
        _ => "low",
    }
}

fn rate_group_rows() -> Vec<Value> {
    let providers = provider_rows();
    let groups = [
        ("local", 1),
        ("google", 2),
        ("google_bonus", 3),
        ("anthropic", 4),
        ("openai", 5),
    ];
    groups
        .iter()
        .map(|(group, priority)| {
            let members = providers
                .iter()
                .filter(|provider| {
                    provider
                        .get("rate_group")
                        .and_then(Value::as_str)
                        .is_some_and(|rate_group| rate_group == *group)
                })
                .filter_map(|provider| provider.get("provider_id").and_then(Value::as_str))
                .collect::<Vec<_>>();
            let healthy = providers.iter().any(|provider| {
                provider
                    .get("rate_group")
                    .and_then(Value::as_str)
                    .is_some_and(|rate_group| rate_group == *group)
                    && provider
                        .get("status")
                        .and_then(Value::as_str)
                        .is_some_and(|status| status == "connected")
            });
            json!({
                "group_id": group,
                "priority": priority,
                "status": if healthy { "healthy" } else { "down" },
                "providers": members,
                "quota_state": {},
                "notes": "local runtime discovery",
            })
        })
        .collect()
}

fn worker_agent_rows() -> Vec<Value> {
    let workers = fs::read_to_string(state_dir().join("workers.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({"workers": []}));
    workers
        .get("workers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|worker| {
            let id = worker
                .get("worker_id")
                .and_then(Value::as_str)
                .unwrap_or("local-worker");
            json!({
                "agent_id": id,
                "parent_id": Value::Null,
                "status": "running",
                "role": worker.get("class").and_then(Value::as_str).unwrap_or("shell_machine"),
                "started_at": worker.get("last_heartbeat_utc").and_then(Value::as_str).unwrap_or("unknown"),
                "last_event_at": worker.get("last_heartbeat_utc").and_then(Value::as_str),
            })
        })
        .collect()
}

fn ollama_models_payload() -> Value {
    let registry = heiwa_provider::AccountRegistry::load();
    let stored_endpoint = heiwa_provider::detect::ollama::registered_endpoint(&registry.accounts);
    let Ok(endpoint) = heiwa_provider::detect::ollama::resolve_configured_endpoint(stored_endpoint)
    else {
        return json!({ "models": [] });
    };
    json!({ "models": tokio::task::block_in_place(|| ollama_models_for_endpoint(&endpoint)) })
}

fn ollama_models_for_endpoint(
    endpoint: &heiwa_provider::detect::ollama::OllamaEndpoint,
) -> Vec<Value> {
    let Ok(client) = reqwest::blocking::Client::builder()
        .connect_timeout(heiwa_provider::detect::ollama::ENDPOINT_CONNECT_TIMEOUT)
        .timeout(heiwa_provider::detect::ollama::ENDPOINT_REQUEST_TIMEOUT)
        .no_proxy()
        .build()
    else {
        return Vec::new();
    };
    client
        .get(endpoint.api_url("/api/tags"))
        .send()
        .ok()
        .and_then(|resp| resp.json::<Value>().ok())
        .and_then(|val| val.get("models").cloned())
        .and_then(|val| serde_json::from_value(val).ok())
        .unwrap_or_default()
}

fn hook_provider_rows() -> Vec<Value> {
    let home = crate::home::heiwa_home().unwrap_or_else(|| PathBuf::from("."));
    let runtime_root = crate::home::heiwa_runtime_dir();
    vec![
        json_hook_provider_row(
            "claude",
            "Claude Code",
            &home.join(".claude").join("settings.json"),
            Some(
                runtime_root
                    .join("generated")
                    .join("claude")
                    .join("settings.json"),
            ),
            &["PreToolUse", "UserPromptSubmit"],
            Some(
                runtime_root
                    .join("logs")
                    .join("policy")
                    .join("claude-runtime-safety.jsonl"),
            ),
            vec![
                "provider-owned-hook-api",
                "schema-requires-hookEventName",
                "heiwa-observes-and-hardens",
            ],
        ),
        json_hook_provider_row(
            "gemini",
            "Gemini CLI",
            &home.join(".gemini").join("settings.json"),
            Some(
                runtime_root
                    .join("generated")
                    .join("gemini")
                    .join("settings.json"),
            ),
            &["BeforeTool", "SessionStart"],
            Some(
                runtime_root
                    .join("logs")
                    .join("policy")
                    .join("gemini-runtime-policy.jsonl"),
            ),
            vec![
                "provider-owned-hook-api",
                "before-tool-policy",
                "session-bootstrap",
            ],
        ),
        codex_hook_provider_row(&home),
        json!({
            "provider_id": "antigravity",
            "display_name": "Antigravity",
            "status": "delegated",
            "config_path": home.join(".gemini").join("antigravity").display().to_string(),
            "generated_config_status": generated_file_status(&runtime_root.join("generated").join("antigravity").join("settings.json")),
            "audit_file": Value::Null,
            "events": [],
            "notes": [
                "inherits-gemini-posture",
                "separate-live-hook-registry-not-detected",
            ],
        }),
    ]
}

fn json_hook_provider_row(
    provider_id: &str,
    display_name: &str,
    config_path: &Path,
    generated_config_path: Option<PathBuf>,
    event_names: &[&str],
    audit_file: Option<PathBuf>,
    notes: Vec<&str>,
) -> Value {
    let events = hook_events_from_json_config(config_path, event_names);
    let command_count = events
        .iter()
        .filter_map(|event| event.get("hooks").and_then(Value::as_array))
        .map(Vec::len)
        .sum::<usize>();
    let missing_command = events
        .iter()
        .filter_map(|event| event.get("hooks").and_then(Value::as_array))
        .flatten()
        .any(|hook| {
            hook.get("command_exists")
                .and_then(Value::as_bool)
                .is_some_and(|exists| !exists)
        });
    let status = if !config_path.exists() {
        "unconfigured"
    } else if command_count == 0 || missing_command {
        "degraded"
    } else {
        "active"
    };
    let generated_config_status = generated_config_path
        .as_deref()
        .map(|path| generated_hook_status(config_path, path))
        .unwrap_or_else(|| "not_applicable".to_string());

    json!({
        "provider_id": provider_id,
        "display_name": display_name,
        "status": status,
        "config_path": config_path.display().to_string(),
        "generated_config_status": generated_config_status,
        "audit_file": audit_file.map(|path| Value::String(path.display().to_string())).unwrap_or(Value::Null),
        "events": events,
        "notes": notes,
    })
}

fn codex_hook_provider_row(home: &Path) -> Value {
    let config_path = home.join(".codex").join("config.toml");
    let runtime_root = crate::home::heiwa_runtime_dir();
    json!({
        "provider_id": "codex",
        "display_name": "Codex",
        "status": "unsupported",
        "config_path": config_path.display().to_string(),
        "generated_config_status": generated_file_status(&runtime_root.join("generated").join("codex").join("config.toml")),
        "audit_file": Value::Null,
        "events": [],
        "notes": [
            "native-hook-parity-not-detected",
            "phase-1-safety-launcher-only",
            "app-should-show-boundary-not-fake-parity",
        ],
    })
}

fn hooks_summary() -> Value {
    let rows = hook_provider_rows();
    let mut active = 0i64;
    let mut degraded = 0i64;
    let mut unconfigured = 0i64;
    let mut unsupported = 0i64;
    let mut delegated = 0i64;
    let mut event_count = 0i64;
    let mut command_count = 0i64;

    for row in &rows {
        match row
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
        {
            "active" => active += 1,
            "degraded" => degraded += 1,
            "unconfigured" => unconfigured += 1,
            "unsupported" => unsupported += 1,
            "delegated" => delegated += 1,
            _ => {}
        }
        if let Some(events) = row.get("events").and_then(Value::as_array) {
            event_count += events.len() as i64;
            command_count += events
                .iter()
                .filter_map(|event| event.get("hooks").and_then(Value::as_array))
                .map(|hooks| hooks.len() as i64)
                .sum::<i64>();
        }
    }

    json!({
        "source": "live-home-config",
        "providers": rows.len(),
        "active": active,
        "degraded": degraded,
        "unconfigured": unconfigured,
        "unsupported": unsupported,
        "delegated": delegated,
        "events": event_count,
        "commands": command_count,
    })
}

fn hook_events_from_json_config(config_path: &Path, event_names: &[&str]) -> Vec<Value> {
    let Some(config) = fs::read_to_string(config_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    else {
        return Vec::new();
    };
    let Some(hooks) = config.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut events = Vec::new();
    for event_name in event_names {
        let Some(entries) = hooks.get(*event_name).and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            let matcher = entry
                .get("matcher")
                .and_then(Value::as_str)
                .unwrap_or("*")
                .to_string();
            let hook_commands = entry
                .get("hooks")
                .and_then(Value::as_array)
                .map(|hooks| {
                    hooks
                        .iter()
                        .map(|hook| {
                            let command = hook
                                .get("command")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let command_path = hook_command_path(&command);
                            let command_exists =
                                command_path.as_deref().map(Path::new).map(Path::exists);
                            json!({
                                "name": hook.get("name").and_then(Value::as_str),
                                "kind": hook.get("type").and_then(Value::as_str),
                                "command": command,
                                "command_path": command_path,
                                "command_exists": command_exists,
                                "timeout_ms": hook.get("timeout").and_then(Value::as_i64),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            events.push(json!({
                "event": event_name,
                "matcher": matcher,
                "hooks": hook_commands,
            }));
        }
    }
    events
}

fn hook_command_path(command: &str) -> Option<String> {
    let home = crate::home::heiwa_home().unwrap_or_else(|| PathBuf::from("."));
    command
        .split_whitespace()
        .rev()
        .map(|token| token.trim_matches('"').trim_matches('\''))
        .find(|token| token.starts_with('/') || token.starts_with("~/"))
        .map(|token| {
            if let Some(rest) = token.strip_prefix("~/") {
                home.join(rest).display().to_string()
            } else {
                token.to_string()
            }
        })
}

fn generated_hook_status(live_path: &Path, generated_path: &Path) -> String {
    match (
        json_hook_fingerprint(live_path),
        json_hook_fingerprint(generated_path),
    ) {
        (Some(live), Some(generated)) if live == generated => "matches_hooks".to_string(),
        (Some(_), Some(_)) => "drift".to_string(),
        (Some(_), None) if generated_path.exists() => "unreadable_generated_hooks".to_string(),
        (Some(_), None) => "no_generated_config".to_string(),
        _ => generated_file_status(generated_path),
    }
}

fn json_hook_fingerprint(path: &Path) -> Option<String> {
    let parsed = fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())?;
    serde_json::to_string(parsed.get("hooks")?).ok()
}

fn generated_file_status(path: &Path) -> String {
    if path.exists() {
        "present-not-live-source".to_string()
    } else {
        "missing".to_string()
    }
}

fn worker_entry_is_live(entry: &Value, now: i64) -> bool {
    let last = entry
        .get("last_heartbeat_utc")
        .and_then(Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|timestamp| timestamp.timestamp())
        .unwrap_or(0);
    let ttl = entry
        .get("ttl_seconds")
        .and_then(Value::as_i64)
        .unwrap_or(HEARTBEAT_TTL_SECS);
    (now - last) <= ttl
}

fn write_app_heartbeat(runtime_state_dir: &Path, worker_id: &str) -> Result<()> {
    let path = runtime_state_dir.join("workers.json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut workers = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({"workers": []}));
    let now = chrono::Utc::now();
    let entry = json!({
        "worker_id": worker_id,
        "class": "shell_machine",
        "node": hostname_string(),
        "last_heartbeat_utc": now.to_rfc3339(),
        "ttl_seconds": HEARTBEAT_TTL_SECS,
        "transport": "localhost-http-websocket",
    });
    let arr = workers.as_object_mut().and_then(|obj| {
        obj.entry("workers")
            .or_insert(Value::Array(Vec::new()))
            .as_array_mut()
    });
    if let Some(arr) = arr {
        arr.retain(|worker| worker_entry_is_live(worker, now.timestamp()));
        if let Some(idx) = arr
            .iter()
            .position(|worker| worker.get("worker_id").and_then(Value::as_str) == Some(worker_id))
        {
            arr[idx] = entry;
        } else {
            arr.push(entry);
        }
    }
    fs::write(path, serde_json::to_string_pretty(&workers)?)?;
    Ok(())
}

fn detect_keep_awake() -> String {
    match which("caffeinate") {
        Some(path) => format!(
            "caffeinate-available:{}:used-while-heiwa-app-open",
            path.display()
        ),
        None => "caffeinate-not-found".to_string(),
    }
}

fn spawn_caffeinate() -> Option<Child> {
    let path = which("caffeinate")?;
    Command::new(path)
        .args(["-dimsu"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

fn stop_caffeinate(child: &mut Option<Child>) {
    if let Some(child) = child.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn open_url(url: &str) -> Result<()> {
    Command::new("/usr/bin/open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

fn which(bin: &str) -> Option<PathBuf> {
    let output = Command::new("/usr/bin/which").arg(bin).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn workers_summary(state_dir: &Path) -> Value {
    let workers_path = state_dir.join("workers.json");
    let raw = fs::read_to_string(&workers_path).ok();
    let parsed: Value = raw
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({"workers": []}));
    let entries = parsed
        .get("workers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    let mut live = 0i64;
    let mut runtime_live = 0i64;
    let mut task_live = 0i64;
    let mut stale = 0i64;
    for entry in &entries {
        if worker_entry_is_live(entry, now) {
            live += 1;
            if entry.get("class").and_then(Value::as_str) == Some("shell_machine") {
                runtime_live += 1;
            } else {
                task_live += 1;
            }
        } else {
            stale += 1;
        }
    }
    json!({
        "path": workers_path.display().to_string(),
        "live": live,
        "runtime_live": runtime_live,
        "task_live": task_live,
        "stale": stale,
        "total": entries.len(),
    })
}

fn approvals_summary(state_dir: &Path) -> Value {
    let requests = state_dir.join("dispatch").join("requests");
    let decisions = state_dir
        .join("dispatch")
        .join("approvals")
        .join("decisions");
    let pending =
        crate::cmd::approvals::scan_pending_requests_in(&requests, &decisions).len() as i64;
    let decided = count_json(&decisions);
    json!({
        "requests_dir": requests.display().to_string(),
        "decisions_dir": decisions.display().to_string(),
        "pending": pending,
        "decided": decided,
    })
}

fn count_json(dir: &Path) -> i64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            count += 1;
        }
    }
    count
}

fn mail_summary() -> Value {
    let home = crate::home::heiwa_home().unwrap_or_else(|| PathBuf::from("."));
    let data_dir = home.join("Library").join("Mail");
    let data_present = data_dir.exists();
    json!({
        "policy": "metadata-only-no-body",
        "data_dir": data_dir.display().to_string(),
        "data_present": data_present,
        "bridge_state": if data_present { "ready-for-metadata-probe" } else { "no-mail-data" },
    })
}

fn cockpit_static_root() -> PathBuf {
    let shell_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = shell_manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let override_root = env::var_os("HEIWA_COCKPIT_DIR").map(PathBuf::from);
    let portable_root = env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|parent| parent.join("cockpit")));
    cockpit_static_root_from(
        override_root.as_deref(),
        &heiwa_install::get_heiwa_dir(),
        portable_root.as_deref(),
        &repo_root,
    )
}

fn cockpit_static_root_from(
    override_root: Option<&Path>,
    install_root: &Path,
    portable_root: Option<&Path>,
    repo_root: &Path,
) -> PathBuf {
    if let Some(root) = override_root.filter(|root| root.join("index.html").is_file()) {
        return root.to_path_buf();
    }
    let installed = install_root.join("app").join("cockpit-current");
    if installed.join("index.html").is_file() {
        return installed;
    }
    if let Some(root) = portable_root.filter(|root| root.join("index.html").is_file()) {
        return root.to_path_buf();
    }
    let cockpit_dist = repo_root
        .join("apps")
        .join("heiwa_app")
        .join("clients")
        .join("cockpit")
        .join("dist");
    if cockpit_dist.join("index.html").is_file() {
        return cockpit_dist;
    }
    repo_root
        .join("apps")
        .join("heiwa_app")
        .join("clients")
        .join("web")
}

fn static_file_for(root: &Path, request_path: &str) -> PathBuf {
    let clean_path = request_path
        .trim_start_matches('/')
        .split('?')
        .next()
        .unwrap_or("");
    let safe = clean_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .all(|segment| {
            !Path::new(segment)
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        });
    if !safe || clean_path.is_empty() {
        return root.join("index.html");
    }
    let candidate = root.join(clean_path);
    if candidate.is_file() {
        return candidate;
    }
    if candidate.is_dir() {
        return candidate.join("index.html");
    }
    let html_candidate = root.join(format!("{clean_path}.html"));
    if html_candidate.is_file() {
        return html_candidate;
    }
    root.join("index.html")
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "css" => "text/css; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn request_method(request: &str) -> Option<&str> {
    request.lines().next()?.split_whitespace().next()
}

fn request_target(request: &str) -> Option<&str> {
    request.lines().next()?.split_whitespace().nth(1)
}

fn request_path(request: &str) -> Option<&str> {
    request_target(request)?.split('?').next()
}

fn query_param(target: &str, name: &str) -> Option<String> {
    let query = target.split_once('?')?.1;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(key) == name {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(value: &str) -> String {
    let mut out = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hi = hex_value(bytes[index + 1]);
                let lo = hex_value(bytes[index + 2]);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi << 4) | lo);
                    index += 3;
                } else {
                    out.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_websocket_request(request: &str) -> bool {
    request
        .lines()
        .any(|line| line.to_ascii_lowercase().starts_with("upgrade: websocket"))
}

fn header_value(request: &str, name: &str) -> Option<String> {
    let needle = format!("{name}:");
    request.lines().find_map(|line| {
        if line.to_ascii_lowercase().starts_with(&needle) {
            line.split_once(':')
                .map(|(_, value)| value.trim().to_string())
        } else {
            None
        }
    })
}

fn websocket_accept_key(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_GUID.as_bytes());
    BASE64.encode(hasher.finalize())
}

fn parse_port(args: &[String]) -> Result<u16> {
    match flag_value(args, "--port") {
        Some(raw) => raw
            .parse::<u16>()
            .map_err(|_| anyhow!("invalid --port value: {raw}")),
        None => Ok(DEFAULT_PORT),
    }
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter.next().cloned();
        }
        if let Some(rest) = arg.strip_prefix(&format!("{flag}=")) {
            return Some(rest.to_string());
        }
    }
    None
}

fn state_dir() -> PathBuf {
    crate::home::heiwa_state_dir()
}

fn hostname_string() -> String {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .unwrap_or_else(|| "unknown-host".to_string())
}

fn provider_display_name(provider: &str) -> &'static str {
    match provider {
        "ollama" => "Ollama",
        "gemini" => "Gemini CLI",
        "antigravity" => "Antigravity",
        "claude" => "Claude Code",
        "codex" => "Codex",
        _ => "Provider",
    }
}

pub(crate) fn auth_kind_label(kind: &heiwa_provider::AuthKind) -> &'static str {
    match kind {
        heiwa_provider::AuthKind::OauthCli => "oauth_cli",
        heiwa_provider::AuthKind::ApiKey => "api_key",
        heiwa_provider::AuthKind::RouterApi => "api_key",
        heiwa_provider::AuthKind::LocalRuntime => "local",
        heiwa_provider::AuthKind::CustomProfile => "subscription",
    }
}

fn cockpit_status(status: &str) -> &'static str {
    match status {
        "connected" | "running" => "connected",
        "installed_unverified" | "installed_stopped" => "degraded",
        "not_installed" => "unlinked",
        _ => "error",
    }
}

fn supported_lanes(provider: &str) -> Vec<&'static str> {
    match provider {
        "ollama" => vec!["local"],
        "claude" | "codex" | "gemini" | "antigravity" => vec!["oauth_cli"],
        _ => vec![],
    }
}

fn print_help() {
    println!("heiwa app");
    println!();
    println!("Usage:");
    println!("  heiwa app start [--port N] [--no-open]");
    println!("  heiwa app api get <path> [--port N]");
    println!("  heiwa app api post <path> --body JSON [--port N]");
    println!("  heiwa app update [--source github|checkout] [--dry-run]");
    println!("  heiwa app runtime status [--json]");
    println!("  heiwa app status [--json]");
    println!("  heiwa app [--json]");
    println!();
    println!("Starts or probes the local Heiwa.app cockpit runtime.");
}

fn print_update_help() {
    println!("heiwa app update");
    println!();
    println!("Usage:");
    println!("  heiwa app update [--source github|checkout] [--dry-run]");
    println!();
    println!("Defaults to GitHub Releases for user/runtime updates.");
    println!(
        "Use --source checkout only for explicit developer reinstall from the current checkout."
    );
}

fn print_api_help() {
    println!("heiwa app api");
    println!();
    println!("Usage:");
    println!("  heiwa app api get <path> [--port N] [--json]");
    println!("  heiwa app api post <path> --body JSON [--port N] [--json]");
    println!();
    println!("Programmatic bridge to the local Heiwa.app runtime APIs for sessions, calendar, capabilities, and subagent dispatch.");
}

fn print_start_help() {
    println!("heiwa app start");
    println!();
    println!("Usage:");
    println!("  heiwa app start [--port N] [--no-open]");
    println!();
    println!("Binds 127.0.0.1, serves the per-user browser console by default,");
    println!("starts caffeinate while running, and writes a worker heartbeat.");
    println!("Set HEIWA_STATE_DIR to redirect app worker heartbeat state for verification.");
}

#[cfg(test)]
mod app_readmodel_tests {
    use super::*;
    use heiwa_evidence::OperatorJournal;
    use heiwa_session::operator::{OperatorSessionService, StartTurnRequest};
    use tokio::sync::broadcast;

    #[test]
    fn cockpit_static_root_prefers_override_then_installed_release_assets() {
        let state = temp_state_dir("cockpit-static-root");
        let install_root = state.join("install");
        let installed = install_root.join("app").join("cockpit-current");
        let repo_root = state.join("repo");
        let portable = state.join("portable").join("cockpit");
        let checkout = repo_root
            .join("apps")
            .join("heiwa_app")
            .join("clients")
            .join("cockpit")
            .join("dist");
        let override_root = state.join("override");
        for root in [&installed, &portable, &checkout, &override_root] {
            fs::create_dir_all(root).expect("create cockpit root");
            fs::write(root.join("index.html"), b"<!doctype html>").expect("write cockpit index");
        }

        assert_eq!(
            cockpit_static_root_from(None, &install_root, Some(&portable), &repo_root),
            installed
        );
        assert_eq!(
            cockpit_static_root_from(
                Some(&override_root),
                &install_root,
                Some(&portable),
                &repo_root,
            ),
            override_root
        );
        fs::remove_file(installed.join("index.html")).expect("remove installed index");
        assert_eq!(
            cockpit_static_root_from(None, &install_root, Some(&portable), &repo_root),
            portable
        );

        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn checkout_app_bundle_plan_distinguishes_ready_and_missing_sources() {
        let ready = app_bundle_update_plan(&json!({"present": true}), true);
        assert_eq!(ready["wired"], true);
        assert_eq!(ready["status"], "ready");
        assert_eq!(ready["would_install"], true);
        assert_eq!(ready["will_install"], false);
        assert!(ready["blocker"].is_null());

        let missing = app_bundle_update_plan(&json!({"present": false}), false);
        assert_eq!(missing["wired"], true);
        assert_eq!(missing["status"], "not_built");
        assert_eq!(missing["would_install"], false);
        assert_eq!(missing["will_install"], false);
        assert!(missing["blocker"].is_string());
    }

    #[test]
    fn checkout_install_selects_the_bundled_macho_linker_on_apple_silicon() {
        let environment = checkout_cargo_environment_from(
            "macos",
            "aarch64",
            None,
            Some(std::ffi::OsString::from("-C target-cpu=native")),
            Some(Path::new("/toolchains/pinned")),
            true,
        )
        .expect("bundled linker environment");

        assert_eq!(environment.strategy, "rust_bundled_macho_linker");
        assert_eq!(
            environment.linker.as_deref(),
            Some(Path::new(
                "/toolchains/pinned/lib/rustlib/aarch64-apple-darwin/bin/rust-lld"
            ))
        );
        assert_eq!(
            environment.rustflags.as_deref(),
            Some(std::ffi::OsStr::new(
                "-C target-cpu=native -C linker-flavor=ld64.lld"
            ))
        );
        assert_eq!(
            environment.receipt()["strategy"],
            "rust_bundled_macho_linker"
        );
        let mut cargo = Command::new("cargo");
        environment.apply(&mut cargo);
        assert!(cargo.get_envs().any(|(key, value)| {
            key == APPLE_ARM64_LINKER_ENV
                && value == environment.linker.as_deref().map(Path::as_os_str)
        }));
        assert!(cargo.get_envs().any(|(key, value)| {
            key == "RUSTFLAGS" && value == environment.rustflags.as_deref()
        }));
    }

    #[test]
    fn checkout_install_preserves_an_explicit_operator_linker() {
        let environment = checkout_cargo_environment_from(
            "macos",
            "aarch64",
            Some(std::ffi::OsString::from("/operator/linker")),
            Some(std::ffi::OsString::from("-C target-cpu=native")),
            None,
            false,
        )
        .expect("operator linker environment");

        assert_eq!(environment.strategy, "operator_override");
        assert!(environment.linker.is_none());
        assert!(environment.rustflags.is_none());
        assert_eq!(environment.receipt()["operator_override"], true);
    }

    #[test]
    fn checkout_install_does_not_treat_an_empty_linker_as_an_override() {
        let environment = checkout_cargo_environment_from(
            "macos",
            "aarch64",
            Some(std::ffi::OsString::new()),
            None,
            Some(Path::new("/toolchains/pinned")),
            true,
        )
        .expect("bundled linker environment");

        assert_eq!(environment.strategy, "rust_bundled_macho_linker");
    }

    #[test]
    fn checkout_install_uses_host_defaults_off_apple_silicon() {
        let environment =
            checkout_cargo_environment_from("linux", "x86_64", None, None, None, false)
                .expect("host environment");

        assert_eq!(environment.strategy, "host_default");
        assert!(environment.linker.is_none());
        assert!(environment.rustflags.is_none());
    }

    #[test]
    fn checkout_install_rejects_a_missing_bundled_linker() {
        let error = checkout_cargo_environment_from(
            "macos",
            "aarch64",
            None,
            None,
            Some(Path::new("/toolchains/incomplete")),
            false,
        )
        .expect_err("missing linker must block checkout install");

        assert!(error.to_string().contains("rust-lld"));
        assert!(error.to_string().contains("pinned Rust toolchain"));
    }

    #[test]
    fn app_heartbeat_prunes_expired_worker_records() {
        let state = temp_state_dir("worker-prune");
        let now = chrono::Utc::now();
        fs::write(
            state.join("workers.json"),
            serde_json::to_vec_pretty(&json!({
                "workers": [
                    {
                        "worker_id": "expired-worker",
                        "last_heartbeat_utc": (now - chrono::Duration::minutes(10)).to_rfc3339(),
                        "ttl_seconds": 120
                    },
                    {
                        "worker_id": "live-peer",
                        "last_heartbeat_utc": now.to_rfc3339(),
                        "ttl_seconds": 120
                    }
                ]
            }))
            .expect("worker registry JSON"),
        )
        .expect("write worker registry");

        write_app_heartbeat(&state, "current-worker").expect("write current heartbeat");

        let workers: Value = serde_json::from_slice(
            &fs::read(state.join("workers.json")).expect("read worker registry"),
        )
        .expect("parse worker registry");
        let worker_ids = workers["workers"]
            .as_array()
            .expect("workers array")
            .iter()
            .filter_map(|worker| worker["worker_id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(worker_ids, vec!["live-peer", "current-worker"]);

        let _ = fs::remove_dir_all(state);
    }

    #[test]
    fn worker_summary_separates_runtime_hosts_from_task_workers() {
        let state = temp_state_dir("worker-classes");
        let now = chrono::Utc::now();
        fs::write(
            state.join("workers.json"),
            serde_json::to_vec_pretty(&json!({
                "workers": [
                    {
                        "worker_id": "heiwa-app-1",
                        "class": "shell_machine",
                        "last_heartbeat_utc": now.to_rfc3339(),
                        "ttl_seconds": 120
                    },
                    {
                        "worker_id": "task-worker-1",
                        "class": "model_worker",
                        "last_heartbeat_utc": now.to_rfc3339(),
                        "ttl_seconds": 120
                    },
                    {
                        "worker_id": "stale-task",
                        "class": "model_worker",
                        "last_heartbeat_utc": (now - chrono::Duration::minutes(10)).to_rfc3339(),
                        "ttl_seconds": 120
                    }
                ]
            }))
            .expect("worker registry JSON"),
        )
        .expect("write worker registry");

        let summary = workers_summary(&state);

        assert_eq!(summary["live"], 2);
        assert_eq!(summary["runtime_live"], 1);
        assert_eq!(summary["task_live"], 1);
        assert_eq!(summary["stale"], 1);
        let _ = fs::remove_dir_all(state);
    }

    #[test]
    fn browser_bootstrap_is_single_use_and_issues_expiring_http_only_session() {
        let mut sessions = BrowserSessionStore::default();
        let origin = "http://127.0.0.1:7474";
        let bootstrap = sessions.issue_bootstrap_at(1_000, origin);
        let session = sessions
            .consume_bootstrap_at(&bootstrap, 1_001, origin)
            .expect("fresh bootstrap must issue one browser session");

        assert!(sessions
            .consume_bootstrap_at(&bootstrap, 1_001, origin)
            .is_none());
        let cookie_name = browser_session_cookie_name(7474);
        assert!(sessions.authenticates_cookie_at(
            &format!("other=1; {cookie_name}={session}"),
            1_001,
            7474,
            origin,
        ));
        assert!(!sessions.authenticates_cookie_at(
            &format!("{cookie_name}={session}"),
            1_001,
            7475,
            "http://127.0.0.1:7475",
        ));
        assert!(!sessions.authenticates_cookie_at(
            &format!("{cookie_name}={session}"),
            1_001,
            7474,
            "http://127.0.0.1:7555",
        ));
        assert!(!sessions.authenticates_cookie_at(
            &format!("{cookie_name}={session}"),
            1_001 + BROWSER_SESSION_TTL_SECONDS + 1,
            7474,
            origin,
        ));
    }

    #[tokio::test]
    async fn browser_bootstrap_http_redirect_hides_capability_and_sets_port_cookie() {
        async fn request_once(
            target: Option<String>,
            browser_sessions: Arc<Mutex<BrowserSessionStore>>,
        ) -> (String, String) {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let target = target.unwrap_or_else(|| {
                let bootstrap = browser_sessions.lock().unwrap().issue_bootstrap_at(
                    chrono::Utc::now().timestamp(),
                    &format!("http://{address}"),
                );
                format!("/?heiwa_bootstrap={bootstrap}")
            });
            let client_target = target.clone();
            let client = tokio::spawn(async move {
                let mut client = TcpStream::connect(address).await.unwrap();
                client
                    .write_all(
                        format!(
                            "GET {client_target} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                let mut response = Vec::new();
                client.read_to_end(&mut response).await.unwrap();
                String::from_utf8(response).unwrap()
            });
            let (server, _) = listener.accept().await.unwrap();
            handle_connection(
                server,
                Arc::new("2026-07-24T00:00:00Z".to_string()),
                Arc::new(Mutex::new(LocalRequestReplayCache::default())),
                browser_sessions,
            )
            .await
            .unwrap();
            (client.await.unwrap(), target)
        }

        let browser_sessions = Arc::new(Mutex::new(BrowserSessionStore::default()));
        let (accepted, target) = request_once(None, browser_sessions.clone()).await;
        let bootstrap = query_param(&target, "heiwa_bootstrap").unwrap();
        assert!(accepted.starts_with("HTTP/1.1 303 See Other\r\n"));
        assert!(accepted.contains("\r\nLocation: /\r\n"));
        assert!(accepted.contains("Set-Cookie: heiwa_local_operator_"));
        assert!(accepted.contains("; HttpOnly; SameSite=Strict; Path=/; Max-Age="));
        assert!(!accepted.contains(&bootstrap));

        let (replay, _) = request_once(Some(target), browser_sessions).await;
        assert!(replay.starts_with("HTTP/1.1 401 Unauthorized\r\n"));
        assert!(replay.contains("invalid_browser_bootstrap"));
        assert!(!replay.contains(&bootstrap));
    }

    #[tokio::test]
    async fn runtime_snapshot_http_is_safe_inside_async_server_context() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let mut client = TcpStream::connect(address).await.unwrap();
            client
                .write_all(
                    format!(
                        "GET /api/v1/runtime/snapshot HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            String::from_utf8(response).unwrap()
        });
        let (server, _) = listener.accept().await.unwrap();

        handle_connection(
            server,
            Arc::new("2026-08-01T00:00:00Z".to_string()),
            Arc::new(Mutex::new(LocalRequestReplayCache::default())),
            Arc::new(Mutex::new(BrowserSessionStore::default())),
        )
        .await
        .unwrap();

        let response = client.await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        let body = response.split("\r\n\r\n").nth(1).expect("response body");
        let payload: Value = serde_json::from_str(body).expect("JSON snapshot");
        assert_eq!(payload["ok"], true);
    }

    #[test]
    fn ollama_models_payload_uses_resolved_override_not_live_default() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let override_url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let bytes = stream.read(&mut request).unwrap();
            let request = std::str::from_utf8(&request[..bytes]).unwrap();
            assert!(request.starts_with("GET /api/tags HTTP/1.1"), "{request}");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 31\r\nConnection: close\r\n\r\n{\"models\":[{\"name\":\"fixture\"}]}",
                )
                .unwrap();
        });
        let endpoint = heiwa_provider::detect::ollama::resolve_endpoint(
            heiwa_provider::detect::ollama::EndpointOverride::Value(&override_url),
            Some("http://127.0.0.1:11434"),
        )
        .unwrap();

        let models = ollama_models_for_endpoint(&endpoint);
        server.join().unwrap();
        assert_eq!(models, vec![json!({"name": "fixture"})]);
    }

    #[test]
    fn runtime_auth_classifier_defaults_api_mutations_closed() {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            assert!(is_runtime_authenticated_request(
                method,
                "/api/v1/future-action"
            ));
        }
        assert!(is_runtime_authenticated_request(
            "GET",
            "/api/v1/operator/threads"
        ));
        assert!(is_runtime_authenticated_request(
            "POST",
            "/api/v1/calendar/holds"
        ));
        assert!(!is_runtime_authenticated_request(
            "POST",
            "/api/v1/route/preview"
        ));
        assert!(!is_runtime_authenticated_request("GET", "/api/v1/status"));
        assert!(!is_runtime_authenticated_request("GET", "/"));
    }

    async fn written_status_line(status: u16) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move { TcpStream::connect(address).await.unwrap() });
        let (mut server, _) = listener.accept().await.unwrap();
        write_response(
            &mut server,
            status,
            "application/json",
            b"{}".to_vec(),
            false,
        )
        .await
        .unwrap();
        drop(server);
        let mut client = client.await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        String::from_utf8(response)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_string()
    }

    async fn read_raw_http_request(raw: &[u8]) -> Result<(String, String)> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let raw = raw.to_vec();
        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream.write_all(&raw).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        let (mut server, _) = listener.accept().await?;
        let result = read_http_request_and_body(&mut server)
            .await
            .map(|(request, body)| (request, String::from_utf8_lossy(&body).to_string()));
        client.await.unwrap();
        result
    }

    fn temp_state_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("heiwa-shell-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp state dir");
        dir
    }

    #[test]
    fn runtime_approval_summary_excludes_decided_requests() {
        let state = temp_state_dir("approval-summary");
        let requests = state.join("dispatch").join("requests");
        let decisions = state.join("dispatch").join("approvals").join("decisions");
        fs::create_dir_all(&requests).expect("create request directory");
        fs::create_dir_all(&decisions).expect("create decision directory");
        fs::write(
            requests.join("req_completed.json"),
            json!({"request_id":"req_completed"}).to_string(),
        )
        .expect("write approval request");
        fs::write(
            decisions.join("req_completed.json"),
            json!({"id":"req_completed","outcome":"approved"}).to_string(),
        )
        .expect("write approval decision");

        let summary = approvals_summary(&state);

        assert_eq!(summary["pending"], 0);
        assert_eq!(summary["decided"], 1);

        fs::remove_file(decisions.join("req_completed.json"))
            .expect("remove decision to expose pending request");
        assert_eq!(approvals_summary(&state)["pending"], 1);
        let _ = fs::remove_dir_all(&state);
    }

    fn test_operator_sessions(root: &Path) -> Arc<OperatorSessionService> {
        Arc::new(OperatorSessionService::new(
            OperatorJournal::new(root.to_path_buf()).expect("operator journal"),
        ))
    }

    async fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::spawn(async move { TcpStream::connect(address).await.unwrap() });
        let (server, _) = listener.accept().await.unwrap();
        (server, client.await.unwrap())
    }

    async fn read_server_ws_json(stream: &mut TcpStream) -> Value {
        let (opcode, payload) = read_server_ws_frame(stream).await;
        assert_eq!(opcode, 0x1, "server frame must be text");
        serde_json::from_slice(&payload).expect("JSON websocket frame")
    }

    async fn read_next_server_ws_event(stream: &mut TcpStream) -> Value {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(1), read_server_ws_json(stream))
                .await
                .expect("event became visible");
            if frame["type"] == "event" {
                return frame;
            }
            assert_eq!(frame["type"], "heartbeat", "unexpected frame: {frame}");
        }
    }

    async fn read_server_ws_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
        let mut header = [0_u8; 2];
        stream.read_exact(&mut header).await.expect("frame header");
        assert_eq!(header[0] & 0x80, 0x80, "server frame must be final");
        assert_eq!(header[1] & 0x80, 0, "server frame must not be masked");
        let length = match header[1] & 0x7f {
            value @ 0..=125 => value as usize,
            126 => {
                let mut bytes = [0_u8; 2];
                stream.read_exact(&mut bytes).await.unwrap();
                u16::from_be_bytes(bytes) as usize
            }
            127 => {
                let mut bytes = [0_u8; 8];
                stream.read_exact(&mut bytes).await.unwrap();
                usize::try_from(u64::from_be_bytes(bytes)).unwrap()
            }
            _ => unreachable!(),
        };
        let mut payload = vec![0_u8; length];
        stream
            .read_exact(&mut payload)
            .await
            .expect("frame payload");
        (header[0] & 0x0f, payload)
    }

    async fn write_masked_client_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) {
        assert!(payload.len() <= 125);
        let mask = [0x11_u8, 0x22, 0x33, 0x44];
        let mut frame = vec![0x80 | opcode, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % mask.len()]),
        );
        stream.write_all(&frame).await.unwrap();
    }

    fn test_ws_intervals() -> OperatorWebsocketIntervals {
        OperatorWebsocketIntervals {
            poll: Duration::from_millis(5),
            heartbeat: Duration::from_millis(80),
            write_timeout: Duration::from_secs(1),
        }
    }

    #[tokio::test]
    async fn operator_websocket_replays_resumes_and_polls_external_appends() {
        let root = tempfile::tempdir().unwrap();
        let sessions = test_operator_sessions(root.path());
        sessions
            .start_turn("ws-thread", StartTurnRequest::auto("request-1", "hello"))
            .expect("seed turn");
        let seeded = sessions.events_after("ws-thread", None, 100).unwrap();
        assert_eq!(seeded.events.len(), 3);

        let (_sender, receiver) = broadcast::channel(8);
        let (server, mut client) = tcp_pair().await;
        let loop_sessions = sessions.clone();
        let first = tokio::spawn(async move {
            operator_events_loop(
                server,
                loop_sessions,
                receiver,
                "ws-thread".to_string(),
                None,
                test_ws_intervals(),
            )
            .await
        });
        let mut cursors = Vec::new();
        for expected in ["thread_created", "turn_started", "user_message"] {
            let frame = read_server_ws_json(&mut client).await;
            assert_eq!(frame["type"], "event");
            assert_eq!(frame["event"]["event_type"], expected);
            cursors.push(frame["cursor"].as_str().unwrap().to_string());
        }
        assert_eq!(read_server_ws_json(&mut client).await["type"], "caught_up");
        first.abort();

        let (_sender, receiver) = broadcast::channel(8);
        let (server, mut resumed_client) = tcp_pair().await;
        let loop_sessions = sessions.clone();
        let after = cursors[0].clone();
        let resumed = tokio::spawn(async move {
            operator_events_loop(
                server,
                loop_sessions,
                receiver,
                "ws-thread".to_string(),
                Some(after),
                test_ws_intervals(),
            )
            .await
        });
        for expected in ["turn_started", "user_message"] {
            let frame = read_server_ws_json(&mut resumed_client).await;
            assert_eq!(frame["event"]["event_type"], expected);
        }
        assert_eq!(
            read_server_ws_json(&mut resumed_client).await["type"],
            "caught_up"
        );

        let external = test_operator_sessions(root.path());
        external
            .start_turn(
                "ws-thread",
                StartTurnRequest::auto("request-2", "from peer"),
            )
            .expect("external append");
        for expected in ["turn_started", "user_message"] {
            let frame = read_next_server_ws_event(&mut resumed_client).await;
            assert_eq!(frame["event"]["event_type"], expected);
        }
        let next = tokio::time::timeout(
            Duration::from_millis(200),
            read_server_ws_json(&mut resumed_client),
        )
        .await
        .expect("resumed stream heartbeat");
        assert_eq!(next["type"], "heartbeat");
        resumed.abort();
    }

    #[tokio::test]
    async fn operator_websocket_emits_injected_heartbeat_without_advancing_cursor() {
        let root = tempfile::tempdir().unwrap();
        let sessions = test_operator_sessions(root.path());
        let (_sender, receiver) = broadcast::channel(8);
        let (server, mut client) = tcp_pair().await;
        let task = tokio::spawn(async move {
            operator_events_loop(
                server,
                sessions,
                receiver,
                "heartbeat-thread".to_string(),
                None,
                OperatorWebsocketIntervals {
                    poll: Duration::from_millis(5),
                    heartbeat: Duration::from_millis(10),
                    write_timeout: Duration::from_secs(1),
                },
            )
            .await
        });
        assert_eq!(read_server_ws_json(&mut client).await["type"], "caught_up");
        let heartbeat = read_server_ws_json(&mut client).await;
        assert_eq!(heartbeat["type"], "heartbeat");
        assert!(heartbeat.get("cursor").is_none());
        task.abort();
    }

    #[tokio::test]
    async fn operator_websocket_reports_replaced_stream_cursor_then_closes() {
        let root = tempfile::tempdir().unwrap();
        let sessions = test_operator_sessions(root.path());
        sessions.ensure_thread("original-thread").unwrap();
        let old_cursor = sessions
            .events_after("original-thread", None, 10)
            .unwrap()
            .next_cursor
            .unwrap();

        let replacement_root = tempfile::tempdir().unwrap();
        let replacement = test_operator_sessions(replacement_root.path());
        replacement.ensure_thread("replacement-thread").unwrap();
        fs::copy(
            replacement_root.path().join("operator_events.jsonl"),
            root.path().join("operator_events.jsonl"),
        )
        .unwrap();

        let (_sender, receiver) = broadcast::channel(8);
        let (server, mut client) = tcp_pair().await;
        let task = tokio::spawn(async move {
            operator_events_loop(
                server,
                sessions,
                receiver,
                "original-thread".to_string(),
                Some(old_cursor),
                test_ws_intervals(),
            )
            .await
        });
        let frame = read_server_ws_json(&mut client).await;
        assert_eq!(frame["type"], "invalid_cursor");
        assert_eq!(frame["code"], "invalid_cursor");
        assert_eq!(frame["action"], "replay_from_start");
        assert!(task.await.unwrap().is_ok());
        let mut trailing = [0_u8; 1];
        assert_eq!(client.read(&mut trailing).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn operator_websocket_forwards_only_matching_transient_frames() {
        let root = tempfile::tempdir().unwrap();
        let sessions = test_operator_sessions(root.path());
        let (sender, receiver) = broadcast::channel(8);
        let (server, mut client) = tcp_pair().await;
        let task = tokio::spawn(async move {
            operator_events_loop(
                server,
                sessions,
                receiver,
                "wanted-thread".to_string(),
                None,
                OperatorWebsocketIntervals {
                    poll: Duration::from_millis(5),
                    heartbeat: Duration::from_millis(30),
                    write_timeout: Duration::from_secs(1),
                },
            )
            .await
        });
        assert_eq!(read_server_ws_json(&mut client).await["type"], "caught_up");
        sender
            .send(heiwa_shell::operator::OperatorStreamFrame::AssistantDelta {
                thread_id: "other-thread".to_string(),
                turn_id: "other-turn".to_string(),
                text: "do not forward".to_string(),
            })
            .unwrap();
        sender
            .send(heiwa_shell::operator::OperatorStreamFrame::AssistantDelta {
                thread_id: "wanted-thread".to_string(),
                turn_id: "wanted-turn".to_string(),
                text: "forward me".to_string(),
            })
            .unwrap();
        let frame = read_server_ws_json(&mut client).await;
        assert_eq!(frame["type"], "assistant_delta");
        assert_eq!(frame["turn_id"], "wanted-turn");
        assert_eq!(frame["text"], "forward me");
        let next = read_server_ws_json(&mut client).await;
        assert_eq!(next["type"], "heartbeat", "unrelated delta leaked: {next}");
        task.abort();
    }

    #[tokio::test]
    async fn operator_websocket_masked_close_terminates_promptly() {
        let root = tempfile::tempdir().unwrap();
        let sessions = test_operator_sessions(root.path());
        let (_sender, receiver) = broadcast::channel(8);
        let (server, mut client) = tcp_pair().await;
        let task = tokio::spawn(async move {
            operator_events_loop(
                server,
                sessions,
                receiver,
                "close-thread".to_string(),
                None,
                test_ws_intervals(),
            )
            .await
        });
        assert_eq!(read_server_ws_json(&mut client).await["type"], "caught_up");
        write_masked_client_frame(&mut client, 0x8, &1000_u16.to_be_bytes()).await;
        tokio::time::timeout(Duration::from_millis(50), task)
            .await
            .expect("close terminates without waiting for poll")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn operator_websocket_masked_ping_receives_pong() {
        let root = tempfile::tempdir().unwrap();
        let sessions = test_operator_sessions(root.path());
        let (_sender, receiver) = broadcast::channel(8);
        let (server, mut client) = tcp_pair().await;
        let task = tokio::spawn(async move {
            operator_events_loop(
                server,
                sessions,
                receiver,
                "ping-thread".to_string(),
                None,
                OperatorWebsocketIntervals {
                    poll: Duration::from_millis(50),
                    heartbeat: Duration::from_secs(10),
                    write_timeout: Duration::from_secs(1),
                },
            )
            .await
        });
        assert_eq!(read_server_ws_json(&mut client).await["type"], "caught_up");
        write_masked_client_frame(&mut client, 0x9, b"probe").await;
        let (opcode, payload) =
            tokio::time::timeout(Duration::from_millis(50), read_server_ws_frame(&mut client))
                .await
                .expect("pong is prompt");
        assert_eq!(opcode, 0xA);
        assert_eq!(payload, b"probe");
        task.abort();
    }

    #[tokio::test]
    async fn operator_websocket_oversized_control_frame_closes_safely() {
        let root = tempfile::tempdir().unwrap();
        let sessions = test_operator_sessions(root.path());
        let (_sender, receiver) = broadcast::channel(8);
        let (server, mut client) = tcp_pair().await;
        let task = tokio::spawn(async move {
            operator_events_loop(
                server,
                sessions,
                receiver,
                "invalid-control-thread".to_string(),
                None,
                test_ws_intervals(),
            )
            .await
        });
        assert_eq!(read_server_ws_json(&mut client).await["type"], "caught_up");
        // Control frames may never use extended lengths. The reader must
        // reject this header without waiting for the declared payload.
        client.write_all(&[0x89, 0xFE, 0x00, 0x7E]).await.unwrap();
        tokio::time::timeout(Duration::from_millis(50), task)
            .await
            .expect("invalid control closes promptly")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn operator_websocket_write_timeout_bounds_backpressure() {
        let (mut writer, _unread_peer) = tokio::io::duplex(64);
        let payload = "x".repeat(128 * 1024);
        let started = std::time::Instant::now();
        let error = write_operator_ws_text(&mut writer, &payload, Duration::from_millis(5))
            .await
            .expect_err("unread peer must time out");
        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    fn test_local_api_policy() -> LocalAppApiTransportPolicy {
        LocalAppApiTransportPolicy {
            connect_timeout: Duration::from_millis(50),
            write_timeout: Duration::from_millis(5),
            read_timeout: Duration::from_millis(5),
            max_response_bytes: 1024,
        }
    }

    #[tokio::test]
    async fn local_app_api_read_deadline_rejects_never_ending_response() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let token = "read-deadline-secret";
        let error = call_local_app_api_with_policy(
            "GET",
            "/api/v1/operator/threads",
            port,
            None,
            token,
            test_local_api_policy(),
        )
        .await
        .err()
        .expect("never-ending response must time out");
        server.abort();
        assert!(error.to_string().contains("read timed out"));
        assert!(!error.to_string().contains(token));
    }

    #[tokio::test]
    async fn local_app_api_response_cap_rejects_oversized_response() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            let body = "x".repeat(2048);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let token = "response-cap-secret";
        let error = call_local_app_api_with_policy(
            "GET",
            "/api/v1/operator/threads",
            port,
            None,
            token,
            test_local_api_policy(),
        )
        .await
        .err()
        .expect("oversized response must fail closed");
        server.await.unwrap();
        assert!(error.to_string().contains("response too large"));
        assert!(!error.to_string().contains(token));
    }

    #[tokio::test]
    async fn local_app_api_write_deadline_bounds_unread_peer() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let body = format!("\"{}\"", "x".repeat(8 * 1024 * 1024));
        let token = "write-deadline-secret";
        let error = call_local_app_api_with_policy(
            "POST",
            "/api/v1/operator/threads",
            port,
            Some(&body),
            token,
            test_local_api_policy(),
        )
        .await
        .err()
        .expect("unread peer must time out");
        server.abort();
        // The guarantee is that a peer which accepts and then goes silent
        // cannot hang this call, and that the resulting error never carries
        // the auth token. WHICH phase bounds it is platform-dependent and not
        // something this test should pin:
        //
        //   Unix    an 8 MiB write to an unread socket fills the send buffer
        //           and blocks -> "write timed out"
        //   Windows Winsock buffers the whole body, so the write COMPLETES,
        //           the call moves on to read, and the silent peer trips the
        //           read deadline instead -> "read timed out"
        //
        // Pinning the write phase is why this failed on every Windows runner.
        // The token assertion below is the security-critical one and stays
        // exact.
        let message = error.to_string();
        assert!(
            ["write timed out", "write failed", "read timed out"]
                .iter()
                .any(|expected| message.contains(expected)),
            "call must be bounded by a deadline, got: {message}"
        );
        assert!(!message.contains(token));
    }

    #[tokio::test]
    async fn http_reader_rejects_truncated_declared_body() {
        let request = b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\nhi";
        assert!(read_raw_http_request(request).await.is_err());
    }

    #[tokio::test]
    async fn http_reader_rejects_invalid_content_length() {
        let request = b"POST / HTTP/1.1\r\nContent-Length: nope\r\n\r\n";
        assert!(read_raw_http_request(request).await.is_err());
    }

    #[tokio::test]
    async fn http_reader_rejects_oversized_body_before_reading_it() {
        let request = b"POST / HTTP/1.1\r\nContent-Length: 10485761\r\n\r\n";
        assert!(read_raw_http_request(request).await.is_err());
    }

    #[tokio::test]
    async fn http_reader_rejects_overflowing_total_length() {
        let request = format!("POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n", usize::MAX);
        assert!(read_raw_http_request(request.as_bytes()).await.is_err());
    }

    #[tokio::test]
    async fn http_reader_rejects_duplicate_content_length_headers() {
        let request = b"POST / HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\nhi";
        assert!(read_raw_http_request(request).await.is_err());
    }

    #[tokio::test]
    async fn http_response_status_lines_use_standard_reason_phrases() {
        for (status, expected) in [
            (200, "HTTP/1.1 200 OK"),
            (201, "HTTP/1.1 201 Created"),
            (202, "HTTP/1.1 202 Accepted"),
            (204, "HTTP/1.1 204 No Content"),
            (400, "HTTP/1.1 400 Bad Request"),
            (401, "HTTP/1.1 401 Unauthorized"),
            (404, "HTTP/1.1 404 Not Found"),
            (405, "HTTP/1.1 405 Method Not Allowed"),
            (409, "HTTP/1.1 409 Conflict"),
            (500, "HTTP/1.1 500 Internal Server Error"),
            (503, "HTTP/1.1 503 Service Unavailable"),
        ] {
            assert_eq!(written_status_line(status).await, expected);
        }
    }

    #[test]
    fn api_payload_exposes_life_today_for_cockpit() {
        let payload =
            api_payload("/api/v1/life/today", "2026-05-26T00:00:00Z").expect("life today endpoint");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        let data = payload.get("data").expect("data envelope");
        assert_eq!(
            data.get("command").and_then(Value::as_str),
            Some("life today")
        );
        assert_eq!(
            data.get("timezone").and_then(Value::as_str),
            Some("America/Vancouver")
        );
        assert!(data.get("pending_approvals").is_some_and(Value::is_array));
        assert!(data
            .get("runtime")
            .and_then(|runtime| runtime.get("evidence_mode"))
            .is_some_and(Value::is_string));
    }

    #[test]
    fn api_payload_wires_life_social_route() {
        // ROUTE WIRING ONLY. This asserts the path is dispatched and returns
        // the standard envelope. It points HEIWA_STATE_DIR at an empty temp
        // root rather than reading ambient operator state. An earlier version
        // claimed to verify metadata-only enforcement and, under an empty HOME,
        // silently took the `available:false` branch and asserted nothing.
        //
        // Schema validation, forbidden fields, scalars, version and policy
        // mismatch, staleness and the reconnect error state are covered
        // hermetically in `cmd::life::social_projection_tests`, which injects
        // a path and writes its own fixtures.
        // Pointed at an empty temp state dir so the test never reads the
        // operator's real projection, and holding the shared env lock so it
        // cannot race the projection tests that also set HEIWA_STATE_DIR.
        let dir = tempfile::tempdir().expect("tempdir");
        let _guard = crate::cmd::life::social_projection_tests::StateDirGuard::set(dir.path());

        let payload = api_payload("/api/v1/life/social", "2026-05-26T00:00:00Z")
            .expect("life social endpoint");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        let data = payload.get("data").expect("data envelope");
        assert_eq!(
            data.get("command").and_then(Value::as_str),
            Some("life social")
        );
        // Deterministic: nothing is published in the temp dir.
        assert_eq!(data.get("available").and_then(Value::as_bool), Some(false));
        // Both branches answer, so a missing producer is a reportable state
        // rather than a 500.
        assert!(data.get("available").is_some_and(Value::is_boolean));
        assert!(data.get("contacts").is_some_and(Value::is_array));
        assert!(data.get("reconnect").is_some_and(Value::is_object));
    }

    #[test]
    fn api_payload_exposes_life_freshness_for_cockpit() {
        let payload = api_payload("/api/v1/life/freshness", "2026-05-26T00:00:00Z")
            .expect("life freshness endpoint");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        let data = payload.get("data").expect("data envelope");
        assert_eq!(
            data.get("command").and_then(Value::as_str),
            Some("life freshness")
        );
        assert!(data.get("stale_sources").is_some_and(Value::is_number));
        assert!(data.get("sources").is_some_and(Value::is_array));
    }

    #[test]
    fn files_tree_and_preview_are_read_only_text_surfaces() {
        let dir = temp_state_dir("files-readmodel");
        let file = dir.join("note.md");
        fs::write(&file, "# Heiwa\nfile preview works\n").expect("write preview file");

        let tree =
            files_tree_payload_from_target(&format!("/api/v1/files/tree?path={}", dir.display()))
                .expect("tree payload");
        assert_eq!(
            tree.get("command").and_then(Value::as_str),
            Some("files tree")
        );
        let entries = tree
            .get("entries")
            .and_then(Value::as_array)
            .expect("entries array");
        assert!(entries
            .iter()
            .any(|entry| entry.get("name").and_then(Value::as_str) == Some("note.md")));

        let preview = file_preview_payload_from_target(&format!(
            "/api/v1/files/preview?path={}",
            file.display()
        ))
        .expect("preview payload");
        assert_eq!(preview.get("kind").and_then(Value::as_str), Some("file"));
        assert_eq!(preview.get("binary").and_then(Value::as_bool), Some(false));
        assert!(preview
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| content.contains("file preview works")));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn browser_probe_normalizes_urls_without_fetching() {
        let probe = browser_probe_payload_from_target("/api/v1/browser/probe?url=example.com/docs")
            .expect("browser probe");
        assert_eq!(
            probe.get("url").and_then(Value::as_str),
            Some("https://example.com/docs")
        );
        assert_eq!(
            probe.get("mode").and_then(Value::as_str),
            Some("embedded_webview")
        );
    }

    #[test]
    fn query_param_percent_decodes_values() {
        assert_eq!(
            query_param("/api/v1/files/preview?path=%2Ftmp%2Fhello+world.md", "path"),
            Some("/tmp/hello world.md".to_string())
        );
    }

    #[test]
    fn api_payload_exposes_approvals_summary_for_cockpit() {
        let payload = api_payload("/api/v1/approvals/summary", "2026-05-26T00:00:00Z")
            .expect("approvals summary endpoint");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        let data = payload.get("data").expect("data envelope");
        assert!(data.get("pending_count").is_some_and(Value::is_number));
        assert!(data.get("pending").is_some_and(Value::is_array));
        assert!(data.get("requests_dir").is_some_and(Value::is_string));
    }

    #[test]
    fn receipts_payload_scans_known_local_receipt_lanes() {
        let state = temp_state_dir("receipt-readmodel");
        let calendar = state.join("calendar").join("receipts");
        let automations = state.join("automations").join("receipts");
        let promotion = state.join("evidence").join("promotion");
        fs::create_dir_all(&calendar).expect("create calendar receipts");
        fs::create_dir_all(&automations).expect("create automation receipts");
        fs::create_dir_all(&promotion).expect("create promotion receipts");
        fs::write(
            calendar.join("rcpt-calendar.json"),
            json!({
                "receipt_id": "rcpt-calendar",
                "kind": "calendar_hold_created",
                "created_at": "2026-06-12T09:00:00Z"
            })
            .to_string(),
        )
        .expect("write calendar receipt");
        fs::write(
            automations.join("rcpt-auto.json"),
            json!({
                "kind": "automation_execution_event",
                "event": "queued",
                "execution_id": "exec-1",
                "created_at": "2026-06-12T10:00:00Z"
            })
            .to_string(),
        )
        .expect("write automation receipt");
        fs::write(
            promotion.join("heiwa-app-update.json"),
            json!({
                "schema_version": "heiwa_promotion_receipt_v1",
                "receipt_id": "heiwa-app-update",
                "created_at": "2026-06-12T08:00:00Z"
            })
            .to_string(),
        )
        .expect("write promotion receipt");

        let payload = receipts_payload_for_state_dir(&state);
        assert_eq!(
            payload.get("command").and_then(Value::as_str),
            Some("receipts summary")
        );
        let counts = payload.get("counts").expect("counts");
        assert_eq!(counts.get("total").and_then(Value::as_u64), Some(3));
        assert_eq!(counts.get("calendar").and_then(Value::as_u64), Some(1));
        assert_eq!(counts.get("automations").and_then(Value::as_u64), Some(1));
        assert_eq!(counts.get("promotion").and_then(Value::as_u64), Some(1));
        let receipts = payload
            .get("receipts")
            .and_then(Value::as_array)
            .expect("receipts array");
        assert_eq!(receipts.len(), 3);
        assert_eq!(
            receipts[0].get("lane").and_then(Value::as_str),
            Some("automations")
        );
        assert_eq!(
            receipts[0].get("receipt_id").and_then(Value::as_str),
            Some("exec-1")
        );
        assert_eq!(
            receipts[1].get("lane").and_then(Value::as_str),
            Some("calendar")
        );
        assert_eq!(
            receipts[2].get("lane").and_then(Value::as_str),
            Some("promotion")
        );

        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn api_payload_exposes_receipts_for_cockpit() {
        let payload =
            api_payload("/api/v1/receipts", "2026-05-26T00:00:00Z").expect("receipts endpoint");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        let data = payload.get("data").expect("data envelope");
        assert_eq!(
            data.get("command").and_then(Value::as_str),
            Some("receipts summary")
        );
        assert!(data.get("counts").is_some_and(Value::is_object));
        assert!(data.get("receipts").is_some_and(Value::is_array));
    }

    #[test]
    fn dispatch_results_populate_history_runs_and_artifacts() {
        let state = temp_state_dir("history-readmodel");
        let results = state.join("dispatch").join("results");
        fs::create_dir_all(&results).expect("create results dir");
        fs::write(
            results.join("res_demo.json"),
            json!({
                "schema_version": "operator_dispatch_result_v1",
                "request_id": "req_demo",
                "result_id": "res_demo",
                "completed_at": "2026-05-24T12:00:00Z",
                "status": "denied",
                "executed_mode": "none",
                "adapter": "filesystem",
                "summary": "Denied unsafe filesystem write",
                "evidence_refs": ["evidence/2026-05-24/receipt.json"],
                "redaction_applied": true
            })
            .to_string(),
        )
        .expect("write dispatch result");

        let history = history_summary_for_state_dir(&state);
        let runs = history
            .get("recent_runs")
            .and_then(Value::as_array)
            .expect("recent_runs array");
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].get("mission_id").and_then(Value::as_str),
            Some("req_demo")
        );
        assert_eq!(
            runs[0].get("status").and_then(Value::as_str),
            Some("denied")
        );
        assert_eq!(
            runs[0].get("summary").and_then(Value::as_str),
            Some("Denied unsafe filesystem write")
        );
        let artifacts = history
            .get("artifacts")
            .and_then(Value::as_array)
            .expect("artifacts array");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(
            artifacts[0].get("id").and_then(Value::as_str),
            Some("evidence/2026-05-24/receipt.json")
        );

        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn dispatch_results_and_events_populate_inbox_items_with_sources() {
        let state = temp_state_dir("inbox-readmodel");
        let results = state.join("dispatch").join("results");
        let events = state.join("events");
        fs::create_dir_all(&results).expect("create results dir");
        fs::create_dir_all(&events).expect("create events dir");
        fs::write(
            results.join("res_demo.json"),
            json!({
                "schema_version": "operator_dispatch_result_v1",
                "request_id": "req_demo",
                "result_id": "res_demo",
                "completed_at": "2026-05-24T12:00:00Z",
                "status": "denied",
                "executed_mode": "none",
                "adapter": "network",
                "summary": "Blocked external network request",
                "evidence_refs": ["evidence/2026-05-24/network.json"],
                "redaction_applied": true
            })
            .to_string(),
        )
        .expect("write dispatch result");
        fs::write(
            events.join("events.jsonl"),
            format!(
                "{}\n",
                json!({
                    "schema_version": "operator_event_envelope_v1",
                    "event_id": "evt_demo",
                    "ts": "2026-05-24T12:01:00Z",
                    "event_type": "dispatch.policy.classified",
                    "severity": "warn",
                    "source": "operator-x",
                    "subject": "network request",
                    "redaction_applied": true,
                    "payload_ref": "dispatch/results/res_demo.json"
                })
            ),
        )
        .expect("write events log");

        let inbox = inbox_items_for_state_dir(&state);
        assert_eq!(inbox.len(), 2);
        assert_eq!(
            inbox[0]
                .get("source")
                .and_then(|s| s.get("source_type"))
                .and_then(Value::as_str),
            Some("event_log")
        );
        assert_eq!(
            inbox[0].get("plane").and_then(Value::as_str),
            Some("execution")
        );
        assert_eq!(
            inbox[1]
                .get("source")
                .and_then(|s| s.get("source_type"))
                .and_then(Value::as_str),
            Some("dispatch_result")
        );
        assert_eq!(
            inbox[1].get("plane").and_then(Value::as_str),
            Some("evidence")
        );
        assert_eq!(
            inbox[1]
                .get("receipt_refs")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );

        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn event_type_mapping_preserves_iee_flow_planes() {
        assert_eq!(plane_for_event_type("dispatch.request.created"), "intake");
        assert_eq!(
            plane_for_event_type("dispatch.policy.classified"),
            "execution"
        );
        assert_eq!(plane_for_event_type("dispatch.result.written"), "evidence");
    }

    #[test]
    fn scan_dispatch_ids_in_returns_json_file_stems() {
        let dir = temp_state_dir("dispatch-scan");
        fs::write(dir.join("req_alpha.json"), "{}").expect("write alpha");
        fs::write(dir.join("req_beta.json"), "{}").expect("write beta");
        fs::write(dir.join("ignore.txt"), "noop").expect("write decoy");

        let ids = scan_dispatch_ids_in(&dir);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("req_alpha"));
        assert!(ids.contains("req_beta"));
        assert!(!ids.contains("ignore"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_dispatch_ids_in_returns_empty_on_missing_dir() {
        let missing = env::temp_dir().join("heiwa-shell-dispatch-missing-{nope}");
        let ids = scan_dispatch_ids_in(&missing);
        assert!(ids.is_empty());
    }

    #[test]
    fn api_payload_exposes_goals_for_cockpit() {
        let payload = api_payload("/api/v1/goals", "2026-05-26T00:00:00Z").expect("goals endpoint");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        let data = payload.get("data").expect("data envelope");
        assert!(data.get("goals_dir").is_some_and(Value::is_string));
        assert!(data.get("goals").is_some_and(Value::is_array));
        assert!(data
            .get("counts")
            .and_then(|c| c.get("open"))
            .is_some_and(Value::is_number));
    }

    #[test]
    fn api_payload_exposes_compress_summary_for_cockpit() {
        let payload = api_payload("/api/v1/compress/summary", "2026-05-26T00:00:00Z")
            .expect("compress summary endpoint");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        let data = payload.get("data").expect("data envelope");
        assert!(data.get("receipts_dir").is_some_and(Value::is_string));
        assert!(data.get("count").is_some_and(Value::is_number));
        assert!(data
            .get("totals")
            .and_then(|t| t.get("cumulative_ratio"))
            .is_some_and(Value::is_number));
        assert!(data.get("recent").is_some_and(Value::is_array));
    }

    #[test]
    fn resource_api_payload_reports_snapshot_policy_and_admissions() {
        let payload =
            api_payload("/api/v1/resource", "2026-06-02T00:00:00Z").expect("resource endpoint");
        let data = payload.get("data").expect("data");

        assert!(
            data.get("snapshot")
                .and_then(|snapshot| snapshot.get("cpu_count"))
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 0),
            "resource snapshot should include cpu_count: {payload}"
        );
        assert!(
            data.get("policy")
                .and_then(|policy| policy.get("hard_load_ratio"))
                .and_then(Value::as_f64)
                .is_some_and(|hard| hard > 0.0),
            "resource policy should include load thresholds: {payload}"
        );
        assert!(
            data.get("admissions")
                .and_then(|admissions| admissions.get("local_model_large"))
                .is_some(),
            "resource admissions should include local_model_large: {payload}"
        );
        assert_ne!(
            data.get("sources")
                .and_then(|sources| sources.get("battery_percent"))
                .and_then(Value::as_str),
            Some("not_probed_v0"),
            "resource snapshot must perform a platform power probe: {payload}"
        );
        assert_ne!(
            data.get("sources")
                .and_then(|sources| sources.get("thermal_pressure"))
                .and_then(Value::as_str),
            Some("unknown_v0"),
            "resource snapshot must perform a platform thermal probe: {payload}"
        );
    }

    #[test]
    fn session_api_payload_reports_serving_port() {
        let payload = api_payload_for_port("/api/v1/session", "2026-06-02T00:00:00Z", 7475)
            .expect("session endpoint");
        let data = payload.get("data").expect("data");
        assert_eq!(
            data.get("app_url").and_then(Value::as_str),
            Some("http://127.0.0.1:7475/")
        );
    }

    #[test]
    fn runtime_snapshot_includes_resource_state() {
        let payload =
            api_payload("/api/v1/runtime/snapshot", "2026-06-02T00:00:00Z").expect("snapshot");
        let data = payload.get("data").expect("data");

        assert!(
            data.get("resource").is_some(),
            "runtime snapshot should include resource state: {payload}"
        );
        let machine = data
            .get("machine")
            .expect("runtime snapshot should identify local machine perspective");
        assert_eq!(machine["device_class"], "full_node");
        assert_eq!(machine["perspective"]["locality"], "local");
        assert_eq!(machine["perspective"]["execution_scope"], "this_device");
        assert_eq!(machine["perspective"]["data_scope"], "shared_user");
        assert_eq!(machine["perspective"]["sync_status"], "local_only");
        assert!(
            machine["hardware"]["logical_cpu_count"]
                .as_u64()
                .is_some_and(|count| count > 0),
            "machine perspective should include CPU resources: {payload}"
        );
    }

    #[test]
    fn machine_perspective_reads_the_mesh_state_rather_than_asserting_no_peers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let perspective = machine_perspective(dir.path());

        assert_eq!(perspective["locality"], "local");
        assert_eq!(perspective["execution_scope"], "this_device");
        assert_eq!(perspective["data_scope"], "shared_user");
        assert_eq!(perspective["sync_status"], "local_only");
        assert_eq!(perspective["enrolled_peer_count"], 0);
        assert!(
            perspective["node_id"].is_null(),
            "an un-enrolled machine has no node id: {perspective}"
        );
    }

    #[test]
    fn machine_perspective_surfaces_an_unreadable_mesh_registry() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            heiwa_mesh::peers::registry_path_in(dir.path()),
            "{ not json",
        )
        .expect("write corrupt registry");

        let perspective = machine_perspective(dir.path());
        assert_eq!(
            perspective["sync_status"], "unknown",
            "a machine that cannot read its own mesh state must not claim local_only: {perspective}"
        );
        assert_eq!(
            perspective["mesh_errors"][0]["code"],
            "peer_registry_unreadable"
        );
    }

    #[test]
    fn monitor_api_payload_combines_user_and_machine_ops() {
        let payload =
            api_payload("/api/v1/monitor", "2026-06-02T00:00:00Z").expect("monitor endpoint");
        let data = payload.get("data").expect("data");

        assert_eq!(
            data.get("schema_version").and_then(Value::as_str),
            Some("heiwa_monitor_v1")
        );
        assert!(
            data.get("machine_ops")
                .and_then(|machine| machine.get("resource"))
                .is_some(),
            "monitor payload should include machine resource state: {payload}"
        );
        assert!(
            data.get("user_ops")
                .and_then(|user| user.get("approvals"))
                .is_some(),
            "monitor payload should include user approval state: {payload}"
        );
        assert_eq!(
            data.get("safety")
                .and_then(|safety| safety.get("mode"))
                .and_then(Value::as_str),
            Some("read_only")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_macos_memory_pressure_free_percentage_as_available_bytes() {
        let raw = "\
The system has 25769803776 (1572864 pages with a page size of 16384).

System-wide memory free percentage: 53%
";

        let bytes =
            parse_macos_memory_pressure_available_bytes(raw).expect("parse memory_pressure output");

        assert_eq!(bytes, 13_657_996_001);
    }

    #[test]
    fn capability_catalogs_read_sanitized_local_state() {
        let state = temp_state_dir("capability-catalogs");
        let dir = state.join("capabilities");
        fs::create_dir_all(&dir).expect("create capabilities dir");
        fs::write(
            dir.join("local-capability-inventory-2026-06-03.json"),
            json!({
                "schema_version": "heiwa_local_capability_inventory_v1",
                "providers": [
                    {"provider": "gemini", "version": "0.38.2"}
                ],
                "codex_plugins_observed": ["Browser", "Chrome"],
                "codex_mcp_servers": ["figma", "notion", "node_repl"],
                "installed_apps_observed": ["Codex.app", "Claude.app", "Gemini.app"],
                "reference_sources": ["official.openai.agents-sdk", "official.ollama.api"],
                "integration_families": ["provider_apps", "mcp_servers", "local_models"],
                "runtime_targets": ["rust", "typescript", "wasm"],
                "performance_targets": ["microsecond_readmodel", "bounded_local_worker"],
                "next_runtime_targets": ["api_v1_capabilities_read_model"]
            })
            .to_string(),
        )
        .expect("write capability catalog");

        let payload = capabilities_payload_for_state_dir(&state);

        let catalogs = payload
            .get("catalogs")
            .and_then(Value::as_array)
            .expect("catalogs array");
        assert_eq!(catalogs.len(), 1);
        assert_eq!(
            catalogs[0].get("catalog_id").and_then(Value::as_str),
            Some("local-capability-inventory-2026-06-03")
        );
        assert_eq!(
            catalogs[0].get("schema_version").and_then(Value::as_str),
            Some("heiwa_local_capability_inventory_v1")
        );
        assert_eq!(
            catalogs[0]
                .get("codex_plugins_observed")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            catalogs[0].get("codex_mcp_servers").and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            catalogs[0]
                .get("installed_apps_observed")
                .and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            catalogs[0].get("reference_sources").and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            catalogs[0]
                .get("integration_families")
                .and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            catalogs[0].get("runtime_targets").and_then(Value::as_u64),
            Some(3)
        );
        assert_eq!(
            catalogs[0]
                .get("performance_targets")
                .and_then(Value::as_u64),
            Some(2)
        );

        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn capability_payload_exposes_tool_call_contract() {
        let state = temp_state_dir("capability-tools");
        let payload = capabilities_payload_for_state_dir(&state);

        let tools = payload
            .get("tools")
            .and_then(Value::as_array)
            .expect("capabilities payload must expose tool contracts");

        let fs_read = tools
            .iter()
            .find(|tool| tool.get("id").and_then(Value::as_str) == Some("fs.read"))
            .expect("fs.read tool contract");
        assert_eq!(
            fs_read.get("execution_state").and_then(Value::as_str),
            Some("executable")
        );
        assert_eq!(
            fs_read.get("risk_class").and_then(Value::as_str),
            Some("host_safe_readonly")
        );
        assert_eq!(
            fs_read.get("plane").and_then(Value::as_str),
            Some("evidence")
        );
        assert_eq!(
            fs_read.get("lease_required").and_then(Value::as_bool),
            Some(true)
        );

        let computer_use = tools
            .iter()
            .find(|tool| tool.get("id").and_then(Value::as_str) == Some("computer.use"))
            .expect("computer-use target contract");
        assert_eq!(
            computer_use.get("execution_state").and_then(Value::as_str),
            Some("target_only")
        );
        assert_eq!(
            computer_use.get("approval_class").and_then(Value::as_str),
            Some("approval_required")
        );

        let _ = fs::remove_dir_all(&state);
    }
}
