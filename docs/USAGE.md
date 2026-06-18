# Using Mobius

Mobius runs a graph of Claude Code agents. A node is one `claude` agent with a
role; an edge routes one node's finished turn into another's input, forming a
loop. You build the graph as data, watch it run, and inspect or steer any node.

There are two ways to drive it, and they reach the same graph.

## 1. Talk to the conductor

The right-hand chat is a Claude that conducts the graph for you. Tell it what you
want in plain English and it stages nodes, wires edges, executes, and reports
back. It drives the graph through the same tools an external Claude Code instance
would; it has no other access.

A first session:

1. Set the **workspace** in the toolbar to the repo you want to work on, for
   example `C:\Users\you\code\nightshade`, and press Set. Every agent runs there,
   so they share that project's context.
2. In the chat, ask for what you want:
   > Spawn an implementer and a reviewer on this repo, wire them into a loop, and
   > have the implementer add a retry policy with backoff to the HTTP client. The
   > reviewer should push back until it is solid.
3. Watch the graph appear on the left and the agents start passing work back and
   forth. Ask the conductor things like "what is the reviewer waiting on?" or
   "pause the implementer" at any time.

## 2. Build the graph by hand

You can also stage and execute the graph yourself.

1. Set the **workspace**.
2. **Stage** a node: type an id and a role (its system prompt) in the toolbar and
   press Stage. It appears on the graph with a dashed outline, designed but not
   running.
3. **Connect** nodes: select a node, then in the side panel pick a target and a
   prompt template and press Connect. `{output}` in the template is replaced by
   the source node's output when the edge fires. Stage as many nodes and edges as
   you like; nothing runs yet.
4. **Execute**: press Execute in the toolbar to start every staged node.
5. **Kick it off**: select a node and send it a first prompt from the side panel.
   Its finished turns flow along the edges and the loop runs.

## The side panel

Click any node to open it. You see its role, status, turn count, working
directory, and its outbound connections. Below that is a terminal view of
everything that crossed the agent's pipes:

- `in` — a prompt sent to the agent (its stdin)
- `out` — assistant text, `···` thinking, `tool` a tool call (its stdout)
- `end` — the result that ends a turn
- `err` — a stderr line

The panel also has Pause, Resume, and Stop, and a box to send the node a one-off
prompt.

## Edges and triggers

An edge fires when its source finishes a turn. From the UI an edge fires on every
turn. Through the conductor (or the MCP `add_edge` tool) you can also fire only
when the output contains a substring or matches a regex, which is how you build
"loop until approved" style graphs.

## Headless, from Rust

The same host the app boots is a library. A program builds and runs a graph with
no UI:

```rust
use mobius::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let host = open().await?;
    set_workspace(&host, "C:/Users/you/code/nightshade")?;
    stage_node(&host, node_spec("impl", "Implement the task in this repo")).await?;
    stage_node(&host, node_spec("review", "Review the diff and push back hard")).await?;
    add_edge(&host, edge_on_turn_end("impl", "review", "Review this work:\n{output}"))?;
    add_edge(&host, edge_on_turn_end("review", "impl", "Address this review:\n{output}"))?;
    execute(&host)?;
    send_prompt(&host, "impl", "Add a retry policy to the HTTP client")?;
    run(&host).await
}
```

Run the bundled version with `just example`.

## Requirements

`just run` needs the `claude` CLI on your `PATH`; the agents and the conductor are
`claude` subprocesses. The graph runs entirely on your machine.
