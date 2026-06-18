//! Mobius's page: the Leptos UI in the webview.
//!
//! Data-oriented throughout. State is a `Copy` struct of signals
//! ([`state::MobiusState`]); behavior is free functions; components are plain
//! `#[component]` functions. Nothing owns the bus or the graph.
//!
//! - `app.rs` composes the shell.
//! - `bus.rs` is the hearsay websocket peer and maps inbound messages to signals.
//! - `state.rs` is all page state as `Copy` signals, plus the render helpers.
//! - `components/` holds the toolbar, the graph view, the node inspector, and the
//!   conductor chat.

mod app;
mod bus;
mod components;
mod presets;
mod state;

pub use app::App;
