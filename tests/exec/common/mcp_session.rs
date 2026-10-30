use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
};

use crossbeam_channel::{Receiver, unbounded};
use serde_json::Value;

use super::{scratch::Scratch, wait::WAIT};

/// The MCP protocol version the test client speaks.
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
/// The JSON-RPC version every MCP message carries.
const JSON_RPC_VERSION: &str = "2.0";

/// A `muster mcp` server run over stdio, spoken to as an agent would. Each
/// line it prints is a JSON message, or the raw line when it is not one.
pub struct McpSession {
    /// The server.
    child: Child,
    /// What it printed, a line at a time.
    lines: Receiver<Result<Value, String>>,
    /// The id the last request took.
    next_id: u64,
}

impl McpSession {
    /// Starts `muster mcp` for `project` over `scratch` and initializes it.
    pub fn start(scratch: &mut Scratch, project: &Path) -> Self {
        Self::start_placed(scratch, project, |_| {})
    }

    /// Starts `muster mcp` for `project` over `scratch`, its command first
    /// changed by `place`, and initializes it.
    pub fn start_placed(
        scratch: &mut Scratch,
        project: &Path,
        place: impl FnOnce(&mut Command),
    ) -> Self {
        let mut command =
            scratch.command(&["-p", project.to_str().unwrap(), "mcp", "--as", "test-agent"]);
        place(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        scratch.keep_child(child.id());
        let out = child.stdout.take().unwrap();
        let (sender, lines) = unbounded();
        thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { return };
                let message = serde_json::from_str::<Value>(&line).map_err(|_| line);
                if sender.send(message).is_err() {
                    return;
                }
            }
        });
        let mut session = Self {
            child,
            lines,
            next_id: 0,
        };
        session.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "cli-exec", "version": "0" },
            }),
        );
        session.write(&serde_json::json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": "notifications/initialized",
        }));
        session
    }

    /// The server's pid.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Writes one message to the server.
    pub fn write(&mut self, message: &Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    /// Sends `method` with `params` and returns the whole response.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&serde_json::json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        }));
        loop {
            let message = self
                .lines
                .recv_timeout(WAIT)
                .unwrap_or_else(|_| panic!("no answer to {method}"))
                .unwrap_or_else(|line| panic!("stdout carried a non-JSON line: {line:?}"));
            if message["id"] == id {
                return message;
            }
        }
    }

    /// Calls the tool `name` once with each of `calls`, every request
    /// written before any answer is read, as an agent's parallel calls
    /// arrive; returns the responses in the order of `calls`.
    pub fn pipelined(&mut self, name: &str, calls: Vec<Value>) -> Vec<Value> {
        let first = self.next_id + 1;
        for arguments in calls {
            self.next_id += 1;
            let id = self.next_id;
            self.write(&serde_json::json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": id,
                "method": "tools/call",
                "params": { "name": name, "arguments": arguments },
            }));
        }
        let mut answers: Vec<Option<Value>> =
            vec![None; usize::try_from(self.next_id + 1 - first).unwrap()];
        while answers.iter().any(Option::is_none) {
            let message = self
                .lines
                .recv_timeout(WAIT)
                .unwrap_or_else(|_| panic!("no answer to every {name}"))
                .unwrap_or_else(|line| panic!("stdout carried a non-JSON line: {line:?}"));
            if let Some(slot) = message["id"]
                .as_u64()
                .and_then(|id| id.checked_sub(first))
                .and_then(|index| answers.get_mut(usize::try_from(index).ok()?))
            {
                *slot = Some(message);
            }
        }
        answers.into_iter().flatten().collect()
    }

    /// Calls the tool `name` with `arguments` and returns the response.
    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": arguments }),
        )
    }

    /// Calls the tool `name` with `arguments` and returns its result,
    /// failing on an error.
    pub fn result(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.call(name, arguments);
        assert!(response["error"].is_null(), "{name} failed: {response}");
        response["result"].clone()
    }

    /// The text of the first content block of the tool call's result.
    pub fn text(&mut self, name: &str, arguments: Value) -> String {
        result_text(&self.result(name, arguments))
    }

    /// Calls the tool `name` with `arguments`, which the caller got wrong,
    /// and returns its result, failing unless it is flagged as an error.
    pub fn mistake(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.result(name, arguments);
        assert_eq!(result["isError"], true, "{name} succeeded: {result}");
        result
    }
}

impl Drop for McpSession {
    /// Closes the server's input, which ends it, and reaps it.
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

/// The text of the first content block of a tool result.
pub fn result_text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}
