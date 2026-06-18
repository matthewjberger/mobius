//! Mobius: program prompt loops over Claude Code.
//!
//! Data-oriented throughout. The graph is a plain value; behavior is free
//! functions; the host is a handle, not an object that owns the world. A single
//! orchestrator task owns the agent subprocesses and the graph, fed by one
//! command channel; the bus carries snapshots and output streams to whoever is
//! watching.
//!
//! - [`open`] / [`start_host`] boot the broker, the orchestrator, the conductor,
//!   and the MCP endpoint, and return a [`Host`].
//! - [`spawn_node`], [`add_edge`], [`send_prompt`], [`stop_node`], and the rest
//!   drive the graph.
//! - [`snapshot`], [`get_node`], and [`transcript`] query it.
//! - [`run`] parks the host so a headless program stays alive.

mod analyzer;
mod bus;
mod conductor;
mod graph;
mod mcp;
mod node;
mod orchestrator;

use protocol::{Edge, GraphSnapshot, NodeId, NodeOutput, NodeSpec, NodeView};
use tokio::sync::{mpsc, oneshot};

use crate::orchestrator::Command;

pub use protocol::{
    ConductorEvent, ConductorPrompt, NodeStateUpdate, NodeStatus, OutputKind, Trigger, UiCommand,
    topics,
};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

/// The common imports for building and driving a graph.
pub mod prelude {
    pub use crate::graph::{edge_on_contains, edge_on_match, edge_on_turn_end, node_spec};
    pub use crate::{
        Host, HostConfig, Result, add_edge, execute, get_node, open, pause_node, remove_edge,
        resume_node, run, send_prompt, set_workspace, snapshot, spawn_node, stage_node, start_host,
        stop_all, stop_node, transcript,
    };
    pub use protocol::{Edge, GraphSnapshot, NodeSpec, NodeStatus, NodeView, Trigger};
}

/// Where the host's three listeners bind, and which optional pieces to run.
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// The hearsay broker for native peers.
    pub tcp_addr: String,
    /// The hearsay websocket listener the page connects to.
    pub ws_addr: String,
    /// The MCP HTTP endpoint the conductor and external Claude instances drive.
    pub mcp_addr: String,
    /// The directory new agents run in by default.
    pub workspace: String,
    /// Run the MCP server.
    pub with_mcp: bool,
    /// Run the conductor: a Claude subprocess driving the graph from chat.
    pub with_conductor: bool,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            tcp_addr: "127.0.0.1:9610".to_string(),
            ws_addr: "127.0.0.1:9611".to_string(),
            mcp_addr: "127.0.0.1:9612".to_string(),
            workspace: ".".to_string(),
            with_mcp: true,
            with_conductor: true,
        }
    }
}

/// A handle to a running host. Holds the broker alive and the command channel
/// every operation flows through.
pub struct Host {
    commands: mpsc::UnboundedSender<Command>,
    _broker: hearsay::Broker,
}

/// Boots a host on the default ports with the conductor and MCP server running.
pub async fn open() -> Result<Host> {
    start_host(HostConfig::default()).await
}

/// Boots a host: the broker, the websocket listener, the command bridge, the
/// orchestrator, and the optional MCP server and conductor.
pub async fn start_host(config: HostConfig) -> Result<Host> {
    let broker = hearsay::start_broker(&config.tcp_addr).await?;
    hearsay::start_websocket_listener(&broker, &config.ws_addr).await?;

    let (command_tx, command_rx) = mpsc::unbounded_channel();

    let publisher = bus::connect("orchestrator", &config.tcp_addr).await?;
    let bridge = bus::connect_subscribed("commands", &config.tcp_addr, &[topics::COMMAND]).await?;
    tokio::spawn(bus::forward_commands(bridge, command_tx.clone()));

    tokio::spawn(orchestrator::run(
        command_rx,
        command_tx.clone(),
        publisher,
        config.workspace.clone(),
        config.tcp_addr.clone(),
    ));

    if config.with_mcp {
        mcp::start(command_tx.clone(), config.mcp_addr.clone());
    }
    if config.with_conductor {
        tokio::spawn(conductor::run(config.clone()));
    }

    Ok(Host {
        commands: command_tx,
        _broker: broker,
    })
}

