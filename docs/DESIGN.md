# Mobius design

Mobius is an all-Rust tool for programming **prompt loops** over Claude Code:
graphs of Claude agents that feed each other, run continuously, and expose their
state for inspection and control. You drive the graph two ways: programmatically,
from Rust, by building a graph and running it; and conversationally, from a chat
window in the app where you talk to a conductor Claude in plain English and it
drives, queries, and interrogates the running graph for you.

The UI is Leptos, compiled to wasm and hosted in a `wry` webview. The graph, the
agents, the bus, and the conductor all live in the native host, because the host
is the point: it owns the `claude` subprocesses, and a browser tab cannot. The
web build exists for development; the product ships native. The whole stack is
Rust plus a few lines of wasm bootstrap JavaScript. No npm, no bundler, no
JavaScript framework.

This document is the architecture and the decisions. It is the source of truth as
the product grows.

## Principles

- **All Rust, no npm.** Every dependency is a Rust crate. The only JavaScript is
  the trunk-generated wasm bootstrap. The UI is Leptos, the host is tokio, the
  bus is [hearsay](https://github.com/matthewjberger/hearsay).
- **Data-oriented, not OOP.** State is plain structs; behavior is free functions.
  The page state is a `Copy` struct of signals, the host state is a `Graph` value
  threaded through free functions. Nothing is an object that owns the app, the
  bus, or the agents.
- **One wire format.** Every cross-context message is serde in `protocol`. The
  page and the host share the exact same types, routed by hearsay topic.
- **The bus is the backbone.** Agents, the orchestrator, the conductor, and the
  page are all peers on one hearsay broker. State flows as published snapshots and
  output streams, not bespoke sockets.
- **The library is the product, the app is a client.** The orchestration lives in
  the `mobius` crate as a programmable API. The desktop shell is one consumer of
  it; a headless Rust program is another.

## Crates

A four-crate workspace, mirroring nightshade's `template_api_leptos` layout.

- **`mobius` (`core/`):** the orchestration library, native. Owns the graph, the
  agent subprocesses, the hearsay broker, the conductor relay, and the MCP server.
  This is the programmable surface: `open()` a host, `spawn_node`, `add_edge`,
  `send_prompt`, `snapshot`. `examples/` drives it with no UI.
- **`protocol`:** the wire format shared by the host and the page. Graph and node
  types, the normalized output stream, the UI command set, and the topic names.
  Pure serde, so it compiles to both native and wasm.
- **`desktop`:** the `wry` shell. Serves the web bundle into a webview window and
  boots the `mobius` host. The binary you run and ship is `mobius`.
- **`mobius-ui` (`src/`):** the Leptos page. The graph view, the node inspector,
  and the conductor chat. Joins the bus as a browser websocket client and renders
  what flows over it.

## Contexts

Three isolated contexts, each doing what it is best at.

- **Host (`mobius` crate, in the desktop process):** a tokio runtime on a
  background thread. Hosts the hearsay broker, runs the orchestrator loop, owns
  every `claude` subprocess, runs the conductor, and serves the MCP endpoint.
- **Desktop shell (`desktop/`):** `winit` plus `wry`. Serves the wasm bundle on an
  ephemeral localhost port (embedded with `rust-embed` in release, read from
  `dist` in debug) and opens it in a webview. Starts the host on launch.
- **Page (`mobius-ui`):** the Leptos UI in the webview. No subprocesses, no disk.
  It is a hearsay websocket peer: it subscribes to the graph and output topics to
  render, and publishes commands and conductor prompts to drive.

## The bus

One hearsay broker, started by the host, is the seam between everything.

- **TCP** on `127.0.0.1:9610` for native peers (the orchestrator, and, later,
  out-of-process agents).
- **WebSocket** on `127.0.0.1:9611` for the page, via hearsay's `websockets`
  feature. The page speaks the same postcard wire protocol a TCP peer does; on
  `wasm32` hearsay compiles down to its protocol types (`PeerEvent`, `Message`)
  and the page frames them itself over a `web_sys::WebSocket`.

Topics (`protocol::topics`):

- `mobius/graph` - the host publishes a `GraphSnapshot` whenever the graph
  changes, and on request. Pub/sub keeps no history, so a page publishes
  `RequestSnapshot` on connect to prime its view.
- `mobius/nodes/output` - a `NodeOutput` per normalized stream event from every
  agent: assistant text, thinking, a tool call, a turn result, a stderr line.
- `mobius/nodes/state` - a `NodeStateUpdate` when an agent's status or turn count
  changes.
- `mobius/conductor/output` - the conductor's stream, normalized the same way, for
  the chat pane.
- `mobius/conductor/prompt` - the page publishes the user's plain-English message
  here; the conductor relay feeds it to the conductor subprocess.
- `mobius/command` - the page publishes a `UiCommand` here to drive the graph from
  the UI (spawn, stop, pause, resume, send a prompt, wire an edge).

## The graph model

A **node** is one Claude Code agent: a persistent `claude` subprocess in
`--input-format stream-json --output-format stream-json` mode, with a role
(`system_prompt`), a working directory, an allowed-tool set, and a model. A
**`NodeSpec`** is its declaration; a **`NodeView`** is its live state (status, turn
count, last output).

An **edge** is a routing rule: when its source node finishes a turn and a
**`Trigger`** matches (every turn, output contains a string, output matches a
regex), the orchestrator renders the edge's `prompt_template` (with the source's
output substituted for `{output}`) and writes it to the target node's stdin. Edges
are what make it a loop: wire a node's output back to its own input, or two nodes
to each other, and they run until a terminal condition.

