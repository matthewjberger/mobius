//! The top bar: identity, host connection, the workspace the graph runs in, an
//! inline form to stage a node, and the button to execute the staged graph.

use leptos::prelude::*;
use protocol::{NodeSpec, UiCommand};

use crate::bus::{self, Bus};
use crate::state::MobiusState;

#[component]
pub fn Toolbar(state: MobiusState, bus: Bus) -> impl IntoView {
    let new_id = RwSignal::new(String::new());
    let new_role = RwSignal::new(String::new());
    let workspace = RwSignal::new(String::new());

    Effect::new(move |_| {
        let current = state.snapshot.get().workspace;
        if !current.is_empty() && workspace.get_untracked().is_empty() {
            workspace.set(current);
        }
    });

    let stage = {
        let bus = bus.clone();
        move || {
            let id = new_id.get_untracked().trim().to_string();
            let role = new_role.get_untracked().trim().to_string();
            if id.is_empty() || role.is_empty() {
                return;
            }
            let spec = NodeSpec {
                label: id.clone(),
                id,
                system_prompt: role,
                cwd: String::new(),
                allowed_tools: Vec::new(),
                model: None,
            };
            bus::publish_command(&bus, &UiCommand::StageNode { spec });
            new_id.set(String::new());
            new_role.set(String::new());
        }
    };

    let set_workspace = {
        let bus = bus.clone();
        move || {
            let path = workspace.get_untracked().trim().to_string();
            if path.is_empty() {
                return;
            }
            bus::publish_command(&bus, &UiCommand::SetWorkspace { path });
        }
    };

    let browse = {
        let bus = bus.clone();
        move |_| bus::publish_command(&bus, &UiCommand::PickWorkspace)
    };

    let execute = {
        let bus = bus.clone();
        move |_| bus::publish_command(&bus, &UiCommand::Execute)
    };

    let kill = {
        let bus = bus.clone();
        move |_| bus::publish_command(&bus, &UiCommand::StopAll)
    };

    view! {
        <div class="toolbar">
            <div class="brand">
                <span class="logo">"\u{221E}"</span>
                <span class="brand-name">"Mobius"</span>
            </div>
            <div class=move || if state.connected.get() { "conn online" } else { "conn offline" }>
                <span class="conn-dot"></span>
                <span>
                    {move || if state.connected.get() { "host connected" } else { "connecting to host..." }}
                </span>
            </div>
            <div class="workspace-field">
                <span class="field-label">"workspace"</span>
                <input
                    class="ti workspace-input"
                    placeholder="e.g. C:\\Users\\you\\code\\nightshade"
                    prop:value=move || workspace.get()
                    on:input=move |event| workspace.set(event_target_value(&event))
                    on:keydown={
                        let set_workspace = set_workspace.clone();
                        move |event| {
                            if event.key() == "Enter" {
                                set_workspace();
                            }
                        }
                    }
                />
                <button class="btn" on:click=browse>
                    "Browse..."
                </button>
                <button
                    class="btn"
                    on:click={
                        let set_workspace = set_workspace.clone();
                        move |_| set_workspace()
                    }
                >
                    "Set"
                </button>
            </div>
            <div class="spacer"></div>
            <div class="new-node">
                <input
                    class="ti id"
                    placeholder="node id"
                    prop:value=move || new_id.get()
                    on:input=move |event| new_id.set(event_target_value(&event))
                />
                <input
                    class="ti role"
                    placeholder="role / system prompt"
                    prop:value=move || new_role.get()
                    on:input=move |event| new_role.set(event_target_value(&event))
                    on:keydown={
                        let stage = stage.clone();
                        move |event| {
                            if event.key() == "Enter" {
                                stage();
                            }
                        }
                    }
                />
                <button
                    class="btn"
                    on:click={
                        let stage = stage.clone();
                        move |_| stage()
                    }
                >
                    "Stage"
                </button>
            </div>
            <button class="btn primary execute" on:click=execute>
                {move || format!("Execute ({})", staged_count(state))}
            </button>
            <button class="btn kill" on:click=kill>"Stop all"</button>
        </div>
    }
}

fn staged_count(state: MobiusState) -> usize {
    state
        .snapshot
        .get()
        .nodes
        .iter()
        .filter(|view| matches!(view.status, protocol::NodeStatus::Staged))
        .count()
}
