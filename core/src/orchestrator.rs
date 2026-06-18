//! The orchestrator: one task that owns the graph and every agent subprocess.
//! Every mutation arrives as a [`Command`] on a single channel, so there is one
//! writer and the state is never contended. On each change it publishes a fresh
//! snapshot and streams every agent's output to the bus.
//!
//! The graph is a [`petgraph`] `StableDiGraph` whose node weights are the live
//! agents and whose edge weights are the routing rules. Stable indices survive
//! node removal, and the directed structure gives edge traversal, reachability,
//! and cycle detection for free as the orchestration grows.

use std::collections::HashMap;

use hearsay::Client;
use petgraph::Direction;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use protocol::{
    Edge, GraphSnapshot, NodeId, NodeOutput, NodeSpec, NodeStateUpdate, NodeStatus, NodeView,
    OutputKind, Trigger,
};
use regex::Regex;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, oneshot};

use crate::{Result, bus, node};

const TRANSCRIPT_LIMIT: usize = 500;

/// Everything that flows into the orchestrator: the public API, the bus bridge,
/// the MCP server, and the agent readers all speak this.
pub(crate) enum Command {
    Stage {
        spec: NodeSpec,
        reply: Option<oneshot::Sender<Result<NodeId>>>,
    },
    Execute,
    SetWorkspace {
        path: String,
    },
    Spawn {
        spec: NodeSpec,
        reply: Option<oneshot::Sender<Result<NodeId>>>,
    },
    Stop {
        node: NodeId,
    },
    Pause {
        node: NodeId,
    },
    Resume {
        node: NodeId,
    },
    SendPrompt {
        node: NodeId,
        text: String,
    },
    StopAll,
    AddEdge {
        edge: Edge,
    },
    RemoveEdge {
        id: String,
    },
    Snapshot {
        reply: oneshot::Sender<GraphSnapshot>,
    },
    GetNode {
        node: NodeId,
        reply: oneshot::Sender<Option<NodeView>>,
    },
    Transcript {
        node: NodeId,
        reply: oneshot::Sender<Vec<NodeOutput>>,
    },
    Republish,
    Stream {
        node: NodeId,
        kind: OutputKind,
        text: String,
    },
    TurnEnded {
        node: NodeId,
    },
    Exited {
        node: NodeId,
    },
}

impl From<protocol::UiCommand> for Command {
    fn from(ui: protocol::UiCommand) -> Self {
        use protocol::UiCommand;
        match ui {
            UiCommand::StageNode { spec } => Command::Stage { spec, reply: None },
            UiCommand::Execute => Command::Execute,
            UiCommand::SetWorkspace { path } => Command::SetWorkspace { path },
            UiCommand::SpawnNode { spec } => Command::Spawn { spec, reply: None },
            UiCommand::StopNode { node } => Command::Stop { node },
            UiCommand::PauseNode { node } => Command::Pause { node },
            UiCommand::ResumeNode { node } => Command::Resume { node },
            UiCommand::SendPrompt { node, text } => Command::SendPrompt { node, text },
            UiCommand::StopAll => Command::StopAll,
            UiCommand::AddEdge { edge } => Command::AddEdge { edge },
            UiCommand::RemoveEdge { edge } => Command::RemoveEdge { id: edge },
            UiCommand::RequestSnapshot => Command::Republish,
        }
    }
}

struct Agent {
    spec: NodeSpec,
    status: NodeStatus,
    turns: u32,
    last_output: String,
    turn_buffer: String,
    transcript: Vec<NodeOutput>,
    stdin: Option<ChildStdin>,
    child: Option<Child>,
    paused_queue: Vec<String>,
}

/// The orchestrator's whole state: the agent graph, the id-to-index lookup, and
/// the workspace new agents run in.
struct Graph {
    graph: StableDiGraph<Agent, Edge>,
    index: HashMap<NodeId, NodeIndex>,
    workspace: String,
}

impl Graph {
    fn new(workspace: String) -> Self {
        Self {
            graph: StableDiGraph::new(),
            index: HashMap::new(),
            workspace,
        }
    }

