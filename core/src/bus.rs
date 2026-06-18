//! The host's side of the hearsay bus: connecting publishers and subscribers,
//! publishing the graph's snapshots and output streams, and forwarding the page's
//! commands into the orchestrator's channel.

use hearsay::{Client, ClientSettings, Route};
use protocol::{
    Communication, ConductorEvent, GraphSnapshot, NodeOutput, NodeStateUpdate, OutputKind,
    UiCommand, topics,
};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::Result;
use crate::orchestrator::Command;

/// Connects a client to the broker with the default settings.
pub async fn connect(name: &str, address: &str) -> Result<Client> {
    let mut client = hearsay::create_client(name, ClientSettings::default());
    hearsay::connect(&mut client, address).await?;
    Ok(client)
}

/// Connects a client and subscribes it to the given topics.
pub async fn connect_subscribed(name: &str, address: &str, subjects: &[&str]) -> Result<Client> {
    let mut client = connect(name, address).await?;
    hearsay::subscribe(&mut client, subjects).await?;
    Ok(client)
}

/// Drains the `mobius/command` topic and forwards each [`UiCommand`] into the
/// orchestrator as a [`Command`].
pub async fn forward_commands(mut client: Client, commands: mpsc::UnboundedSender<Command>) {
    while let Some(message) = hearsay::next_message(&mut client).await {
        let Ok(ui) = serde_json::from_str::<UiCommand>(&message.payload) else {
            continue;
        };
        if commands.send(Command::from(ui)).is_err() {
            break;
        }
    }
}

/// Publishes a serializable payload to a topic, swallowing transport errors so a
/// dropped page never stalls the orchestrator.
pub async fn publish<T: Serialize>(client: &Client, topic: &str, payload: &T) {
    let _ = hearsay::publish(client, topic, payload, Route::Global).await;
}

/// Publishes the whole graph to `mobius/graph`.
pub async fn publish_snapshot(client: &Client, snapshot: &GraphSnapshot) {
    publish(client, topics::GRAPH, snapshot).await;
}

/// Publishes one normalized agent output line to `mobius/nodes/output`.
pub async fn publish_output(client: &Client, node: &str, kind: OutputKind, text: &str) {
    let output = NodeOutput {
        node: node.to_string(),
        kind,
        text: text.to_string(),
    };
    publish(client, topics::NODE_OUTPUT, &output).await;
}

/// Publishes an agent's status change to `mobius/nodes/state`.
pub async fn publish_state(client: &Client, update: &NodeStateUpdate) {
    publish(client, topics::NODE_STATE, update).await;
}

/// Publishes one conductor stream line to `mobius/conductor/output`.
pub async fn publish_conductor(client: &Client, event: &ConductorEvent) {
    publish(client, topics::CONDUCTOR_OUTPUT, event).await;
}

/// Publishes one edge message to `mobius/comms`.
pub async fn publish_comm(client: &Client, comm: &Communication) {
    publish(client, topics::COMMS, comm).await;
}