/// Adds a node to the design without starting it, and returns its id. The
/// staging step: build the whole graph as data, then [`execute`] it.
pub async fn stage_node(host: &Host, spec: NodeSpec) -> Result<NodeId> {
    let (reply, answer) = oneshot::channel();
    host.commands.send(Command::Stage {
        spec,
        reply: Some(reply),
    })?;
    answer.await?
}

/// Spawns subprocesses for every staged node. The execute step.
pub fn execute(host: &Host) -> Result<()> {
    host.commands.send(Command::Execute)?;
    Ok(())
}

/// Sets the directory new agents run in, so the whole graph shares project
/// context.
pub fn set_workspace(host: &Host, path: &str) -> Result<()> {
    host.commands.send(Command::SetWorkspace {
        path: path.to_string(),
    })?;
    Ok(())
}

/// Adds a node and starts it immediately, returning its id once it is up. For
/// live additions to a running graph; use [`stage_node`] plus [`execute`] to
/// design first.
pub async fn spawn_node(host: &Host, spec: NodeSpec) -> Result<NodeId> {
    let (reply, answer) = oneshot::channel();
    host.commands.send(Command::Spawn {
        spec,
        reply: Some(reply),
    })?;
    answer.await?
}

/// Kills an agent's subprocess and marks it done.
pub async fn stop_node(host: &Host, node: &str) -> Result<()> {
    host.commands.send(Command::Stop {
        node: node.to_string(),
    })?;
    Ok(())
}

/// Holds an agent's inbound edges until it is resumed.
pub fn pause_node(host: &Host, node: &str) -> Result<()> {
    host.commands.send(Command::Pause {
        node: node.to_string(),
    })?;
    Ok(())
}

/// Resumes a paused agent and delivers any prompts queued while it was paused.
pub fn resume_node(host: &Host, node: &str) -> Result<()> {
    host.commands.send(Command::Resume {
        node: node.to_string(),
    })?;
    Ok(())
}

/// The kill switch: stops every running agent at once.
pub fn stop_all(host: &Host) -> Result<()> {
    host.commands.send(Command::StopAll)?;
    Ok(())
}

/// Sends a one-off prompt straight to an agent's input.
pub fn send_prompt(host: &Host, node: &str, text: &str) -> Result<()> {
    host.commands.send(Command::SendPrompt {
        node: node.to_string(),
        text: text.to_string(),
    })?;
    Ok(())
}

/// Adds a routing edge so one agent's finished turns feed another's input.
pub fn add_edge(host: &Host, edge: Edge) -> Result<()> {
    host.commands.send(Command::AddEdge { edge })?;
    Ok(())
}

/// Removes a routing edge by id.
pub fn remove_edge(host: &Host, id: &str) -> Result<()> {
    host.commands
        .send(Command::RemoveEdge { id: id.to_string() })?;
    Ok(())
}

/// Reads the whole graph: every node's spec, status, turn count, and last output.
pub async fn snapshot(host: &Host) -> Result<GraphSnapshot> {
    let (reply, answer) = oneshot::channel();
    host.commands.send(Command::Snapshot { reply })?;
    Ok(answer.await?)
}

/// Reads one node's live state, or `None` if no such node.
pub async fn get_node(host: &Host, node: &str) -> Result<Option<NodeView>> {
    let (reply, answer) = oneshot::channel();
    host.commands.send(Command::GetNode {
        node: node.to_string(),
        reply,
    })?;
    Ok(answer.await?)
}

/// Reads one node's full normalized transcript.
pub async fn transcript(host: &Host, node: &str) -> Result<Vec<NodeOutput>> {
    let (reply, answer) = oneshot::channel();
    host.commands.send(Command::Transcript {
        node: node.to_string(),
        reply,
    })?;
    Ok(answer.await?)
}

/// Parks until Ctrl-C, keeping the host and its graph alive.
pub async fn run(_host: &Host) -> Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// Boots a host on its own runtime thread and keeps it alive for the life of the
/// process. For shells that own their own event loop, like the desktop webview.
pub fn launch(config: HostConfig) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("[mobius] failed to start the runtime: {error}");
                return;
            }
        };
        runtime.block_on(async move {
            match start_host(config).await {
                Ok(host) => {
                    let _host = host;
                    std::future::pending::<()>().await;
                }
                Err(error) => eprintln!("[mobius] failed to start the host: {error}"),
            }
        });
    });
}
