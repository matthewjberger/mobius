//! The MCP endpoint: Model Context Protocol over local HTTP, in front of the
//! orchestrator's command channel. Each `tools/call` becomes a [`Command`] and
//! its reply becomes the tool output. Holds no state of its own; the graph lives
//! in the orchestrator. The conductor and any external Claude Code instance drive
//! the graph through this surface.

use protocol::{Edge, NodeSpec, Trigger};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};

use crate::orchestrator::Command;

/// Starts the MCP server on its own thread and tokio runtime. Idempotent per
/// address: a second bind simply fails and logs.
pub fn start(commands: mpsc::UnboundedSender<Command>, address: String) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("[mcp] failed to start the runtime: {error}");
                return;
            }
        };
        serve(commands, &address, runtime.handle().clone());
    });
}

fn serve(commands: mpsc::UnboundedSender<Command>, address: &str, handle: Handle) {
    let server = match tiny_http::Server::http(address) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("[mcp] failed to bind {address}: {error}");
            return;
        }
    };
    eprintln!("[mcp] listening on http://{address}/mcp");
    for request in server.incoming_requests() {
        let commands = commands.clone();
        let handle = handle.clone();
        std::thread::spawn(move || handle_request(commands, handle, request));
    }
}

fn handle_request(
    commands: mpsc::UnboundedSender<Command>,
    handle: Handle,
    mut request: tiny_http::Request,
) {
    if *request.method() != tiny_http::Method::Post {
        let _ = request.respond(tiny_http::Response::empty(405));
        return;
    }
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        let _ = request.respond(tiny_http::Response::empty(400));
        return;
    }
    let Ok(message) = serde_json::from_str::<Value>(&body) else {
        let _ = request.respond(tiny_http::Response::empty(400));
        return;
    };
    let id = message.get("id").cloned();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    let response = handle.block_on(dispatch(&commands, &method, params, id));
    match response {
        Some(value) => {
            let header =
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .expect("static header is valid");
            let _ = request
                .respond(tiny_http::Response::from_string(value.to_string()).with_header(header));
        }
        None => {
            let _ = request.respond(tiny_http::Response::empty(202));
        }
    }
}

async fn dispatch(
    commands: &mpsc::UnboundedSender<Command>,
    method: &str,
    params: Value,
    id: Option<Value>,
) -> Option<Value> {
    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-03-26")
                .to_string();
            Some(rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "mobius", "version": "0.1.0" }
                }),
            ))
        }
        "notifications/initialized" => None,
        "ping" => Some(rpc_result(id, json!({}))),
        "tools/list" => Some(rpc_result(id, json!({ "tools": tool_definitions() }))),
        "tools/call" => Some(tool_call(commands, params, id).await),
        other => Some(rpc_error(id, -32601, &format!("method not found: {other}"))),
    }
}

async fn tool_call(
    commands: &mpsc::UnboundedSender<Command>,
    params: Value,
    id: Option<Value>,
) -> Value {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    match run_tool(commands, &name, arguments).await {
        Ok(text) => rpc_result(
            id,
            json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
        ),
        Err(error) => rpc_result(
            id,
            json!({ "content": [{ "type": "text", "text": error }], "isError": true }),
        ),
    }
}