The orchestrator owns the subprocesses directly in v1, mediating every edge
in-process, so the whole graph state is one queryable value. Out-of-process agents
supervised over hearsay's `spawn` feature are the documented scaling path, not the
starting point.

## The orchestrator

A single tokio task owns a `Graph` and a map of running agents. Everything that
mutates the graph funnels into one command channel, so there is one writer and the
state is never contended:

- The bus feeds it: `UiCommand`s from the page and prompts from the conductor.
- The MCP server feeds it: each tool call becomes a command, with a oneshot reply
  for the queries.
- Each agent's stdout reader feeds it: a parsed turn-ended signal so it can
  evaluate edges.

On every change it publishes a fresh `GraphSnapshot` to `mobius/graph`, and it
streams every agent's normalized output to `mobius/nodes/output` as it arrives. The
graph is authoritative in one place and observable from everywhere.

## The conductor

The conductor is a Claude Code subprocess the host launches and points at its own
MCP endpoint, with a system prompt that tells it it conducts a graph of Claude
agents and drives it only through the mobius tools. Its stdin comes from
`mobius/conductor/prompt` (what you type in the chat); its stdout, the stream-json,
is normalized and republished to `mobius/conductor/output` for the chat to render.

This is the headline interface: a chat window that just uses Claude. You ask, in
English, "spawn a reviewer on the auth crate and have it argue with the
implementer until they agree," or "why is the test node stuck?", and the conductor
calls `spawn_node`, `add_edge`, `get_transcript`, `get_graph` to make it happen and
answer you. The same MCP endpoint is open to any other Claude Code instance, so the
graph is drivable from outside the app too.

## The MCP surface

The host serves Model Context Protocol over local HTTP (`127.0.0.1:9612/mcp`),
turning each `tools/call` into an orchestrator command and the reply into tool
output. It holds no state of its own; the graph lives in the orchestrator.

- **Query:** `get_graph`, `list_nodes`, `get_node`, `get_transcript`.
- **Drive:** `spawn_node`, `stop_node`, `pause_node`, `resume_node`, `send_prompt`,
  `add_edge`, `remove_edge`.

It mirrors the nightshade editor's and neon's MCP bridge: a thin JSON-RPC front end
over a correlation-id request channel.

## The page

The page is small. A `MobiusState` of `Copy` signals holds the latest snapshot, the
per-node output buffers, the conductor transcript, and the selection. `bus.rs` is
the websocket peer: it connects, subscribes, and maps each inbound `Message` to a
signal write; outbound, it publishes `UiCommand`s and `ConductorPrompt`s. Three
components render it:

- **Graph view** - the nodes as a live graph, colored by status, with the edges
  between them; selecting a node opens the inspector.
- **Node inspector** - the selected node's spec, status, turn count, and streaming
  transcript, with controls to pause, resume, stop, and send a one-off prompt.
- **Conductor chat** - the chat window. You type English; it renders the
  conductor's assistant text, thinking, and the tool calls it makes against the
  graph.

## The workspace and repo analysis

Every agent runs in a **workspace** directory, so the whole graph shares one
project's context. The page can't open a native dialog from the webview, so it
follows the same rule as everything else: it asks the host. `UiCommand::PickWorkspace`
makes the host open an `rfd` folder dialog on a blocking task and feed the choice
back as a `SetWorkspace`. `set_workspace` resolves a node's empty or `.` working
directory at stage time, so staging after picking lands agents in the repo.

**Analyze** is recon as a one-shot agent. `UiCommand::Analyze { goal }` spawns a
transient Claude analyst (`analyzer.rs`) in the workspace with read-only tools,
prompted to inspect the repo and return a JSON array of candidate workflows: named
graphs of node roles and edges, each with a rationale. The host parses that
(tolerating prose around the JSON) and publishes an `AnalyzeResult` to
`mobius/suggestions`. The page renders each suggestion as a one-click card that
stages the whole graph, exactly like a built-in preset but tailored to the repo
and goal. The conductor can trigger the same flow through the `analyze` MCP tool.

## Programmatic use

The same host the desktop boots is a library. A headless program builds and runs a
graph with no webview:

```rust
use mobius::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let host = open().await?;
    spawn_node(&host, node_spec("impl", "Implement the task in ./crate")).await?;
    spawn_node(&host, node_spec("review", "Review the diff and push back hard")).await?;
    add_edge(&host, edge_on_turn_end("impl", "review", "Review this work:\n{output}"))?;
    add_edge(&host, edge_on_turn_end("review", "impl", "Address this review:\n{output}"))?;
    send_prompt(&host, "impl", "Add a retry policy to the client")?;
    run(&host).await
}
```

The desktop shell is exactly this plus a webview and the bus already wired to a
page.

## Milestones

1. **Host core.** The bus, the orchestrator loop, the agent subprocess runner with
   stream-json normalization, the graph snapshot, and the programmable API with a
   headless example. Drive a two-node loop from Rust.
2. **Desktop and page.** The `wry` shell, the page bus client, the graph view, the
   node inspector. Watch a graph run live.
3. **Conductor and MCP.** The MCP server with the query and drive tools, the
   conductor subprocess, and the chat pane. Drive the graph in English.
4. **Authoring and persistence.** Save and load graphs, node-spec templates, edge
   triggers richer than substring and regex, and a graph editor in the UI.
5. **Out-of-process agents.** Supervise agents as separate processes over hearsay's
   `spawn` feature, so a graph spans machines.

## Build

```sh
just run       # native webview over the bundle
just run-web   # serve the page in the browser (no host; observe-only)
```

`just dist` produces the release web bundle; `just build-desktop` embeds it into
the `mobius` executable. Path dependencies are self-contained in the workspace.
