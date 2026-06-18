//! The headless Mobius host. Boots the bus, the orchestrator, the conductor, and
//! the MCP endpoint, then parks. Run it locally and point a web UI at it: the page
//! served by `just run-web` or the GitHub Pages site connects to the websocket bus
//! on localhost, which works even from an https origin because localhost is a
//! secure context.

#[tokio::main]
async fn main() -> mobius::Result<()> {
    let host = mobius::open().await?;
    eprintln!("mobius host running. the web UI connects over the bus:");
    eprintln!("  bus  (ws):   ws://127.0.0.1:9611   <- the page connects here");
    eprintln!("  bus  (tcp):  127.0.0.1:9610        native peers");
    eprintln!("  mcp  (http): http://127.0.0.1:9612/mcp");
    eprintln!("open the UI with `just run-web`, or the GitHub Pages site. Ctrl-C to stop.");
    mobius::run(&host).await
}
