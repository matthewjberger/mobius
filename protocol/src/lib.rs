//! The wire format shared across Mobius's contexts.
//!
//! The host and the page never share memory; they share these types over the
//! hearsay bus, routed by topic. The host publishes [`GraphSnapshot`],
//! [`NodeOutput`], [`NodeStateUpdate`], and [`ConductorEvent`]; the page
//! publishes [`UiCommand`] and [`ConductorPrompt`]. Everything is serde, so the
//! same definitions compile to native and to wasm.

use serde::{Deserialize, Serialize};

/// The topics every context publishes and subscribes on.
pub mod topics {
    /// Host to page: a [`super::GraphSnapshot`] on every graph change and on request.
    pub const GRAPH: &str = "mobius/graph";
    /// Host to page: a [`super::NodeOutput`] per normalized agent stream event.
    pub const NODE_OUTPUT: &str = "mobius/nodes/output";
    /// Host to page: a [`super::NodeStateUpdate`] when an agent's status changes.
    pub const NODE_STATE: &str = "mobius/nodes/state";
    /// Host to page: the conductor's normalized stream, a [`super::ConductorEvent`].
    pub const CONDUCTOR_OUTPUT: &str = "mobius/conductor/output";
    /// Page to host: a [`super::ConductorPrompt`], the user's plain-English message.
    pub const CONDUCTOR_PROMPT: &str = "mobius/conductor/prompt";
    /// Page to host: a [`super::UiCommand`] to drive the graph from the UI.
    pub const COMMAND: &str = "mobius/command";
}

/// A node's identity on the graph. Caller-chosen, unique within a graph.
pub type NodeId = String;

/// The lifecycle state of an agent, for coloring the graph and gating edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    /// In the design, no subprocess yet. Becomes `Idle` on execute.
    Staged,
    /// Spawned, no turn taken yet.
    Idle,
    /// Mid-turn: a prompt is in flight and the agent is producing output.
    Running,
    /// Paused: inbound edges are held and queued until resumed.
    Paused,
    /// Finished a turn, waiting for the next prompt.
    Waiting,
    /// The subprocess exited cleanly.
    Done,
    /// The subprocess failed to launch or exited with an error.
    Failed,
}

/// The declaration of one Claude Code agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: NodeId,
    pub label: String,
    pub system_prompt: String,
    pub cwd: String,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
}

/// When an edge fires off its source node's finished turn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Trigger {
    /// Every time the source finishes a turn.
    OnTurnEnd,
    /// When the source's last output contains this substring.
    OnContains { needle: String },
    /// When the source's last output matches this regex.
    OnMatch { pattern: String },
}

/// A routing rule from one node's output to another node's input. The loop.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub from: NodeId,
    pub to: NodeId,
    pub trigger: Trigger,
    /// The prompt sent to `to`, with `{output}` replaced by `from`'s output.
    pub prompt_template: String,
}

/// One node's live state on a snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeView {
    pub spec: NodeSpec,
    pub status: NodeStatus,
    pub turns: u32,
    pub last_output: String,
}

/// The whole graph as the host publishes it: the authoritative picture the page
/// renders.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphSnapshot {
    /// The directory new agents run in by default, so they share project context.
    pub workspace: String,
    pub nodes: Vec<NodeView>,
    pub edges: Vec<Edge>,
}

/// The kind of a normalized stream event, shared by agents and the conductor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputKind {
    /// A prompt that was injected into the agent's input.
    Prompt,
    /// Assistant prose.
    Assistant,
    /// Extended-thinking text.
    Thinking,
    /// A tool call the agent made, summarized.
    Tool,
    /// A turn result line.
    Result,
    /// A line from the subprocess's stderr.
    Stderr,
    /// A host-emitted note (spawned, exited, edge fired).
    Info,
}

/// One normalized line of an agent's stream-json output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeOutput {
    pub node: NodeId,
    pub kind: OutputKind,
    pub text: String,
}

/// A change to an agent's status or turn count.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeStateUpdate {
    pub node: NodeId,
    pub status: NodeStatus,
    pub turns: u32,
}

/// One normalized line of the conductor's stream, for the chat pane.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConductorEvent {
    pub kind: OutputKind,
    pub text: String,
}

/// The user's plain-English message to the conductor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConductorPrompt {
    pub text: String,
}

/// An imperative the page issues to drive the graph, mirroring the buttons in the
/// node inspector and graph view. The conductor reaches the same operations
/// through the MCP tools instead.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum UiCommand {
    /// Add a node to the design without starting it. The staging step.
    StageNode {
        spec: NodeSpec,
    },
    /// Spawn subprocesses for every staged node. The execute step.
    Execute,
    /// Add a node and start it immediately (used for live additions).
    SpawnNode {
        spec: NodeSpec,
    },
    StopNode {
        node: NodeId,
    },
    PauseNode {
        node: NodeId,
    },
    ResumeNode {
        node: NodeId,
    },
    SendPrompt {
        node: NodeId,
        text: String,
    },
    /// The kill switch: stop every running agent at once.
    StopAll,
    AddEdge {
        edge: Edge,
    },
    RemoveEdge {
        edge: String,
    },
    /// Set the directory new agents run in, so the graph shares project context.
    SetWorkspace {
        path: String,
    },
    /// Ask the host to republish the current graph, to prime a freshly connected page.
    RequestSnapshot,
}
