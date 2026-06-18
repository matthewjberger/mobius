//! The conductor chat: the headline interface. You type plain English; the
//! conductor Claude drives and interrogates the graph through the mobius tools and
//! answers here. Its stream is normalized the same way an agent's is, so you see
//! its prose, its thinking, and the tool calls it makes against the graph.

use leptos::html::Div;
use leptos::prelude::*;
use protocol::OutputKind;

use crate::bus::{self, Bus};
use crate::state::{ChatEntry, MobiusState, kind_class};

#[component]
pub fn Chat(state: MobiusState, bus: Bus) -> impl IntoView {
    let input = RwSignal::new(String::new());
    let log_ref = NodeRef::<Div>::new();

    Effect::new(move |_| {
        state.chat.track();
        state.chat_busy.track();
        if let Some(element) = log_ref.get() {
            element.set_scroll_top(element.scroll_height());
        }
    });

    let send = {
        let bus = bus.clone();
        move || {
            let text = input.get_untracked().trim().to_string();
            if text.is_empty() {
                return;
            }
            bus::send_conductor(&bus, &text);
            state.chat.update(|entries| {
                entries.push(ChatEntry {
                    mine: true,
                    kind: OutputKind::Prompt,
                    text: text.clone(),
                })
            });
            state.chat_busy.set(true);
            input.set(String::new());
        }
    };

    view! {
        <div class="chat">
            <div class="chat-head">
                <span class="chat-title">"Conductor"</span>
                <span class="chat-sub">"drive the graph in plain English"</span>
            </div>
            <div class="chat-log" node_ref=log_ref>
                {move || {
                    state
                        .chat
                        .get()
                        .into_iter()
                        .map(|entry| {
                            let class = if entry.mine {
                                "msg mine".to_string()
                            } else {
                                format!("msg {}", kind_class(entry.kind))
                            };
                            view! { <div class=class>{entry.text}</div> }
                        })
                        .collect_view()
                }}
                <Show when=move || state.chat_busy.get() fallback=|| ()>
                    <div class="msg working">"working..."</div>
                </Show>
            </div>
            <div class="chat-compose">
                <textarea
                    class="chat-input"
                    placeholder=move || {
                        if state.connected.get() {
                            "Ask the conductor to spawn agents, wire a loop, or explain what a node is doing"
                        } else {
                            "connecting to host..."
                        }
                    }
                    prop:value=move || input.get()
                    on:input=move |event| input.set(event_target_value(&event))
                    on:keydown={
                        let send = send.clone();
                        move |event| {
                            if event.key() == "Enter" && !event.shift_key() {
                                event.prevent_default();
                                send();
                            }
                        }
                    }
                ></textarea>
                <button
                    class="btn primary chat-send"
                    on:click={
                        let send = send.clone();
                        move |_| send()
                    }
                >
                    "Send"
                </button>
            </div>
        </div>
    }
}
