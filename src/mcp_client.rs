use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tracing::info;

pub struct McpClient {
    pub name: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    request_id: u64,
}

impl McpClient {
    pub fn spawn(name: &str, command: &str, args: &[String]) -> Result<Self> {
        info!("Spawning MCP server '{}' via: {} {:?}", name, command, args);
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()) // Let stderr print to host console for debugging
            .spawn()
            .with_context(|| format!("Failed to spawn MCP server '{}'", name))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open stdin for MCP server"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open stdout for MCP server"))?;

        Ok(Self {
            name: name.to_string(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            request_id: 1,
        })
    }

    pub fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.request_id;
        self.request_id += 1;

        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": id
        });

        let request_str = serde_json::to_string(&request)?;
        info!("MCP Request to '{}': {}", self.name, request_str);

        writeln!(self.stdin, "{}", request_str)?;
        self.stdin.flush()?;

        let mut response_line = String::new();
        self.stdout
            .read_line(&mut response_line)
            .with_context(|| format!("Failed to read response from MCP server '{}'", self.name))?;

        info!(
            "MCP Response from '{}': {}",
            self.name,
            response_line.trim()
        );

        let response_val: Value = serde_json::from_str(&response_line)?;

        if let Some(error) = response_val.get("error") {
            return Err(anyhow::anyhow!("MCP server returned error: {:?}", error));
        }

        let result = response_val
            .get("result")
            .ok_or_else(|| anyhow::anyhow!("Missing result in MCP response"))?
            .clone();

        Ok(result)
    }

    pub fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<String> {
        let params = json!({
            "name": tool_name,
            "arguments": arguments
        });
        let result = self.send_request("tools/call", params)?;

        let content_text =
            if let Some(content_array) = result.get("content").and_then(|c| c.as_array()) {
                let mut texts = Vec::new();
                for item in content_array {
                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                        texts.push(text.to_string());
                    }
                }
                texts.join("\n")
            } else {
                result.to_string()
            };

        Ok(content_text)
    }

    pub fn list_tools(&mut self) -> Result<Value> {
        self.send_request("tools/list", json!({}))
    }

    pub fn list_resources(&mut self) -> Result<Value> {
        self.send_request("resources/list", json!({}))
    }

    pub fn list_prompts(&mut self) -> Result<Value> {
        self.send_request("prompts/list", json!({}))
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_client_methods() {
        // Spawn a Node.js process that echo-returns responses
        let command = "node";
        let script = r#"
            const readline = require('readline');
            const rl = readline.createInterface({ input: process.stdin });
            rl.on('line', (line) => {
                const req = JSON.parse(line);
                console.log(JSON.stringify({
                    jsonrpc: "2.0",
                    id: req.id,
                    result: { method_called: req.method, params_received: req.params }
                }));
            });
        "#;

        let args = vec!["-e".to_string(), script.to_string()];
        let mut client = McpClient::spawn("echo-server", command, &args).unwrap();

        // Test list_tools
        let tools_res = client.list_tools().unwrap();
        assert_eq!(tools_res["method_called"], "tools/list");

        // Test list_resources
        let res_res = client.list_resources().unwrap();
        assert_eq!(res_res["method_called"], "resources/list");

        // Test list_prompts
        let prompts_res = client.list_prompts().unwrap();
        assert_eq!(prompts_res["method_called"], "prompts/list");
    }
}