async fn run_tool(
    commands: &mpsc::UnboundedSender<Command>,
    name: &str,
    arguments: Value,
) -> Result<String, String> {
    match name {
        "get_graph" => {
            let snapshot = ask(commands, |reply| Command::Snapshot { reply }).await?;
            Ok(serde_json::to_string(&snapshot).unwrap_or_default())
        }
        "list_nodes" => {
            let snapshot = ask(commands, |reply| Command::Snapshot { reply }).await?;
            let nodes: Vec<Value> = snapshot
                .nodes
                .iter()
                .map(|view| {
                    json!({
                        "id": view.spec.id,
                        "label": view.spec.label,
                        "status": format!("{:?}", view.status),
                        "turns": view.turns,
                    })
                })
                .collect();
            Ok(json!({ "nodes": nodes }).to_string())
        }
        "get_node" => {
            let node = string_arg(&arguments, "node")?;
            let view = ask(commands, |reply| Command::GetNode {
                node: node.clone(),
                reply,
            })
            .await?;
            match view {
                Some(view) => Ok(serde_json::to_string(&view).unwrap_or_default()),
                None => Err(format!("no node {node}")),
            }
        }
        "get_transcript" => {
            let node = string_arg(&arguments, "node")?;
            let transcript = ask(commands, |reply| Command::Transcript {
                node: node.clone(),
                reply,
            })
            .await?;
            Ok(serde_json::to_string(&transcript).unwrap_or_default())
        }
        "stage_node" => {
            let spec = node_spec(&arguments)?;
            let id = stage_or_spawn(commands, spec, false).await?;
            Ok(format!("staged {id}"))
        }
        "execute" => fire(commands, Command::Execute),
        "stop_all" => fire(commands, Command::StopAll),
        "set_workspace" => fire(
            commands,
            Command::SetWorkspace {
                path: string_arg(&arguments, "path")?,
            },
        ),
        "spawn_node" => {
            let spec = node_spec(&arguments)?;
            let id = stage_or_spawn(commands, spec, true).await?;
            Ok(format!("spawned {id}"))
        }
        "stop_node" => fire(
            commands,
            Command::Stop {
                node: string_arg(&arguments, "node")?,
            },
        ),
        "pause_node" => fire(
            commands,
            Command::Pause {
                node: string_arg(&arguments, "node")?,
            },
        ),
        "resume_node" => fire(
            commands,
            Command::Resume {
                node: string_arg(&arguments, "node")?,
            },
        ),
        "send_prompt" => fire(
            commands,
            Command::SendPrompt {
                node: string_arg(&arguments, "node")?,
                text: string_arg(&arguments, "text")?,
            },
        ),
        "add_edge" => fire(
            commands,
            Command::AddEdge {
                edge: edge(&arguments)?,
            },
        ),
        "remove_edge" => fire(
            commands,
            Command::RemoveEdge {
                id: string_arg(&arguments, "edge")?,
            },
        ),
        other => Err(format!("unknown tool: {other}")),
    }
}

async fn ask<T>(
    commands: &mpsc::UnboundedSender<Command>,
    make: impl FnOnce(oneshot::Sender<T>) -> Command,
) -> Result<T, String> {
    let (reply, answer) = oneshot::channel();
    commands
        .send(make(reply))
        .map_err(|_| "orchestrator is gone".to_string())?;
    answer
        .await
        .map_err(|_| "no reply from orchestrator".to_string())
}

async fn stage_or_spawn(
    commands: &mpsc::UnboundedSender<Command>,
    spec: NodeSpec,
    spawn: bool,
) -> Result<String, String> {
    let (reply, answer) = oneshot::channel();
    let command = if spawn {
        Command::Spawn {
            spec,
            reply: Some(reply),
        }
    } else {
        Command::Stage {
            spec,
            reply: Some(reply),
        }
    };
    commands
        .send(command)
        .map_err(|_| "orchestrator is gone".to_string())?;
    match answer.await {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("no reply from orchestrator".to_string()),
    }
}

fn fire(commands: &mpsc::UnboundedSender<Command>, command: Command) -> Result<String, String> {
    commands
        .send(command)
        .map_err(|_| "orchestrator is gone".to_string())?;
    Ok("ok".to_string())
}

fn string_arg(arguments: &Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{key} is required"))
}

fn node_spec(arguments: &Value) -> Result<NodeSpec, String> {
    let id = string_arg(arguments, "id")?;
    let system_prompt = string_arg(arguments, "system_prompt")?;
    let label = arguments
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or(&id)
        .to_string();
    let cwd = arguments
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or(".")
        .to_string();
    let allowed_tools = arguments
        .get("allowed_tools")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let model = arguments
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(NodeSpec {
        id,
        label,
        system_prompt,
        cwd,
        allowed_tools,
        model,
    })
}

