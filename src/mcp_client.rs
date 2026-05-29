use std::process::{Command, Stdio, Child, ChildStdin, ChildStdout};
use std::io::{BufRead, BufReader, Write};
use serde_json::{json, Value};
use anyhow::{Result, Context};
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
            
        let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("Failed to open stdin for MCP server"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("Failed to open stdout for MCP server"))?;
        
        Ok(Self {
            name: name.to_string(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            request_id: 1,
        })
    }
    
    pub fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<String> {
        let id = self.request_id;
        self.request_id += 1;
        
        let request = json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            },
            "id": id
        });
        
        let request_str = serde_json::to_string(&request)?;
        info!("MCP Request to '{}': {}", self.name, request_str);
        
        // Write request + newline
        writeln!(self.stdin, "{}", request_str)?;
        self.stdin.flush()?;
        
        // Read response line
        let mut response_line = String::new();
        self.stdout.read_line(&mut response_line)
            .with_context(|| format!("Failed to read response from MCP server '{}'", self.name))?;
            
        info!("MCP Response from '{}': {}", self.name, response_line.trim());
        
        let response_val: Value = serde_json::from_str(&response_line)?;
        
        if let Some(error) = response_val.get("error") {
            return Err(anyhow::anyhow!("MCP server returned error: {:?}", error));
        }
        
        let result = response_val.get("result")
            .ok_or_else(|| anyhow::anyhow!("Missing result in MCP response"))?;
            
        let content_text = if let Some(content_array) = result.get("content").and_then(|c| c.as_array()) {
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
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