    fn agent(&self, id: &str) -> Option<&Agent> {
        self.index
            .get(id)
            .and_then(|node| self.graph.node_weight(*node))
    }

    fn agent_mut(&mut self, id: &str) -> Option<&mut Agent> {
        let node = *self.index.get(id)?;
        self.graph.node_weight_mut(node)
    }
}

/// The orchestrator loop. Owns the publish client and drains the command channel
/// until every sender is gone.
pub(crate) async fn run(
    mut commands: mpsc::UnboundedReceiver<Command>,
    command_tx: mpsc::UnboundedSender<Command>,
    bus: Client,
    workspace: String,
) {
    let mut state = Graph::new(workspace);

    while let Some(command) = commands.recv().await {
        match command {
            Command::Stage { spec, reply } => {
                let id = stage(&mut state, spec);
                if let Some(agent) = state.agent(&id) {
                    announce_state(&bus, agent).await;
                }
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(id));
                }
                publish_snapshot(&bus, &state).await;
            }
            Command::Execute => {
                let staged: Vec<NodeId> = state
                    .graph
                    .node_weights()
                    .filter(|agent| matches!(agent.status, NodeStatus::Staged))
                    .map(|agent| agent.spec.id.clone())
                    .collect();
                for id in staged {
                    if let Some(agent) = state.agent_mut(&id) {
                        let _ = launch_into(agent, &command_tx);
                    }
                    if let Some(agent) = state.agent(&id) {
                        announce_state(&bus, agent).await;
                    }
                }
                publish_snapshot(&bus, &state).await;
            }
            Command::SetWorkspace { path } => {
                state.workspace = path;
                publish_snapshot(&bus, &state).await;
            }
            Command::Spawn { spec, reply } => {
                let id = stage(&mut state, spec);
                let result = match state.agent_mut(&id) {
                    Some(agent) => launch_into(agent, &command_tx).map(|()| id.clone()),
                    None => Err("node vanished after staging".into()),
                };
                if let Some(agent) = state.agent(&id) {
                    announce_state(&bus, agent).await;
                }
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
                publish_snapshot(&bus, &state).await;
            }
            Command::Stop { node } => {
                if let Some(agent) = state.agent_mut(&node) {
                    if let Some(mut child) = agent.child.take() {
                        let _ = child.start_kill();
                    }
                    agent.stdin = None;
                    agent.status = NodeStatus::Done;
                    announce_state(&bus, agent).await;
                }
                publish_snapshot(&bus, &state).await;
            }
            Command::Pause { node } => {
                if let Some(agent) = state.agent_mut(&node) {
                    agent.status = NodeStatus::Paused;
                    announce_state(&bus, agent).await;
                }
                publish_snapshot(&bus, &state).await;
            }
            Command::Resume { node } => {
                let queued = match state.agent_mut(&node) {
                    Some(agent) => {
                        agent.status = NodeStatus::Waiting;
                        std::mem::take(&mut agent.paused_queue)
                    }
                    None => Vec::new(),
                };
                for text in queued {
                    if let Some(agent) = state.agent_mut(&node) {
                        deliver(&bus, &node, agent, &text).await;
                    }
                }
                publish_snapshot(&bus, &state).await;
            }
            Command::SendPrompt { node, text } => {
                if let Some(agent) = state.agent_mut(&node) {
                    deliver(&bus, &node, agent, &text).await;
                }
                publish_snapshot(&bus, &state).await;
            }
            Command::StopAll => {
                let ids: Vec<NodeId> = state
                    .graph
                    .node_weights()
                    .map(|agent| agent.spec.id.clone())
                    .collect();
                for id in ids {
                    if let Some(agent) = state.agent_mut(&id) {
                        if let Some(mut child) = agent.child.take() {
                            let _ = child.start_kill();
                        }
                        agent.stdin = None;
                        agent.paused_queue.clear();
                        if !matches!(agent.status, NodeStatus::Staged) {
                            agent.status = NodeStatus::Done;
                        }
                        announce_state(&bus, agent).await;
                    }
                }
                publish_snapshot(&bus, &state).await;
            }
            Command::AddEdge { edge } => {
                add_edge(&mut state, edge);
                publish_snapshot(&bus, &state).await;
            }
            Command::RemoveEdge { id } => {
                remove_edge(&mut state, &id);
                publish_snapshot(&bus, &state).await;
            }
            Command::Snapshot { reply } => {
                let _ = reply.send(snapshot_of(&state));
            }
            Command::GetNode { node, reply } => {
                let _ = reply.send(state.agent(&node).map(view_of));
            }
            Command::Transcript { node, reply } => {
                let transcript = state
                    .agent(&node)
                    .map(|agent| agent.transcript.clone())
                    .unwrap_or_default();
                let _ = reply.send(transcript);
            }
            Command::Republish => {
                publish_snapshot(&bus, &state).await;
            }
            Command::Stream { node, kind, text } => {
                bus::publish_output(&bus, &node, kind, &text).await;
                if let Some(agent) = state.agent_mut(&node) {
                    push_transcript(
                        agent,
                        NodeOutput {
                            node: node.clone(),
                            kind,
                            text: text.clone(),
                        },
                    );
                    if matches!(kind, OutputKind::Assistant) {
                        if !agent.turn_buffer.is_empty() {
                            agent.turn_buffer.push('\n');
                        }
                        agent.turn_buffer.push_str(&text);
                    }
                }
            }
            Command::TurnEnded { node } => {
                let fired = end_turn(&mut state, &node);
                if let Some(agent) = state.agent(&node) {
                    announce_state(&bus, agent).await;
                }
                for (target, prompt) in fired {
                    if let Some(agent) = state.agent_mut(&target) {
                        deliver(&bus, &target, agent, &prompt).await;
                    }
                }
                publish_snapshot(&bus, &state).await;
            }
            Command::Exited { node } => {
                if let Some(agent) = state.agent_mut(&node) {
                    if !matches!(agent.status, NodeStatus::Done) {
                        agent.status = if agent.turns > 0 {
                            NodeStatus::Done
                        } else {
                            NodeStatus::Failed
                        };
                    }
                    agent.stdin = None;
                    announce_state(&bus, agent).await;
                }
                publish_snapshot(&bus, &state).await;
            }
        }
    }
}

