//! All page state, grouped as signals. `Copy`, so it threads into every
//! component and closure without cloning. Behavior is the free functions in
//! `bus.rs`; this is only data.

use std::collections::HashMap;

use leptos::prelude::*;
use protocol::{GraphSnapshot, NodeId, NodeOutput, NodeStatus, OutputKind, SuggestedGraph};

/// One line in the conductor chat. `mine` is the user's own message; otherwise it
/// is a normalized line from the conductor's stream.
#[derive(Clone)]
pub struct ChatEntry {
    pub mine: bool,
    pub kind: OutputKind,
    pub text: String,
}

#[derive(Clone, Copy)]
pub struct MobiusState {
    /// Whether the bus websocket is open.
    pub connected: RwSignal<bool>,
    /// The latest whole-graph snapshot the host published.
    pub snapshot: RwSignal<GraphSnapshot>,
    /// Each node's accumulated normalized output, for the inspector transcript.
    pub outputs: RwSignal<HashMap<NodeId, Vec<NodeOutput>>>,
    /// The node open in the inspector.
    pub selected: RwSignal<Option<NodeId>>,
    /// The conductor chat transcript.
    pub chat: RwSignal<Vec<ChatEntry>>,
    /// Whether the conductor is mid-turn, for the working indicator.
    pub chat_busy: RwSignal<bool>,
    /// Workflows the analyzer suggested for the current goal.
    pub suggestions: RwSignal<Vec<SuggestedGraph>>,
    /// Whether repo recon is in flight.
    pub analyzing: RwSignal<bool>,
    /// The last analyze error, if any.
    pub analyze_error: RwSignal<Option<String>>,
}

impl MobiusState {
    pub fn new() -> Self {
        Self {
            connected: RwSignal::new(false),
            snapshot: RwSignal::new(GraphSnapshot::default()),
            outputs: RwSignal::new(HashMap::new()),
            selected: RwSignal::new(None),
            chat: RwSignal::new(Vec::new()),
            chat_busy: RwSignal::new(false),
            suggestions: RwSignal::new(Vec::new()),
            analyzing: RwSignal::new(false),
            analyze_error: RwSignal::new(None),
        }
    }
}

impl Default for MobiusState {
    fn default() -> Self {
        Self::new()
    }
}

/// The CSS class for a node's status, shared by the graph and the inspector.
pub fn status_class(status: NodeStatus) -> &'static str {
    match status {
        NodeStatus::Staged => "staged",
        NodeStatus::Idle => "idle",
        NodeStatus::Running => "running",
        NodeStatus::Paused => "paused",
        NodeStatus::Waiting => "waiting",
        NodeStatus::Done => "done",
        NodeStatus::Failed => "failed",
    }
}

/// The CSS class for one normalized output or chat line.
pub fn kind_class(kind: OutputKind) -> &'static str {
    match kind {
        OutputKind::Prompt => "prompt",
        OutputKind::Assistant => "assistant",
        OutputKind::Thinking => "thinking",
        OutputKind::Tool => "tool",
        OutputKind::Result => "result",
        OutputKind::Stderr => "stderr",
        OutputKind::Info => "info",
    }
}

/// Truncates on a character boundary for snippets.
pub fn truncate(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        trimmed.to_string()
    } else {
        let kept: String = trimmed.chars().take(limit).collect();
        format!("{kept}...")
    }
}
