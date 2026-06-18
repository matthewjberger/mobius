//! Builders for the graph's plain data. Keep `protocol` free of behavior; the
//! ergonomic constructors live here.

use protocol::{Edge, NodeSpec, Trigger};

/// A node spec with the given id and system prompt, the id as its label, the
/// current directory as its working directory, and full tool access.
pub fn node_spec(id: &str, system_prompt: &str) -> NodeSpec {
    NodeSpec {
        id: id.to_string(),
        label: id.to_string(),
        system_prompt: system_prompt.to_string(),
        cwd: ".".to_string(),
        allowed_tools: Vec::new(),
        model: None,
    }
}

/// An edge that fires on every finished turn of `from`.
pub fn edge_on_turn_end(from: &str, to: &str, prompt_template: &str) -> Edge {
    edge(from, to, Trigger::OnTurnEnd, prompt_template)
}

/// An edge that fires when `from`'s output contains `needle`.
pub fn edge_on_contains(from: &str, to: &str, needle: &str, prompt_template: &str) -> Edge {
    edge(
        from,
        to,
        Trigger::OnContains {
            needle: needle.to_string(),
        },
        prompt_template,
    )
}

/// An edge that fires when `from`'s output matches the regex `pattern`.
pub fn edge_on_match(from: &str, to: &str, pattern: &str, prompt_template: &str) -> Edge {
    edge(
        from,
        to,
        Trigger::OnMatch {
            pattern: pattern.to_string(),
        },
        prompt_template,
    )
}

fn edge(from: &str, to: &str, trigger: Trigger, prompt_template: &str) -> Edge {
    Edge {
        id: format!("{from}->{to}"),
        from: from.to_string(),
        to: to.to_string(),
        trigger,
        prompt_template: prompt_template.to_string(),
    }
}