/// Adds a node to the graph as data, with no subprocess. Resolves a default or
/// "." working directory to the current workspace. Replacing an existing id kills
/// its subprocess first.
fn stage(state: &mut Graph, mut spec: NodeSpec) -> NodeId {
    if spec.cwd.is_empty() || spec.cwd == "." {
        spec.cwd = state.workspace.clone();
    }
    if let Some(old) = state.index.remove(&spec.id)
        && let Some(mut agent) = state.graph.remove_node(old)
        && let Some(mut child) = agent.child.take()
    {
        let _ = child.start_kill();
    }
    let id = spec.id.clone();
    let node = state.graph.add_node(Agent {
        spec,
        status: NodeStatus::Staged,
        turns: 0,
        last_output: String::new(),
        turn_buffer: String::new(),
        transcript: Vec::new(),
        stdin: None,
        child: None,
        paused_queue: Vec::new(),
    });
    state.index.insert(id.clone(), node);
    id
}

/// Spawns the subprocess for a staged agent and moves it to `Idle`.
fn launch_into(agent: &mut Agent, command_tx: &mpsc::UnboundedSender<Command>) -> Result<()> {
    let (child, stdin) = node::launch_node(&agent.spec, command_tx.clone())?;
    agent.child = Some(child);
    agent.stdin = Some(stdin);
    agent.status = NodeStatus::Idle;
    Ok(())
}

fn add_edge(state: &mut Graph, edge: Edge) {
    let (Some(from), Some(to)) = (
        state.index.get(&edge.from).copied(),
        state.index.get(&edge.to).copied(),
    ) else {
        return;
    };
    remove_edge(state, &edge.id);
    state.graph.add_edge(from, to, edge);
}

