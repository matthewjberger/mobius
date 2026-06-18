//! Repo recon: a Claude analyst that inspects the workspace against a goal and
//! proposes graphs the user can stage. It streams its progress (the files it
//! reads, the thinking) back through the orchestrator so the UI is never a silent
//! spinner, and runs on a faster model since recon does not need the top tier.

use std::process::Stdio;
use std::time::Duration;

use protocol::{AnalyzeResult, OutputKind, SuggestedGraph};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;

use crate::node;
use crate::orchestrator::Command;

const SYSTEM_PROMPT: &str = "You are a repository analyst for Mobius, which runs graphs of Claude Code agents that feed each other in loops. Inspect the working directory with Read, Glob, and Grep to understand the project. Then propose 2 to 4 distinct multi-agent workflows for the user's goal; if no goal is given, propose generally useful workflows for working on this repository (tests, review, docs, refactors). Reply with ONLY a JSON array and no other text. Each element is an object: {\"name\": a short title, \"rationale\": one sentence on why this fits this repo and goal, \"nodes\": [{\"id\": a short lowercase handle, \"role\": the agent's system prompt}], \"edges\": [{\"from\": a node id, \"to\": a node id, \"template\": the prompt sent to the `to` node, using {output} for the `from` node's output}], \"kickoff\": the concrete first instruction to send the entry node to start the loop, specific to this repository}. Keep each workflow to 2 to 4 nodes, make the roles concrete and specific to this repository, and wire the edges into a working loop.";

const MODEL: &str = "sonnet";
const TIMEOUT_SECS: u64 = 180;

/// Runs the analyst against `workspace` for `goal`, streaming progress, and sends
/// the result home.
pub async fn run(goal: String, workspace: String, commands: mpsc::UnboundedSender<Command>) {
    let result = match tokio::time::timeout(
        Duration::from_secs(TIMEOUT_SECS),
        analyze(&goal, &workspace, &commands),
    )
    .await
    {
        Ok(Ok(graphs)) => AnalyzeResult {
            goal,
            graphs,
            error: None,
        },
        Ok(Err(error)) => AnalyzeResult {
            goal,
            graphs: Vec::new(),
            error: Some(error),
        },
        Err(_) => AnalyzeResult {
            goal,
            graphs: Vec::new(),
            error: Some("the analyzer timed out".to_string()),
        },
    };
    let _ = commands.send(Command::AnalyzeDone(result));
}

async fn analyze(
    goal: &str,
    workspace: &str,
    commands: &mpsc::UnboundedSender<Command>,
) -> Result<Vec<SuggestedGraph>, String> {
    let mut command = TokioCommand::new("claude");
    command
        .arg("--print")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--permission-mode")
        .arg("bypassPermissions")
        .arg("--model")
        .arg(MODEL)
        .arg("--allowed-tools")
        .arg("Read")
        .arg("Glob")
        .arg("Grep")
        .arg("--append-system-prompt")
        .arg(SYSTEM_PROMPT)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !workspace.is_empty() {
        command.current_dir(workspace);
    }
    let mut child = command.spawn().map_err(|error| {
        format!("could not launch the claude CLI ({error}). Make sure `claude` is on your PATH.")
    })?;
    let mut stdin = child.stdin.take().ok_or("claude stdin was not piped")?;
    let stdout = child.stdout.take().ok_or("claude stdout was not piped")?;

    let stated = if goal.trim().is_empty() {
        "(no specific goal given)".to_string()
    } else {
        goal.to_string()
    };
    let prompt = format!(
        "Goal: {stated}\n\nInspect this repository and propose workflows that meet the goal."
    );
    let line = node::user_message_line(&prompt);
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| error.to_string())?;
    stdin.flush().await.map_err(|error| error.to_string())?;

    let mut answer = String::new();
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        for (kind, text) in node::normalize_stream_line(&line) {
            match kind {
                OutputKind::Assistant => {
                    answer.push_str(&text);
                    answer.push('\n');
                    let _ = commands.send(Command::AnalyzeProgress(text));
                }
                OutputKind::Thinking | OutputKind::Tool | OutputKind::Info => {
                    let _ = commands.send(Command::AnalyzeProgress(text));
                }
                OutputKind::Result => {
                    let source = if text.trim().is_empty() {
                        &answer
                    } else {
                        &text
                    };
                    return parse_graphs(source)
                        .ok_or_else(|| "the analyzer did not return any workflows".to_string());
                }
                _ => {}
            }
        }
    }
    Err("the analyzer ended without a result".to_string())
}

/// Pulls the JSON array out of the answer, tolerating prose or fences around it.
fn parse_graphs(text: &str) -> Option<Vec<SuggestedGraph>> {
    let start = text.find('[')?;
    let end = text.rfind(']')?;
    if end < start {
        return None;
    }
    serde_json::from_str::<Vec<SuggestedGraph>>(&text[start..=end]).ok()
}
