//! The node inspector: the selected agent's role, status, controls, and
//! connections, plus a terminal-style stream of everything that crossed its
//! pipes. Prompts in are its stdin; assistant, thinking, tool, and result lines
//! are its stdout; stderr is its stderr. Each line is tagged and color-coded, and
//! the view autoscrolls like a terminal.

use leptos::html::Div;
use leptos::prelude::*;
use protocol::{Edge, NodeView, OutputKind, Trigger, UiCommand};

use crate::bus::{self, Bus};
use crate::state::{MobiusState, kind_class, status_class};

#[component]
pub fn Inspector(state: MobiusState, bus: Bus) -> impl IntoView {
    let log_ref = NodeRef::<Div>::new();

    let selected_lines = Memo::new(move |_| {
        let id = state.selected.get();
        state.outputs.with(|map| {
            id.as_ref()
                .and_then(|node| map.get(node))
                .map(Vec::len)
                .unwrap_or(0)
        })
    });

    Effect::new(move |_| {
        state.selected.track();
        selected_lines.track();
        if let Some(element) = log_ref.get() {
            element.set_scroll_top(element.scroll_height());
        }
    });

    view! {
        <div class="inspector">
            {move || {
                let Some(id) = state.selected.get() else {
                    return view! {
                        <div class="inspector-empty">
                            "Select a node to watch its terminal, drive it, and wire it to others."
                        </div>
                    }
                    .into_any();
                };
                let Some(node) = state.snapshot.get().nodes.into_iter().find(|view| view.spec.id == id)
                else {
                    return view! { <div class="inspector-empty">"That node is gone."</div> }.into_any();
                };
                inspector_body(state, bus.clone(), node, log_ref).into_any()
            }}
        </div>
    }
}

fn inspector_body(
    state: MobiusState,
    bus: Bus,
    node: NodeView,
    log_ref: NodeRef<Div>,
) -> impl IntoView {
    let id = node.spec.id.clone();
    let prompt = RwSignal::new(String::new());
    let target = RwSignal::new(String::new());
    let template = RwSignal::new("{output}".to_string());

    let pause = {
        let bus = bus.clone();
        let id = id.clone();
        move |_| bus::publish_command(&bus, &UiCommand::PauseNode { node: id.clone() })
    };
    let resume = {
        let bus = bus.clone();
        let id = id.clone();
        move |_| bus::publish_command(&bus, &UiCommand::ResumeNode { node: id.clone() })
    };
    let stop = {
        let bus = bus.clone();
        let id = id.clone();
        move |_| bus::publish_command(&bus, &UiCommand::StopNode { node: id.clone() })
    };
    let send = {
        let bus = bus.clone();
        let id = id.clone();
        move || {
            let text = prompt.get_untracked().trim().to_string();
            if text.is_empty() {
                return;
            }
            bus::publish_command(
                &bus,
                &UiCommand::SendPrompt {
                    node: id.clone(),
                    text,
                },
            );
            prompt.set(String::new());
        }
    };
    let connect = {
        let bus = bus.clone();
        let from = id.clone();
        move || {
            let to = target.get_untracked();
            if to.is_empty() {
                return;
            }
            let chosen = template.get_untracked();
            let prompt_template = if chosen.trim().is_empty() {
                "{output}".to_string()
            } else {
                chosen
            };
            bus::publish_command(
                &bus,
                &UiCommand::AddEdge {
                    edge: Edge {
                        id: format!("{from}->{to}"),
                        from: from.clone(),
                        to,
                        trigger: Trigger::OnTurnEnd,
                        prompt_template,
                    },
                },
            );
        }
    };

    let snapshot = state.snapshot.get();
    let outgoing = snapshot
        .edges
        .iter()
        .filter(|edge| edge.from == id)
        .map(|edge| {
            let bus = bus.clone();
            let edge_id = edge.id.clone();
            let to = edge.to.clone();
            view! {
                <span class="edge-chip">
                    <span>{format!("\u{2192} {to}")}</span>
                    <button
                        class="chip-x"
                        on:click=move |_| {
                            bus::publish_command(&bus, &UiCommand::RemoveEdge { edge: edge_id.clone() })
                        }
                    >
                        "\u{00d7}"
                    </button>
                </span>
            }
        })
        .collect_view();
    let options = snapshot
        .nodes
        .iter()
        .filter(|view| view.spec.id != id)
        .map(|view| {
            let value = view.spec.id.clone();
            let text = value.clone();
            view! { <option value=value>{text}</option> }
        })
        .collect_view();

    let transcript_id = id.clone();
    let lines = move || {
        state
            .outputs
            .get()
            .get(&transcript_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|line| {
                view! {
                    <div class=format!("line {}", kind_class(line.kind))>
                        <span class="gutter">{gutter(line.kind)}</span>
                        <span class="line-text">{line.text}</span>
                    </div>
                }
            })
            .collect_view()
    };

    let status = status_class(node.status);
    let status_label = status_class(node.status);
    let label = node.spec.label.clone();
    let cwd = node.spec.cwd.clone();
    let role = node.spec.system_prompt.clone();
    let turns = node.turns;

    view! {
        <div class="inspector-head">
            <div class="inspector-title">
                <span class="node-label">{label}</span>
                <span class=format!("pill {status}")>{status_label}</span>
                <span class="muted">{format!("{turns} turns")}</span>
            </div>
            <div class="inspector-controls">
                <button class="btn" on:click=pause>"Pause"</button>
                <button class="btn" on:click=resume>"Resume"</button>
                <button class="btn danger" on:click=stop>"Stop"</button>
            </div>
        </div>
        <div class="inspector-role">{role}</div>
        <div class="inspector-cwd">{format!("cwd: {cwd}")}</div>
        <div class="connections">
            <div class="conn-list">{outgoing}</div>
            <div class="conn-builder">
                <select
                    class="ti select"
                    on:change=move |event| target.set(event_target_value(&event))
                >
                    <option value="">"connect to..."</option>
                    {options}
                </select>
                <input
                    class="ti template"
                    prop:value=move || template.get()
                    on:input=move |event| template.set(event_target_value(&event))
                />
                <button
                    class="btn"
                    on:click={
                        let connect = connect.clone();
                        move |_| connect()
                    }
                >
                    "Connect"
                </button>
            </div>
        </div>
        <div class="terminal" node_ref=log_ref>
            {lines}
        </div>
        <div class="inspector-compose">
            <input
                class="ti"
                placeholder="send a prompt to this node"
                prop:value=move || prompt.get()
                on:input=move |event| prompt.set(event_target_value(&event))
                on:keydown={
                    let send = send.clone();
                    move |event| {
                        if event.key() == "Enter" {
                            send();
                        }
                    }
                }
            />
            <button
                class="btn primary"
                on:click={
                    let send = send.clone();
                    move |_| send()
                }
            >
                "Send"
            </button>
        </div>
    }
}

fn gutter(kind: OutputKind) -> &'static str {
    match kind {
        OutputKind::Prompt => "in",
        OutputKind::Assistant => "out",
        OutputKind::Thinking => "···",
        OutputKind::Tool => "tool",
        OutputKind::Result => "end",
        OutputKind::Stderr => "err",
        OutputKind::Info => "·",
    }
}