fn remove_edge(state: &mut Graph, id: &str) {
    let found = state
        .graph
        .edge_references()
        .find(|edge| edge.weight().id == id)
        .map(|edge| edge.id());
    if let Some(edge) = found {
        state.graph.remove_edge(edge);
    }
}

/// Records the finished turn for `node` and returns the prompts its outbound
/// edges fire, as `(target_id, rendered_prompt)`.
fn end_turn(state: &mut Graph, node: &str) -> Vec<(NodeId, String)> {
    let Some(source) = state.index.get(node).copied() else {
        return Vec::new();
    };
    let output = match state.graph.node_weight_mut(source) {
        Some(agent) => {
            agent.turns += 1;
            agent.last_output = std::mem::take(&mut agent.turn_buffer);
            agent.status = NodeStatus::Waiting;
            agent.last_output.clone()
        }
        None => return Vec::new(),
    };
    state
        .graph
        .edges_directed(source, Direction::Outgoing)
        .filter(|edge| trigger_matches(&edge.weight().trigger, &output))
        .filter_map(|edge| {
            let target = state.graph.node_weight(edge.target())?;
            Some((
                target.spec.id.clone(),
                render(&edge.weight().prompt_template, &output),
            ))
        })
        .collect()
}

async fn deliver(bus: &Client, node: &str, agent: &mut Agent, text: &str) {
    if matches!(agent.status, NodeStatus::Paused) {
        agent.paused_queue.push(text.to_string());
        return;
    }
    bus::publish_output(bus, node, OutputKind::Prompt, text).await;
    push_transcript(
        agent,
        NodeOutput {
            node: node.to_string(),
            kind: OutputKind::Prompt,
            text: text.to_string(),
        },
    );
    if let Some(stdin) = agent.stdin.as_mut() {
        let line = node::user_message_line(text);
        let ok = stdin.write_all(line.as_bytes()).await.is_ok()
            && stdin.write_all(b"\n").await.is_ok()
            && stdin.flush().await.is_ok();
        agent.status = if ok {
            NodeStatus::Running
        } else {
            agent.stdin = None;
            NodeStatus::Failed
        };
    }
    announce_state(bus, agent).await;
}

fn trigger_matches(trigger: &Trigger, output: &str) -> bool {
    match trigger {
        Trigger::OnTurnEnd => true,
        Trigger::OnContains { needle } => output.contains(needle),
        Trigger::OnMatch { pattern } => Regex::new(pattern)
            .map(|regex| regex.is_match(output))
            .unwrap_or(false),
    }
}

fn render(template: &str, output: &str) -> String {
    template.replace("{output}", output)
}

fn push_transcript(agent: &mut Agent, output: NodeOutput) {
    agent.transcript.push(output);
    if agent.transcript.len() > TRANSCRIPT_LIMIT {
        let excess = agent.transcript.len() - TRANSCRIPT_LIMIT;
        agent.transcript.drain(0..excess);
    }
}

fn view_of(agent: &Agent) -> NodeView {
    NodeView {
        spec: agent.spec.clone(),
        status: agent.status,
        turns: agent.turns,
        last_output: agent.last_output.clone(),
    }
}

fn snapshot_of(state: &Graph) -> GraphSnapshot {
    let mut nodes: Vec<NodeView> = state.graph.node_weights().map(view_of).collect();
    nodes.sort_by(|left, right| left.spec.id.cmp(&right.spec.id));
    let edges: Vec<Edge> = state.graph.edge_weights().cloned().collect();
    GraphSnapshot {
        workspace: state.workspace.clone(),
        nodes,
        edges,
    }
}

async fn announce_state(bus: &Client, agent: &Agent) {
    bus::publish_state(
        bus,
        &NodeStateUpdate {
            node: agent.spec.id.clone(),
            status: agent.status,
            turns: agent.turns,
        },
    )
    .await;
}

async fn publish_snapshot(bus: &Client, state: &Graph) {
    bus::publish_snapshot(bus, &snapshot_of(state)).await;
}
