//! The page's side of the hearsay bus: a `web_sys::WebSocket` peer that speaks
//! the same postcard wire protocol a native client does. It subscribes to the
//! graph and output topics and maps each inbound message to a signal write;
//! outbound, it publishes the page's commands and conductor prompts. Data only;
//! behavior is the free functions below.

use std::cell::RefCell;
use std::rc::Rc;

use hearsay::{Message, PeerEvent};
use leptos::prelude::*;
use protocol::{
    AnalyzeResult, Communication, ConductorEvent, ConductorPrompt, GraphSnapshot, NodeOutput,
    NodeStateUpdate, OutputKind, UiCommand, topics,
};
use send_wrapper::SendWrapper;
use serde::Serialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{BinaryType, MessageEvent, WebSocket};

use crate::state::{ChatEntry, MobiusState};

const WS_URL: &str = "ws://127.0.0.1:9611";
const RECONNECT_MS: i32 = 1000;
const PAGE_ID: &str = "mobius-page";
const OUTPUT_LIMIT: usize = 800;

/// The page's bus handle. Cloneable, so every component can publish through it.
/// The `WebSocket` is `!Send`; the wrapper makes the handle satisfy leptos's
/// `Send` bound, sound because the page is single-threaded wasm.
#[derive(Clone)]
pub struct Bus {
    socket: SendWrapper<Rc<RefCell<Option<WebSocket>>>>,
}

/// Opens the bus and keeps it open, reconnecting on close.
pub fn connect(state: MobiusState) -> Bus {
    let bus = Bus {
        socket: SendWrapper::new(Rc::new(RefCell::new(None))),
    };
    open(&bus, state);
    bus
}

/// Publishes a [`UiCommand`] to drive the graph.
pub fn publish_command(bus: &Bus, command: &UiCommand) {
    if let Some(socket) = bus.socket.borrow().as_ref()
        && socket.ready_state() == WebSocket::OPEN
    {
        publish(socket, command, topics::COMMAND);
    }
}

/// Publishes a plain-English message to the conductor.
pub fn send_conductor(bus: &Bus, text: &str) {
    if let Some(socket) = bus.socket.borrow().as_ref()
        && socket.ready_state() == WebSocket::OPEN
    {
        publish(
            socket,
            &ConductorPrompt {
                text: text.to_string(),
            },
            topics::CONDUCTOR_PROMPT,
        );
    }
}

fn open(bus: &Bus, state: MobiusState) {
    let Ok(socket) = WebSocket::new(WS_URL) else {
        schedule_reconnect(bus.clone(), state);
        return;
    };
    socket.set_binary_type(BinaryType::Arraybuffer);

    let open_socket = socket.clone();
    let onopen = Closure::<dyn FnMut()>::new(move || {
        state.connected.set(true);
        hello_and_subscribe(&open_socket);
        publish(&open_socket, &UiCommand::RequestSnapshot, topics::COMMAND);
    });
    socket.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() else {
            return;
        };
        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
        if let Ok(message) = postcard::from_bytes::<Message>(&bytes) {
            route(&message, state);
        }
    });
    socket.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    let close_bus = bus.clone();
    let onclose = Closure::<dyn FnMut()>::new(move || {
        state.connected.set(false);
        schedule_reconnect(close_bus.clone(), state);
    });
    socket.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();

    *bus.socket.borrow_mut() = Some(socket);
}

fn route(message: &Message, state: MobiusState) {
    if message.topic == topics::GRAPH
        && let Ok(snapshot) = serde_json::from_str::<GraphSnapshot>(&message.payload)
    {
        state.snapshot.set(snapshot);
    } else if message.topic == topics::NODE_OUTPUT
        && let Ok(output) = serde_json::from_str::<NodeOutput>(&message.payload)
    {
        state.outputs.update(|map| {
            let lines = map.entry(output.node.clone()).or_default();
            lines.push(output);
            if lines.len() > OUTPUT_LIMIT {
                let excess = lines.len() - OUTPUT_LIMIT;
                lines.drain(0..excess);
            }
        });
    } else if message.topic == topics::NODE_STATE
        && let Ok(update) = serde_json::from_str::<NodeStateUpdate>(&message.payload)
    {
        state.snapshot.update(|snapshot| {
            if let Some(view) = snapshot
                .nodes
                .iter_mut()
                .find(|view| view.spec.id == update.node)
            {
                view.status = update.status;
                view.turns = update.turns;
            }
        });
    } else if message.topic == topics::COMMS
        && let Ok(comm) = serde_json::from_str::<Communication>(&message.payload)
    {
        state.pulse.set(Some((comm.from.clone(), comm.to.clone())));
        state.comms.update(|comms| {
            comms.push(comm);
            if comms.len() > 100 {
                let excess = comms.len() - 100;
                comms.drain(0..excess);
            }
        });
    } else if message.topic == topics::SUGGESTIONS
        && let Ok(result) = serde_json::from_str::<AnalyzeResult>(&message.payload)
    {
        state.analyzing.set(false);
        state.analyze_error.set(result.error);
        state.suggestions.set(result.graphs);
    } else if message.topic == topics::CONDUCTOR_OUTPUT
        && let Ok(event) = serde_json::from_str::<ConductorEvent>(&message.payload)
    {
        if matches!(event.kind, OutputKind::Result) {
            state.chat_busy.set(false);
        }
        let empty_result = matches!(event.kind, OutputKind::Result) && event.text.trim().is_empty();
        if !empty_result {
            state.chat.update(|entries| {
                entries.push(ChatEntry {
                    mine: false,
                    kind: event.kind,
                    text: event.text,
                })
            });
        }
    }
}

fn hello_and_subscribe(socket: &WebSocket) {
    send_event(
        socket,
        &PeerEvent::Hello {
            id: PAGE_ID.to_string(),
        },
    );
    for topic in [
        topics::GRAPH,
        topics::NODE_OUTPUT,
        topics::NODE_STATE,
        topics::CONDUCTOR_OUTPUT,
        topics::SUGGESTIONS,
        topics::COMMS,
    ] {
        send_event(
            socket,
            &PeerEvent::Subscribe {
                id: PAGE_ID.to_string(),
                topic: topic.to_string(),
            },
        );
    }
}

fn publish<T: Serialize>(socket: &WebSocket, payload: &T, topic: &str) {
    let Ok(json) = serde_json::to_string(payload) else {
        return;
    };
    send_event(
        socket,
        &PeerEvent::PublishText {
            id: PAGE_ID.to_string(),
            topic: topic.to_string(),
            payload: json,
            local_only: false,
        },
    );
}

fn send_event(socket: &WebSocket, event: &PeerEvent) {
    if let Ok(bytes) = postcard::to_allocvec(event) {
        let _ = socket.send_with_u8_array(&bytes);
    }
}

fn schedule_reconnect(bus: Bus, state: MobiusState) {
    *bus.socket.borrow_mut() = None;
    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = Closure::<dyn FnMut()>::new(move || open(&bus, state));
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        callback.as_ref().unchecked_ref(),
        RECONNECT_MS,
    );
    callback.forget();
}
