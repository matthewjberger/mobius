//! A two-node loop driven from Rust with no UI: an implementer and a reviewer
//! that pass work back and forth. Run with `cargo run --example review_loop`,
//! then watch the bus or attach the desktop app. Ctrl-C to stop.

use mobius::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let host = open().await?;

    spawn_node(
        &host,
        node_spec(
            "implementer",
            "You implement the requested change in the working directory in small, focused steps.",
        ),
    )
    .await?;
    spawn_node(
        &host,
        node_spec(
            "reviewer",
            "You review the implementer's latest work and push back hard on anything weak. If it is solid, begin your reply with APPROVED.",
        ),
    )
    .await?;

    add_edge(
        &host,
        edge_on_turn_end("implementer", "reviewer", "Review this work:\n{output}"),
    )?;
    add_edge(
        &host,
        edge_on_turn_end("reviewer", "implementer", "Address this review:\n{output}"),
    )?;

    send_prompt(
        &host,
        "implementer",
        "Add a retry policy with exponential backoff to the HTTP client.",
    )?;

    run(&host).await
}
