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

/// Connects to the bus and pumps chat prompts into a Claude subprocess, starting
/// it lazily on the first prompt and restarting it if the pipe breaks. Launch
/// failures are reported to the chat instead of dying silently.
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
    let Ok(publisher) = bus::connect("conductor-out", &config.tcp_addr).await else {
        return;
    };
    let mut stdin: Option<ChildStdin> = None;

    while let Some(message) = hearsay::next_message(&mut prompts).await {
        if message.topic != topics::CONDUCTOR_PROMPT {
            continue;
        }
        let Ok(prompt) = serde_json::from_str::<ConductorPrompt>(&message.payload) else {
            continue;
        };

        if stdin.is_none() {
            match start(&config).await {
                Ok(fresh) => stdin = Some(fresh),
                Err(error) => {
                    report(&publisher, &error).await;
                    continue;
                }
            }
        }

        let line = node::user_message_line(&prompt.text);
        let delivered = match stdin.as_mut() {
            Some(open) => write_line(open, &line).await,
            None => false,
        };
        if !delivered {
            match start(&config).await {
                Ok(mut fresh) => {
                    let _ = write_line(&mut fresh, &line).await;
                    stdin = Some(fresh);
                }
                Err(error) => {
                    stdin = None;
                    report(&publisher, &error).await;
                }
            }
        }
    }
}

async fn report(publisher: &hearsay::Client, message: &str) {
    bus::publish_conductor(
        publisher,
        &ConductorEvent {
            kind: OutputKind::Stderr,
            text: message.to_string(),
        },
    )
    .await;
}

async fn write_line(stdin: &mut ChildStdin, line: &str) -> bool {
    stdin.write_all(line.as_bytes()).await.is_ok()
        && stdin.write_all(b"\n").await.is_ok()
        && stdin.flush().await.is_ok()
}

async fn start(config: &HostConfig) -> Result<ChildStdin, String> {
    let mcp_url = format!("http://{}/mcp", config.mcp_addr);
    let mut command = conductor_command(&mcp_url);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        format!("could not launch the claude CLI ({error}). Make sure `claude` is on your PATH.")
    })?;
    let stdin = child.stdin.take().ok_or("claude stdin was not piped")?;
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
    Ok(stdin)
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
        .arg("bypassPermissions")
        .arg("--model")
        .arg("sonnet")
        .arg("--mcp-config")
        .arg(mcp_config)
        .arg("--allowed-tools")
        .arg("mcp__mobius")
        .arg("--append-system-prompt")
        .arg(SYSTEM_PROMPT);
    command
}
