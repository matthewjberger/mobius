//! The top bar: identity, host connection, the active workspace and what was
//! detected there, and the run controls. Building the graph happens on the canvas
//! and in the conductor, so the bar stays uncluttered.

use leptos::prelude::*;
use protocol::UiCommand;

use crate::bus::{self, Bus};
use crate::state::MobiusState;

#[component]
pub fn Toolbar(state: MobiusState, bus: Bus) -> impl IntoView {
    let open = {
        let bus = bus.clone();
        move |_| bus::publish_command(&bus, &UiCommand::PickWorkspace)
    };

    let execute = {
        let bus = bus.clone();
        move |_| {
            bus::publish_command(&bus, &UiCommand::Execute);
            let kickoff = state.kickoff.get_untracked();
            if !kickoff.trim().is_empty() {
                bus::publish_command(&bus, &UiCommand::Kickoff { text: kickoff });
            }
        }
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
                    {move || if state.connected.get() { "connected" } else { "connecting..." }}
                </span>
            </div>
            <button class="btn open-btn" on:click=open>
                "Open repo..."
            </button>
            <span class="workspace-active" title="the directory new agents run in">
                {move || {
                    let path = state.snapshot.get().workspace;
                    if path.is_empty() || path == "." {
                        "no repo chosen".to_string()
                    } else {
                        path
                    }
                }}
            </span>
            {move || {
                let kind = state.snapshot.get().workspace_kind;
                (!kind.is_empty()).then(|| view! { <span class="repo-badge">{kind}</span> })
            }}
            <div class="spacer"></div>
            <button class="btn primary execute" on:click=execute>
                {move || {
                    let staged = staged_count(state);
                    if staged > 0 {
                        format!("Execute ({staged})")
                    } else {
                        "Execute".to_string()
                    }
                }}
            </button>
            <button class="btn kill" on:click=kill>
                "Stop all"
            </button>
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
