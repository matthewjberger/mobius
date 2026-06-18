# Mobius

Program prompt loops over Claude Code. Mobius runs a graph of Claude agents that
feed each other and loop until done, keeps the whole graph queryable and
drivable, and puts a conductor Claude in a chat window so you can build, steer,
and interrogate the graph in plain English.

It is all Rust: a tokio host that owns the agent subprocesses and a hearsay bus, a
Leptos UI in a `wry` webview, and a programmable core you can drive from a plain
Rust program. The host is the point, so Mobius ships native.

## Run

```sh
just run       # the desktop app: the graph, the inspectors, and the conductor chat
just run-web   # the page in a browser, observe-only (no host)
just example   # a headless two-node loop driven from Rust
```

`just run` needs the `claude` CLI on your `PATH`. The first build needs the pinned
toolchain: `just init` (or install Rust 1.95 with the `wasm32-unknown-unknown`
target, plus `trunk`, `wasm-bindgen`, and `wasm-opt`).

## Docs

- [docs/USAGE.md](docs/USAGE.md) — how to drive it: the conductor chat, staging
  and executing a graph by hand, the node inspector, and the headless Rust API.
- [docs/DESIGN.md](docs/DESIGN.md) — the architecture and the decisions: the
  crates, the bus and its topics, the graph model, the orchestrator, the
  conductor, and the MCP surface.

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at
your option.
