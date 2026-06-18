//! One agent: a persistent `claude` subprocess in stream-json mode. Launching it
//! wires its stdout to the orchestrator as normalized [`OutputKind`] lines and a
//! turn-ended signal, and hands back the stdin and child so the orchestrator can
//! feed and kill it.

use std::process::Stdio;

use protocol::{NodeSpec, OutputKind};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command as TokioCommand};
use tokio::sync::mpsc;

use crate::Result;
use crate::orchestrator::Command;

const TOOL_INPUT_LIMIT: usize = 160;

/// Spawns the agent and the tasks that pump its output into `commands`. Returns
/// the child and its stdin.
pub fn launch_node(
    spec: &NodeSpec,
    commands: mpsc::UnboundedSender<Command>,
) -> Result<(Child, ChildStdin)> {
    let mut command = node_command(spec);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdin = child.stdin.take().ok_or("claude stdin was not piped")?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    if let Some(stdout) = stdout {
        let commands = commands.clone();
        let node = spec.id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                for (kind, text) in normalize_stream_line(&line) {
                    let ended = matches!(kind, OutputKind::Result);
                    let _ = commands.send(Command::Stream {
                        node: node.clone(),
                        kind,
                        text,
                    });
                    if ended {
                        let _ = commands.send(Command::TurnEnded { node: node.clone() });
                    }
                }
            }
            let _ = commands.send(Command::Exited { node });
        });
    }
    if let Some(stderr) = stderr {
        let commands = commands.clone();
        let node = spec.id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = commands.send(Command::Stream {
                    node: node.clone(),
                    kind: OutputKind::Stderr,
                    text: line,
                });
            }
        });
    }

    Ok((child, stdin))
}

/// The stream-json envelope wrapping a prompt for an agent's stdin.
pub fn user_message_line(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] }
    })
    .to_string()
}

/// Maps one stream-json line to zero or more normalized output lines. A `result`
/// line is emitted as [`OutputKind::Result`], which the orchestrator reads as the
/// end of a turn.
pub fn normalize_stream_line(line: &str) -> Vec<(OutputKind, String)> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match value.get("type").and_then(Value::as_str) {
        Some("system") if value.get("subtype").and_then(Value::as_str) == Some("init") => {
            let model = value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("a model");
            out.push((OutputKind::Info, format!("session started ({model})")));
        }
        Some("assistant") => {
            if let Some(content) = value.pointer("/message/content").and_then(Value::as_array) {
                for block in content {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(Value::as_str)
                                && !text.trim().is_empty()
                            {
                                out.push((OutputKind::Assistant, text.to_string()));
                            }
                        }
                        Some("thinking") => {
                            if let Some(text) = block.get("thinking").and_then(Value::as_str)
                                && !text.trim().is_empty()
                            {
                                out.push((OutputKind::Thinking, text.to_string()));
                            }
                        }
                        Some("tool_use") => {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                            let arguments = block
                                .get("input")
                                .map(|input| truncate(&input.to_string(), TOOL_INPUT_LIMIT))
                                .unwrap_or_default();
                            out.push((OutputKind::Tool, format!("{name} {arguments}")));
                        }
                        _ => {}
                    }
                }
            }
        }
        Some("result") => {
            let text = value
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            out.push((OutputKind::Result, text));
        }
        _ => {}
    }
    out
}

fn node_command(spec: &NodeSpec) -> TokioCommand {
    let mut command = TokioCommand::new("claude");
    command
        .arg("--print")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--permission-mode")
        .arg("bypassPermissions");
    if !spec.system_prompt.is_empty() {
        command
            .arg("--append-system-prompt")
            .arg(&spec.system_prompt);
    }
    if !spec.allowed_tools.is_empty() {
        command.arg("--allowed-tools");
        for tool in &spec.allowed_tools {
            command.arg(tool);
        }
    }
    if let Some(model) = &spec.model {
        command.arg("--model").arg(model);
    }
    if !spec.cwd.is_empty() {
        command.current_dir(&spec.cwd);
    }
    command
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        let kept: String = text.chars().take(limit).collect();
        format!("{kept}...")
    }
}
