//! Repo recon: a one-shot Claude analyst that inspects the workspace against a
//! goal and proposes graphs the user can stage. The suggestions are published to
//! the bus for the page to render as one-click templates.

use std::process::Stdio;

use protocol::{AnalyzeResult, SuggestedGraph, topics};
use tokio::process::Command;

use crate::bus;

const SYSTEM_PROMPT: &str = "You are a repository analyst for Mobius, which runs graphs of Claude Code agents that feed each other in loops. Inspect the working directory with Read, Glob, and Grep to understand the project. Then, for the user's goal, propose 2 to 4 distinct multi-agent workflows that would accomplish it. Reply with ONLY a JSON array and no other text. Each element is an object: {\"name\": a short title, \"rationale\": one sentence on why this fits this repo and goal, \"nodes\": [{\"id\": a short lowercase handle, \"role\": the agent's system prompt}], \"edges\": [{\"from\": a node id, \"to\": a node id, \"template\": the prompt sent to the `to` node, using {output} for the `from` node's output}]}. Keep each workflow to 2 to 4 nodes, make the roles concrete and specific to this repository, and wire the edges into a working loop.";

/// Runs the analyst against `workspace` for `goal` and publishes the result.
pub async fn run(goal: String, workspace: String, tcp_addr: String) {
    let Ok(publisher) = bus::connect("analyzer", &tcp_addr).await else {
        return;
    };
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
    bus::publish(&publisher, topics::SUGGESTIONS, &result).await;
}

async fn analyze(goal: &str, workspace: &str) -> Result<Vec<SuggestedGraph>, String> {
    let prompt = format!(
        "Goal: {goal}\n\nInspect this repository and propose workflows that meet the goal."
    );
    let mut command = Command::new("claude");
    command
        .arg("--print")
        .arg("--output-format")
        .arg("json")
        .arg("--permission-mode")
        .arg("dontAsk")
        .arg("--allowed-tools")
        .arg("Read Glob Grep")
        .arg("--append-system-prompt")
        .arg(SYSTEM_PROMPT)
        .arg(prompt)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !workspace.is_empty() {
        command.current_dir(workspace);
    }
    let output = command
        .output()
        .await
        .map_err(|error| format!("failed to launch the analyzer: {error}"))?;
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
