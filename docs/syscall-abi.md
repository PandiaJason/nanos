# WebAssembly System Call ABI Specification

This document details the binary interface (ABI) and memory layout conventions used for communication between the sandboxed WebAssembly (WASM) guest agent and the native `nanos` host runtime.

---

## Architecture Overview

Instead of virtualizing hardware or running an in-guest IP socket stack, `nanos` uses direct memory pointer sharing. The WASM guest and native host share a single linear memory region.

All system operations (filesystem access, web fetches, LLM inference, MCP tool invocations, and multi-agent message routing) are executed as **synchronous FFI system calls** (syscalls) exported from the host's `env` namespace.

```
+-------------------------------------------------------------+
|                        nanos Process                        |
|                                                             |
|   +-----------------------+                                 |
|   |  WASM Guest Sandbox   | (User Space Program)            |
|   |                       |                                 |
|   |  Memory: [Linear RAM] | <----+                          |
|   +-----------+-----------+      |                          |
|               |                  | Direct Memory Read/Write |
|               | FFI Syscall      |                          |
|               v                  |                          |
|   +-----------------------+      |                          |
|   |       Host Engine     | -----+                          |
|   |       (Rust/Metal)    | (Kernel Space Services)         |
|   +-----------------------+                                 |
+-------------------------------------------------------------+
```

---

## Memory Exchange Conventions

1. **Pointers**: All pointers passed across the boundary are 32-bit offsets (`i32`) relative to the base of the WASM guest's linear memory buffer.
2. **String/Buffer Passing**: Strings and binary payloads are passed as a pair of parameters: `(ptr: i32, len: i32)`. The guest is responsible for allocating this memory within its heap before invoking the syscall.
3. **Out Buffers**: For syscalls that return variable-length data, the guest passes an output buffer pointer and its maximum capacity: `(out_ptr: i32, out_max: i32)`. The host writes the response payload directly to `out_ptr` up to `out_max` bytes, and returns the actual number of bytes written as the FFI function's `i32` return value.
4. **Encoding**: All string parameters and text buffers are encoded as standard UTF-8. Structured data is serialized as JSON strings.

---

## Syscall Reference

All FFI exports are declared in the `env` namespace.

### Filesystem Operations

#### `fs_read`
Reads the contents of a file from the host filesystem.

*   **Signature**: `(ptr: i32, len: i32, out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `ptr`: Memory offset of the file path string.
    *   `len`: Length of the path string in bytes.
    *   `out_ptr`: Destination offset for the file contents.
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The number of bytes successfully written to `out_ptr`.
*   **Behavior**: If access is denied by the capability manifest or the file does not exist, the host writes an error string (e.g. `"[Security] PERMISSION_DENIED"`) to `out_ptr` and returns its length.

#### `fs_write`
Writes a buffer to a file in the host filesystem.

*   **Signature**: `(path_ptr: i32, path_len: i32, content_ptr: i32, content_len: i32, out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `path_ptr`: Memory offset of the target file path.
    *   `path_len`: Length of the path string.
    *   `content_ptr`: Memory offset of the data content to write.
    *   `content_len`: Length of the content data.
    *   `out_ptr`: Destination offset for the status string response (e.g., `"OK"`).
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The number of bytes written to `out_ptr` containing the status or error message.

---

### Manifest Configuration

#### `get_manifest_goal`
Retrieves the mission statement (goal) assigned to this agent in the `.nano` manifest.

*   **Signature**: `(out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `out_ptr`: Destination offset for the goal string.
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The length of the goal string written to `out_ptr`.

#### `get_manifest_tools`
Retrieves the JSON-serialized list of tools whitelisted for this agent.

*   **Signature**: `(out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `out_ptr`: Destination offset for the JSON string (e.g., `["fs_read", "fs_write"]`).
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The length of the JSON string written to `out_ptr`.

---

### Network & Web Services

#### `web_get`
Performs an HTTP GET request to an external URL.

*   **Signature**: `(ptr: i32, len: i32, out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `ptr`: Memory offset of the URL string.
    *   `len`: Length of the URL string.
    *   `out_ptr`: Destination offset for the response text.
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The number of bytes written to `out_ptr`.
*   **Behavior**: Blocked by the host unless `permissions.network` is explicitly set to `true` in the manifest.

---

### Memory & Vectors

#### `memory_store`
Saves a text snippet into the host-managed SQLite vector memory db (`nanos_memory.db`).

*   **Signature**: `(ptr: i32, len: i32) -> i32`
*   **Arguments**:
    *   `ptr`: Memory offset of the string content to store.
    *   `len`: Length of the content string.
*   **Returns**: `1` if the write succeeded, `0` otherwise.

#### `memory_recall`
Queries the host-managed memory database for semantically similar content.

*   **Signature**: `(ptr: i32, len: i32, out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `ptr`: Memory offset of the search query string.
    *   `len`: Length of the query string.
    *   `out_ptr`: Destination offset for the matched text results.
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The number of bytes written to `out_ptr`.

---

### Native LLM Inference

#### `llm_infer`
Routes prompt evaluation directly to the native host-side `LlmEngine` (backed by Apple Metal or Linux CUDA).

*   **Signature**: `(prompt_ptr: i32, prompt_len: i32, out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `prompt_ptr`: Memory offset of the raw prompt string.
    *   `prompt_len`: Length of the prompt string.
    *   `out_ptr`: Destination offset for the generated model response text.
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The number of bytes written to `out_ptr`.

---

### Model Context Protocol (MCP) Integration

#### `mcp_call`
Executes a tool on an external Model Context Protocol (MCP) server.

*   **Signature**: `(server_ptr: i32, server_len: i32, tool_ptr: i32, tool_len: i32, args_ptr: i32, args_len: i32, out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `server_ptr`: Memory offset of the registered MCP server name string.
    *   `server_len`: Length of the server name.
    *   `tool_ptr`: Memory offset of the target tool name string.
    *   `tool_len`: Length of the tool name.
    *   `args_ptr`: Memory offset of the JSON-serialized arguments object.
    *   `args_len`: Length of the arguments JSON string.
    *   `out_ptr`: Destination offset for the response string.
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The number of bytes written to `out_ptr`.

---

### Distributed Multi-Agent Messaging

#### `agent_send`
Dispatches a message payload to another agent queue over the message bus (local or TCP network).

*   **Signature**: `(target_ptr: i32, target_len: i32, msg_ptr: i32, msg_len: i32) -> i32`
*   **Arguments**:
    *   `target_ptr`: Memory offset of the recipient agent's name string.
    *   `target_len`: Length of the recipient name.
    *   `msg_ptr`: Memory offset of the message content payload.
    *   `msg_len`: Length of the message payload.
*   **Returns**: `1` if successfully queued, `0` otherwise.

#### `agent_recv`
Reads a message payload queued for the calling agent.

*   **Signature**: `(out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `out_ptr`: Destination offset for the received message string.
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The number of bytes written to `out_ptr`. If no message is present, returns `0`.

---

### Sandbox Code Execution

#### `eval_js`
Executes JavaScript code inside a sub-sandboxed environment.

*   **Signature**: `(code_ptr: i32, code_len: i32, out_ptr: i32, out_max: i32) -> i32`
*   **Arguments**:
    *   `code_ptr`: Memory offset of the JavaScript code string to execute.
    *   `code_len`: Length of the JS code string.
    *   `out_ptr`: Destination offset for the execution stdout/stderr results.
    *   `out_max`: Maximum buffer capacity of the destination.
*   **Returns**: The number of bytes written to `out_ptr`.
