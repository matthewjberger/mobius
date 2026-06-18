//! The conductor: a Claude subprocess pointed at the host's MCP endpoint, fed by
//! the chat prompts the page publishes and streaming its answers back to the
//! page. It drives the graph only through the mobius tools, so talking to it in
//! plain English is talking to the whole graph of agents.

use std::process::Stdio;

use protocol::{ConductorEvent, ConductorPrompt, OutputKind, topics};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};

use crate::{HostConfig, bus, node};

const SYSTEM_PROMPT: &str = "You are the conductor of a graph of Claude Code agents orchestrated by Mobius. You drive and inspect the graph only through the mobius MCP tools; you have no filesystem, shell, or web access of your own. Each node is a Claude agent with a role, a working directory, and a transcript; edges route one node's finished turns into another's input, forming loops. Default to designing, not running. When the user describes what they want, build the graph by staging it: set_workspace to the project they name (so every agent shares that repo's context), stage_node for each agent, and add_edge to wire their outputs into each other's inputs. Then describe the staged graph back to them and ask them to review it. Do NOT call execute until the user explicitly tells you to run or go; the design should sit there, visualized, until they approve it. Once running, use get_graph and list_nodes to see the graph, get_node and get_transcript to interrogate a node, send_prompt to kick off or nudge a node, pause_node, resume_node, and stop_node to control flow, and stop_all as a kill switch. spawn_node adds and starts a node in one step, only for live changes the user asks for. Always report what the graph is doing in plain language.";

/// Connects to the bus, starts the conductor subprocess, and pumps chat prompts
/// into its stdin. Restarts the subprocess if the pipe breaks.
pub async fn run(config: HostConfig) {
    let Ok(mut prompts) = bus::connect_subscribed(
        "conductor-in",
        &config.tcp_addr,
        &[topics::CONDUCTOR_PROMPT],
    )
    .await
    else {
        return;
    };
    let Some(mut stdin) = start(&config).await else {
        return;
    };

    while let Some(message) = hearsay::next_message(&mut prompts).await {
        if message.topic != topics::CONDUCTOR_PROMPT {
            continue;
        }
        let Ok(prompt) = serde_json::from_str::<ConductorPrompt>(&message.payload) else {
            continue;
        };
        let line = node::user_message_line(&prompt.text);
        if !write_line(&mut stdin, &line).await {
            let Some(fresh) = start(&config).await else {
                break;
            };
            stdin = fresh;
            let _ = write_line(&mut stdin, &line).await;
        }
    }
}

async fn write_line(stdin: &mut ChildStdin, line: &str) -> bool {
    stdin.write_all(line.as_bytes()).await.is_ok()
        && stdin.write_all(b"\n").await.is_ok()
        && stdin.flush().await.is_ok()
}

async fn start(config: &HostConfig) -> Option<ChildStdin> {
    let mcp_url = format!("http://{}/mcp", config.mcp_addr);
    let mut command = conductor_command(&mcp_url);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().ok()?;
    let stdin = child.stdin.take()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    if let Some(stdout) = stdout {
        let address = config.tcp_addr.clone();
        tokio::spawn(async move {
            let Ok(publisher) = bus::connect("conductor-stream", &address).await else {
                return;
            };
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                for (kind, text) in node::normalize_stream_line(&line) {
                    bus::publish_conductor(&publisher, &ConductorEvent { kind, text }).await;
                }
            }
        });
    }
    if let Some(stderr) = stderr {
        let address = config.tcp_addr.clone();
        tokio::spawn(async move {
            let Ok(publisher) = bus::connect("conductor-err", &address).await else {
                return;
            };
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                bus::publish_conductor(
                    &publisher,
                    &ConductorEvent {
                        kind: OutputKind::Stderr,
                        text: line,
                    },
                )
                .await;
            }
        });
    }

    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Some(stdin)
}

fn conductor_command(mcp_url: &str) -> Command {
    let mcp_config = serde_json::json!({
        "mcpServers": { "mobius": { "type": "http", "url": mcp_url } }
    })
    .to_string();
    let mut command = Command::new("claude");
    command
        .arg("--print")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--permission-mode")
        .arg("dontAsk")
        .arg("--allowed-tools")
        .arg("mcp__mobius__*")
        .arg("--disallowed-tools")
        .arg("Bash Edit Write Read WebFetch WebSearch Task Glob Grep NotebookEdit")
        .arg("--mcp-config")
        .arg(mcp_config)
        .arg("--append-system-prompt")
        .arg(SYSTEM_PROMPT);
    command
}
