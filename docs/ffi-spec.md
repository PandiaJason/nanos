# 📡 The nanos JSON-RPC FFI Protocol Spec

When running JavaScript/TypeScript agents compiled via the SDK, the agent runs in an ultra-restricted Node.js subprocess that communicates with the `nanos` parent host process over synchronous stdout/stdin JSON-RPC 2.0. This allows running standard JS/TS code with zero capability leakage.

## System Calls (Syscalls)

### 1. `fs_read` (File Read)
Reads a file's contents from the host filesystem. Subject to the manifest's `permissions.fs_read` whitelist.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "fs_read", "params": ["instruction.txt"], "id": 1 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "The secret code is 42.\n", "id": 1 }
  ```

### 2. `fs_write` (File Write)
Writes contents to a file on the host filesystem. Subject to the manifest's `permissions.fs_write` whitelist.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "fs_write", "params": ["secret.txt", "42"], "id": 2 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "Successfully wrote to file.", "id": 2 }
  ```

### 3. `llm_infer` (LLM Inference)
Triggers a local GPU/LLM inference request. The prompt token count and response generation speed are tracked in the execution logs and visual web debugger.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "llm_infer", "params": ["Summarize code: The secret code is 42."], "id": 3 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "The secret code is 42.", "id": 3 }
  ```

### 4. `web_get` (Network HTTP Get)
Performs an HTTP client get request from the host. Subject to the manifest's `permissions.network` boolean.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "web_get", "params": ["https://api.github.com/repos/PandiaJason/nanos"], "id": 4 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "{ \"id\": ... }", "id": 4 }
  ```

### 5. `done` (Execution Finished)
Notifies the host that the agent has accomplished its goal and supplies an execution summary.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "done", "params": ["Agent FFI Loop completed successfully."], "id": 5 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "Done", "id": 5 }
  ```

### 6. `get_manifest_goal` (Get Agent Goal)
Retrieves the target goal description specified in the agent's manifest.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "get_manifest_goal", "params": [], "id": 6 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "Extract the secret key from instruction.txt and write it to secret.txt", "id": 6 }
  ```

### 7. `get_manifest_tools` (Get Allowed Tools List)
Retrieves the list of tools permitted for the agent in the manifest, comma-separated.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "get_manifest_tools", "params": [], "id": 7 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "fs_read,fs_write,done", "id": 7 }
  ```

### 8. `agent_send` (Send Inter-Agent Message)
Sends an asynchronous message to another agent in the fleet message queue.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "agent_send", "params": { "target": "writer", "msg": "secret-code-xyz" }, "id": 8 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "Message sent successfully.", "id": 8 }
  ```

### 9. `agent_recv` (Receive Inter-Agent Message)
Retrieves the next message from the agent's input queue. Blocks or returns if no message is found.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "agent_recv", "params": [], "id": 9 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "secret-code-xyz", "id": 9 }
  ```

### 10. `mcp_call` (Call MCP Server Tool)
Proxies a tool call request to a specified external MCP server over stdio JSON-RPC.
* **Request:**
  ```json
  {
    "jsonrpc": "2.0",
    "method": "mcp_call",
    "params": {
      "server": "ping-server",
      "tool": "ping",
      "arguments": { "message": "hello" }
    },
    "id": 10
  }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "Hello from MCP Ping Server! Arguments received: {\"message\":\"hello\"}", "id": 10 }
  ```
