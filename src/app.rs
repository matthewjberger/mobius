//! The application root: owns the shared state, opens the bus, and composes the
//! toolbar, the graph workspace, and the conductor chat.

use leptos::prelude::*;
use protocol::OutputKind;

use crate::bus;
use crate::components::chat::Chat;
use crate::components::graph_view::GraphView;
use crate::components::inspector::Inspector;
use crate::components::toolbar::Toolbar;
use crate::state::{ChatEntry, MobiusState};

const WELCOME: &str = "I conduct a graph of Claude Code agents. Set your workspace up top (the repo you want to work on), then tell me what you want, for example: \"set up an implementer and a reviewer on this repo and loop them on adding a retry policy to the HTTP client.\" I will stage the graph and show it to you on the left; nothing runs until you say go. You can also stage a template from the canvas or build nodes by hand, then press Execute. Stop all is the kill switch. Ask me what any node is doing at any time.";

#[component]
pub fn App() -> impl IntoView {
    let state = MobiusState::new();
    let bus = bus::connect(state);

    state.chat.update(|entries| {
        entries.push(ChatEntry {
            mine: false,
            kind: OutputKind::Info,
            text: WELCOME.to_string(),
        })
    });

    view! {
        <div class="app-shell">
            <Toolbar state=state bus=bus.clone() />
            <div class="workspace">
                <GraphView state=state bus=bus.clone() />
                <Inspector state=state bus=bus.clone() />
            </div>
            <Chat state=state bus=bus.clone() />
        </div>
    }
}
