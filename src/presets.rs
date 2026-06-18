//! Built-in loops. Each preset is plain data: a set of node roles and the edges
//! that wire them into a loop. Applying one stages the whole graph in one click,
//! ready to point at a workspace and execute.

use protocol::{Edge, NodeSpec, SuggestedGraph, Trigger, UiCommand};

use crate::bus::{self, Bus};

/// A named, ready-to-stage graph.
pub struct Preset {
    pub name: &'static str,
    pub blurb: &'static str,
    /// `(id, system prompt)` for each node.
    pub nodes: &'static [(&'static str, &'static str)],
    /// `(from, to, prompt template)` for each edge, fired on every turn.
    pub edges: &'static [(&'static str, &'static str, &'static str)],
}

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Implement & Review",
        blurb: "An implementer makes the change; a reviewer pushes back until it is solid. They loop.",
        nodes: &[
            (
                "implementer",
                "You implement the requested change in the working directory in small, focused steps. Report what you changed.",
            ),
            (
                "reviewer",
                "You review the implementer's latest work and push back hard on anything weak. If it is solid, begin your reply with APPROVED.",
            ),
        ],
        edges: &[
            ("implementer", "reviewer", "Review this work:\n{output}"),
            ("reviewer", "implementer", "Address this review:\n{output}"),
        ],
    },
    Preset {
        name: "Plan, Build, Test",
        blurb: "A planner breaks down the task, a builder implements each step, a tester verifies and sends failures back.",
        nodes: &[
            (
                "planner",
                "You break the requested task into a short ordered list of concrete steps for a builder. Keep it tight.",
            ),
            (
                "builder",
                "You implement the next step of the plan in the working directory and report what you did.",
            ),
            (
                "tester",
                "You build and test the project. If something fails, explain the failure clearly; if everything passes, begin with PASS.",
            ),
        ],
        edges: &[
            (
                "planner",
                "builder",
                "Here is the plan. Implement the next step:\n{output}",
            ),
            ("builder", "tester", "Verify this change:\n{output}"),
            ("tester", "builder", "Fix what the tests found:\n{output}"),
        ],
    },
    Preset {
        name: "Research, Draft, Critique",
        blurb: "A researcher gathers context, a writer drafts, a critic tightens it and loops back.",
        nodes: &[
            (
                "researcher",
                "You gather the relevant facts and context for the task from the working directory and summarize them.",
            ),
            (
                "writer",
                "You write a clear, concrete draft answering the task, using the research provided.",
            ),
            (
                "critic",
                "You critique the draft for accuracy and clarity and ask for specific revisions. If it is excellent, begin with SHIP.",
            ),
        ],
        edges: &[
            (
                "researcher",
                "writer",
                "Write a draft from this research:\n{output}",
            ),
            ("writer", "critic", "Critique this draft:\n{output}"),
            (
                "critic",
                "writer",
                "Revise the draft per this critique:\n{output}",
            ),
        ],
    },
];

/// Stages a graph the analyzer suggested.
pub fn apply_suggested(bus: &Bus, graph: &SuggestedGraph) {
    for node in &graph.nodes {
        bus::publish_command(
            bus,
            &UiCommand::StageNode {
                spec: NodeSpec {
                    id: node.id.clone(),
                    label: node.id.clone(),
                    system_prompt: node.role.clone(),
                    cwd: String::new(),
                    allowed_tools: Vec::new(),
                    model: None,
                },
            },
        );
    }
    for edge in &graph.edges {
        bus::publish_command(
            bus,
            &UiCommand::AddEdge {
                edge: Edge {
                    id: format!("{}->{}", edge.from, edge.to),
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                    trigger: Trigger::OnTurnEnd,
                    prompt_template: edge.template.clone(),
                },
            },
        );
    }
}

/// Stages every node and edge of a preset onto the graph.
pub fn apply(bus: &Bus, preset: &Preset) {
    for (id, role) in preset.nodes {
        bus::publish_command(
            bus,
            &UiCommand::StageNode {
                spec: NodeSpec {
                    id: id.to_string(),
                    label: id.to_string(),
                    system_prompt: role.to_string(),
                    cwd: String::new(),
                    allowed_tools: Vec::new(),
                    model: None,
                },
            },
        );
    }
    for (from, to, template) in preset.edges {
        bus::publish_command(
            bus,
            &UiCommand::AddEdge {
                edge: Edge {
                    id: format!("{from}->{to}"),
                    from: from.to_string(),
                    to: to.to_string(),
                    trigger: Trigger::OnTurnEnd,
                    prompt_template: template.to_string(),
                },
            },
        );
    }
}