fn edge(arguments: &Value) -> Result<Edge, String> {
    let from = string_arg(arguments, "from")?;
    let to = string_arg(arguments, "to")?;
    let prompt_template = arguments
        .get("prompt_template")
        .and_then(Value::as_str)
        .unwrap_or("{output}")
        .to_string();
    let trigger = match arguments.get("when").and_then(Value::as_str) {
        Some("contains") => Trigger::OnContains {
            needle: string_arg(arguments, "needle")?,
        },
        Some("match") => Trigger::OnMatch {
            pattern: string_arg(arguments, "pattern")?,
        },
        _ => Trigger::OnTurnEnd,
    };
    Ok(Edge {
        id: format!("{from}->{to}"),
        from,
        to,
        trigger,
        prompt_template,
    })
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "get_graph",
            "Read the whole graph: every node's spec, status, turn count, last output, and every edge. The authoritative picture.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "list_nodes",
            "List every node with its id, label, status, and turn count. Cheaper than get_graph.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "get_node",
            "Read one node's full state: spec, status, turn count, and last output.",
            json!({ "type": "object", "properties": { "node": { "type": "string" } }, "required": ["node"] }),
        ),
        tool(
            "get_transcript",
            "Read one node's full normalized transcript: prompts in, and assistant, thinking, tool, and result lines out. Use this to interrogate why a node is doing what it is doing.",
            json!({ "type": "object", "properties": { "node": { "type": "string" } }, "required": ["node"] }),
        ),
        tool(
            "stage_node",
            "Add a Claude Code agent to the graph as a design, without starting it. id is its handle, system_prompt its role. cwd defaults to the workspace, allowed_tools restricts its tools (default all), model overrides the model. Stage the whole graph, then call execute.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "system_prompt": { "type": "string" },
                    "label": { "type": "string" },
                    "cwd": { "type": "string" },
                    "allowed_tools": { "type": "array", "items": { "type": "string" } },
                    "model": { "type": "string" }
                },
                "required": ["id", "system_prompt"]
            }),
        ),
        tool(
            "execute",
            "Start every staged node: spawn its subprocess and move it to idle, ready for prompts and edges. Only call this once the user has approved the design and asked to run it.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "stop_all",
            "The kill switch: stop every running agent at once. The staged design stays so it can be rerun.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "set_workspace",
            "Set the directory new agents run in, so the whole graph shares one project's context. Pass an absolute path, like the repo the user wants to work on.",
            json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] }),
        ),
        tool(
            "spawn_node",
            "Add a Claude Code agent and start it immediately, for live additions to a running graph. Prefer stage_node plus execute when designing a graph up front.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "system_prompt": { "type": "string" },
                    "label": { "type": "string" },
                    "cwd": { "type": "string" },
                    "allowed_tools": { "type": "array", "items": { "type": "string" } },
                    "model": { "type": "string" }
                },
                "required": ["id", "system_prompt"]
            }),
        ),
        tool(
            "send_prompt",
            "Send a one-off prompt straight to a node's input, starting a turn.",
            json!({
                "type": "object",
                "properties": { "node": { "type": "string" }, "text": { "type": "string" } },
                "required": ["node", "text"]
            }),
        ),
        tool(
            "add_edge",
            "Wire one node's finished turns into another's input, forming a loop. when is 'turn_end' (default), 'contains' (with needle), or 'match' (with regex pattern). prompt_template is sent to `to` with {output} replaced by `from`'s output.",
            json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "prompt_template": { "type": "string" },
                    "when": { "type": "string", "enum": ["turn_end", "contains", "match"] },
                    "needle": { "type": "string" },
                    "pattern": { "type": "string" }
                },
                "required": ["from", "to"]
            }),
        ),
        tool(
            "remove_edge",
            "Remove an edge by id (its id is 'from->to').",
            json!({ "type": "object", "properties": { "edge": { "type": "string" } }, "required": ["edge"] }),
        ),
        tool(
            "stop_node",
            "Kill a node's subprocess and mark it done.",
            json!({ "type": "object", "properties": { "node": { "type": "string" } }, "required": ["node"] }),
        ),
        tool(
            "pause_node",
            "Hold a node's inbound edges; prompts queue until it is resumed.",
            json!({ "type": "object", "properties": { "node": { "type": "string" } }, "required": ["node"] }),
        ),
        tool(
            "resume_node",
            "Resume a paused node and deliver any queued prompts.",
            json!({ "type": "object", "properties": { "node": { "type": "string" } }, "required": ["node"] }),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

fn rpc_result(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn rpc_error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": { "code": code, "message": message } })
}
