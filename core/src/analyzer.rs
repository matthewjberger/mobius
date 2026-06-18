//! Repo recon: a one-shot Claude analyst that inspects the workspace against a
//! goal and proposes graphs the user can stage. The result is handed back to the
//! orchestrator, which publishes it on its stable client, so a suggestion is never
//! lost to a closing connection.

use std::process::Stdio;
use std::time::Duration;

use protocol::{AnalyzeResult, SuggestedGraph};
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;

use crate::orchestrator::Command;

const SYSTEM_PROMPT: &str = "You are a repository analyst for Mobius, which runs graphs of Claude Code agents that feed each other in loops. Inspect the working directory with Read, Glob, and Grep to understand the project. Then, for the user's goal, propose 2 to 4 distinct multi-agent workflows that would accomplish it. Reply with ONLY a JSON array and no other text. Each element is an object: {\"name\": a short title, \"rationale\": one sentence on why this fits this repo and goal, \"nodes\": [{\"id\": a short lowercase handle, \"role\": the agent's system prompt}], \"edges\": [{\"from\": a node id, \"to\": a node id, \"template\": the prompt sent to the `to` node, using {output} for the `from` node's output}]}. Keep each workflow to 2 to 4 nodes, make the roles concrete and specific to this repository, and wire the edges into a working loop.";

const TIMEOUT_SECS: u64 = 180;

/// Runs the analyst against `workspace` for `goal` and sends the result home.
pub async fn run(goal: String, workspace: String, commands: mpsc::UnboundedSender<Command>) {
    let result = match analyze(&goal, &workspace).await {
        Ok(graphs) => AnalyzeResult {
            goal,
            graphs,
            error: None,
        },
        Err(error) => AnalyzeResult {
            goal,
            graphs: Vec::new(),
            error: Some(error),
        },
    };
    let _ = commands.send(Command::AnalyzeDone(result));
}

async fn analyze(goal: &str, workspace: &str) -> Result<Vec<SuggestedGraph>, String> {
    let prompt = format!(
        "Goal: {goal}\n\nInspect this repository and propose workflows that meet the goal."
    );
    let mut command = TokioCommand::new("claude");
    command
        .arg("--print")
        .arg("--output-format")
        .arg("json")
        .arg("--permission-mode")
        .arg("bypassPermissions")
        .arg("--allowed-tools")
        .arg("Read")
        .arg("Glob")
        .arg("Grep")
        .arg("--append-system-prompt")
        .arg(SYSTEM_PROMPT)
        .arg(prompt)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !workspace.is_empty() {
        command.current_dir(workspace);
    }
    let output = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), command.output())
        .await
        .map_err(|_| "the analyzer timed out".to_string())?
        .map_err(|error| {
            format!(
                "could not launch the claude CLI ({error}). Make sure `claude` is on your PATH."
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("the analyzer exited with an error: {stderr}"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = result_text(&stdout).ok_or("could not read the analyzer's response")?;
    parse_graphs(&text).ok_or_else(|| "the analyzer did not return any workflows".to_string())
}

/// `--output-format json` wraps the answer as `{ "result": "<text>", ... }`.
fn result_text(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    value
        .get("result")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
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
