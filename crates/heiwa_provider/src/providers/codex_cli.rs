use crate::adapter::{Message, ProviderAdapter, StreamEvent, TokenUsage};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

/// CLI subprocess adapter for OpenAI Codex.
///
/// Wraps `codex exec --json [-m <model>] <prompt>`.
/// For ChatGPT-Plus subscription users authenticated via `codex login`.
pub struct CodexCliAdapter;

impl Default for CodexCliAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCliAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProviderAdapter for CodexCliAdapter {
    async fn send(
        &self,
        model: &str,
        messages: &[Message],
        stream_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let prompt: String = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let mut cmd = Command::new(crate::resolve_command_or_name("codex"));
        crate::adapter::configure_cli_command(&mut cmd);
        cmd.arg("exec")
            .arg("--json")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            // Progress belongs to Codex. An unread pipe can fill and block
            // inference, and raw diagnostics must not become assistant text.
            .stderr(Stdio::null());

        if !model.is_empty() {
            cmd.arg("-m").arg(model);
        }

        cmd.arg("--").arg(&prompt);

        // One terminal event for both the stream consumer and Result caller.
        // The child is reaped before success or failure is published.
        match run_codex(&mut cmd, &stream_tx).await {
            Ok(Some(usage)) => {
                let _ = stream_tx.send(StreamEvent::Done(usage)).await;
                Ok(())
            }
            Ok(None) => Ok(()), // Consumer cancelled; no receiver remains.
            Err(error) => {
                let message = format!("Codex execution failed: {error:#}");
                let _ = stream_tx.send(StreamEvent::Error(message.clone())).await;
                Err(anyhow!(message))
            }
        }
    }

    async fn interrupt(&self) -> Result<()> {
        Ok(())
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "gpt-5".to_string(),
            "gpt-5-codex".to_string(),
            "o3".to_string(),
        ]
    }
}

async fn run_codex(
    cmd: &mut Command,
    stream_tx: &mpsc::Sender<StreamEvent>,
) -> Result<Option<TokenUsage>> {
    let mut child = cmd.spawn().context("could not start Codex CLI")?;
    let result = read_codex(&mut child, stream_tx).await;
    if !matches!(result, Ok(Some(_))) {
        // kill() also waits. kill_on_drop remains the backstop when the
        // entire adapter future is aborted by its supervisor.
        child.kill().await.ok();
    }
    result
}

async fn read_codex(
    child: &mut Child,
    stream_tx: &mpsc::Sender<StreamEvent>,
) -> Result<Option<TokenUsage>> {
    let stdout = child.stdout.take().context("Codex stdout unavailable")?;
    let mut reader = BufReader::new(stdout).lines();
    let mut completion = None;

    loop {
        let line = tokio::select! {
            biased;
            _ = stream_tx.closed() => return Ok(None),
            line = reader.next_line() => line.context("could not read Codex output")?,
        };
        let Some(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let obj: serde_json::Value =
            serde_json::from_str(&line).map_err(|_| anyhow!("invalid Codex JSONL event"))?;
        let text = match obj.get("type").and_then(|t| t.as_str()) {
            // Current codex exec --json emits complete item snapshots. Only
            // final agent text is a token; reasoning and tool output are not.
            Some("item.completed") => obj.get("item").and_then(|item| {
                (item.get("type").and_then(|t| t.as_str()) == Some("agent_message"))
                    .then(|| item.get("text").and_then(|t| t.as_str()))
                    .flatten()
            }),
            Some("agent_message_delta") | Some("message_delta") => {
                obj.get("delta").and_then(|d| d.as_str())
            }
            Some("agent_message") | Some("message") => obj.get("text").and_then(|t| t.as_str()),
            Some("turn.completed")
            | Some("task_complete")
            | Some("turn_complete")
            | Some("result") => {
                completion = Some(extract_usage(&obj));
                None
            }
            Some("turn.failed") | Some("error") => {
                let message = obj
                    .pointer("/error/message")
                    .or_else(|| obj.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("provider reported failure");
                return Err(anyhow!("{message}"));
            }
            _ => None,
        };
        if let Some(text) = text {
            if stream_tx
                .send(StreamEvent::Token(text.to_string()))
                .await
                .is_err()
            {
                return Ok(None);
            }
        }
    }

    let status = tokio::select! {
        biased;
        _ = stream_tx.closed() => return Ok(None),
        status = child.wait() => status.context("could not reap Codex CLI")?,
    };
    if !status.success() {
        return Err(anyhow!("Codex CLI exited with {status}"));
    }
    completion
        .map(Some)
        .ok_or_else(|| anyhow!("Codex output ended without a completion event"))
}

fn extract_usage(result: &serde_json::Value) -> TokenUsage {
    let usage = result.get("usage").unwrap_or(result);
    TokenUsage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        output_tokens: usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        cache_read_tokens: usage
            .get("cached_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        cache_write_tokens: 0,
        cost_usd: usage
            .get("total_cost_usd")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    }
}
