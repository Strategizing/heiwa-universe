//! `heiwa-claims` — read and refresh the executable claim registry.
//!
//! Argument parsing is hand-rolled on purpose. This binary is meant to be
//! callable from a shell gate, from CI, and from a Rust surface without pulling
//! a CLI framework into the dependency graph of something a Stop hook runs.

use std::process::ExitCode;

use heiwa_claims::{evaluate, repo_root, verify, ClaimState, Registry};

const USAGE: &str = "\
heiwa-claims — executable claim registry

  list [--json]           show every claim and its computed state
  check [--json]          exit non-zero if any claim misses its required state
  show <claim_id>         show one claim in full
  verify <claim_id>|--all run verifiers and record evidence
  verifiers               list the allowlisted verifiers

Computed states: planned, implemented, verified, degraded, retired.
Manifests never declare state; it is derived from evidence bound to source.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("list");
    let json = args.iter().any(|a| a == "--json");

    match run(command, &args, json) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("heiwa-claims: {err}");
            ExitCode::from(2)
        }
    }
}

fn run(command: &str, args: &[String], json: bool) -> heiwa_claims::Result<ExitCode> {
    if matches!(command, "-h" | "--help" | "help") {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    if command == "verifiers" {
        for def in heiwa_claims::verifier::VERIFIERS {
            println!("{:<16} v{:<4} {}", def.id, def.version, def.description);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let root = repo_root()?;
    let registry = Registry::load(&root)?;

    match command {
        "list" | "check" => {
            let reports = evaluate(&root, &registry)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&reports)
                        .map_err(|e| heiwa_claims::ClaimError::Io(e.to_string()))?
                );
            } else {
                for report in &reports {
                    let mark = if report.satisfied { "ok  " } else { "MISS" };
                    println!(
                        "{mark} {:<34} {:<12} (needs {:<11}) {}",
                        report.claim_id,
                        report.state.as_str(),
                        format!("{:?}", report.required_state).to_lowercase(),
                        report.reason
                    );
                }
            }
            let unsatisfied = reports.iter().filter(|r| !r.satisfied).count();
            if command == "check" && unsatisfied > 0 {
                eprintln!(
                    "\n{unsatisfied} claim(s) do not meet their required state. \
                     Run `heiwa-claims verify <claim_id>` or withdraw the claim."
                );
                return Ok(ExitCode::FAILURE);
            }
            Ok(ExitCode::SUCCESS)
        }
        "show" => {
            let id = args.get(1).ok_or_else(|| {
                heiwa_claims::ClaimError::Params("show requires a claim_id".into())
            })?;
            let reports = evaluate(&root, &registry)?;
            let report = reports
                .iter()
                .find(|r| &r.claim_id == id)
                .ok_or_else(|| heiwa_claims::ClaimError::Params(format!("no claim `{id}`")))?;
            println!(
                "{}",
                serde_json::to_string_pretty(report)
                    .map_err(|e| heiwa_claims::ClaimError::Io(e.to_string()))?
            );
            Ok(ExitCode::SUCCESS)
        }
        "verify" => {
            let all = args.iter().any(|a| a == "--all");
            let targets: Vec<&heiwa_claims::Claim> = if all {
                registry.claims.iter().collect()
            } else {
                let id = args.get(1).ok_or_else(|| {
                    heiwa_claims::ClaimError::Params("verify requires a claim_id or --all".into())
                })?;
                vec![registry
                    .get(id)
                    .ok_or_else(|| heiwa_claims::ClaimError::Params(format!("no claim `{id}`")))?]
            };

            let mut failed = 0usize;
            for claim in targets {
                match verify(&root, claim) {
                    Ok(record) => {
                        let pass = record.result == heiwa_claims::VerifyResult::Pass;
                        if !pass {
                            failed += 1;
                        }
                        println!(
                            "{} {:<34} {}",
                            if pass { "pass" } else { "FAIL" },
                            claim.claim_id,
                            record.detail.lines().next().unwrap_or("")
                        );
                    }
                    Err(err) => {
                        failed += 1;
                        eprintln!("SKIP {:<34} {err}", claim.claim_id);
                    }
                }
            }
            Ok(if failed > 0 {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        other => {
            eprintln!("unknown command `{other}`\n\n{USAGE}");
            Ok(ExitCode::from(2))
        }
    }
}

// Keep the unused-import lint honest about the re-export surface this binary
// relies on staying public.
#[allow(dead_code)]
fn _state_is_public(s: ClaimState) -> &'static str {
    s.as_str()
}
