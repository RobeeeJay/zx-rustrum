//! An MCP server over the emulator, so a language model can drive it.
//!
//! See [`docs/mcp.md`](../../docs/mcp.md). The transport is JSON-RPC 2.0 over
//! stdin and stdout, one message a line; `Server::handle` takes a request and
//! gives back the reply, which is what the tests drive.

pub mod activity;
pub mod analysis;
pub mod catalogue;
pub mod control;
pub mod coverage;
pub mod deck;
pub mod disk;
pub mod input;
pub mod json;
pub mod listing;
pub mod looking;
pub mod memory;
pub mod microdrive;
pub mod names;
pub mod peripherals;
pub mod picture;
pub mod sound;
pub mod tools;
pub mod zx81;

pub use tools::Session;

use json::Json;

/// The version of the protocol this speaks.
pub const PROTOCOL: &str = "2024-11-05";

pub struct Server {
    pub session: Session,
    /// Whether the client has said hello. Nothing is refused for want of it —
    /// a server that will not answer until greeted is a server that is hard
    /// to test by hand — but it is worth knowing.
    pub initialized: bool,
}

impl Default for Server {
    fn default() -> Self {
        Server::new()
    }
}

impl Server {
    pub fn new() -> Server {
        Server {
            session: Session::new(),
            initialized: false,
        }
    }

    /// Answer one message. `None` where the message was a notification, which
    /// by JSON-RPC's rules is answered with silence.
    pub fn handle(&mut self, line: &str) -> Option<String> {
        let request = match json::parse(line) {
            Ok(request) => request,
            Err(why) => return Some(error(Json::Null, -32700, &format!("parse error: {why}"))),
        };
        let id = request.get("id").cloned().unwrap_or(Json::Null);
        let Some(method) = request.get("method").and_then(|m| m.as_str()) else {
            return Some(error(id, -32600, "no method"));
        };
        let params = request.get("params").cloned().unwrap_or(Json::Null);

        // A notification has no id, and gets no reply whatever it asks for.
        let notification = request.get("id").is_none();
        let answer = self.dispatch(method, &params);
        if notification {
            return None;
        }
        Some(match answer {
            Ok(result) => reply(id, result),
            Err(Fault { code, message }) => error(id, code, &message),
        })
    }

    fn dispatch(&mut self, method: &str, params: &Json) -> Result<Json, Fault> {
        match method {
            "initialize" => {
                self.initialized = true;
                Ok(Json::obj([
                    ("protocolVersion", Json::str(PROTOCOL)),
                    ("capabilities", Json::obj([("tools", Json::obj([]))])),
                    (
                        "serverInfo",
                        Json::obj([
                            ("name", Json::str("zx-rustrum")),
                            ("version", Json::str(env!("CARGO_PKG_VERSION"))),
                        ]),
                    ),
                    ("instructions", Json::str(tools::INSTRUCTIONS)),
                ]))
            }
            "notifications/initialized" | "notifications/cancelled" => Ok(Json::obj([])),
            "ping" => Ok(Json::obj([])),
            "tools/list" => Ok(Json::obj([("tools", catalogue::tools())])),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| Fault::new(-32602, "tools/call needs a name"))?
                    .to_string();
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(Json::Obj(vec![]));
                // A tool that fails is not a protocol error: the model is told
                // what went wrong and can try something else, which is what
                // isError is for.
                Ok(match self.session.call(&name, &arguments) {
                    Ok(reply) => reply.content(),
                    Err(why) => content(&why, true),
                })
            }
            other => Err(Fault::new(-32601, &format!("no method {other:?}"))),
        }
    }
}

/// A JSON-RPC error: a code and something to read.
pub struct Fault {
    pub code: i64,
    pub message: String,
}

impl Fault {
    pub fn new(code: i64, message: &str) -> Fault {
        Fault {
            code,
            message: message.to_string(),
        }
    }
}

pub fn content(text: &str, is_error: bool) -> Json {
    Json::obj([
        (
            "content",
            Json::arr(vec![Json::obj([
                ("type", Json::str("text")),
                ("text", Json::str(text)),
            ])]),
        ),
        ("isError", Json::Bool(is_error)),
    ])
}

fn reply(id: Json, result: Json) -> String {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", id),
        ("result", result),
    ])
    .to_string()
}

fn error(id: Json, code: i64, message: &str) -> String {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", id),
        (
            "error",
            Json::obj([
                ("code", Json::num(code as f64)),
                ("message", Json::str(message)),
            ]),
        ),
    ])
    .to_string()
}

/// Read messages from `input` and write the replies to `output`, a line each.
pub fn serve(input: impl std::io::BufRead, mut output: impl std::io::Write) -> std::io::Result<()> {
    let mut server = Server::new();
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle(&line) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}
