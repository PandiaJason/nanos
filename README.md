<div align="center">
  <img src="assets/nanos_logo.png" alt="nanos Logo" width="180">
  <h1>⚡ nanos</h1>
  <p><b>The lightweight, secure, and ultra-fast WebAssembly micro-runtime for AI agents.</b></p>

  <p>
    <a href="https://github.com/PandiaJason/nanos/actions"><img src="https://img.shields.io/badge/build-passing-success?style=for-the-badge&logo=github" alt="Build Status"></a>
    <a href="https://crates.io/crates/nanos"><img src="https://img.shields.io/badge/crates.io-v0.1.0-orange?style=for-the-badge&logo=rust" alt="Rust Crates"></a>
    <a href="https://webassembly.org/"><img src="https://img.shields.io/badge/runtime-WASM-blueviolet?style=for-the-badge&logo=webassembly" alt="WASM"></a>
    <img src="https://img.shields.io/badge/sandbox-hardware--isolated-00cc88?style=for-the-badge" alt="Sandboxed">
    <img src="https://img.shields.io/badge/GPU-Metal%20%2F%20CUDA-ff6b6b?style=for-the-badge" alt="GPU">
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=for-the-badge" alt="License"></a>
  </p>

  <h3>📉 30x RAM Reduction (< 15MB RSS vs ~450MB) · ⚡ < 50ms Sandbox Boot · zero docker · zero python</h3>
  <p><b>Just the agent, the weights, and the silicon.</b></p>
</div>

---

## 🚨 Project Status: Active Research Prototype

`nanos` is currently in active research and development. To keep this project grounded and credible for developers, we draw an honest line between what is fully working today, what is currently in active development, and what is planned for the roadmap.

| Status | Features | Technical Justification & Current State |
| :--- | :--- | :--- |
| **✅ Working Today** | • **Single-Agent WASM Sandbox**<br>• **Metal GPU Offload (macOS)**<br>• **Local GGUF & Ollama**<br>• **In-Memory FFI Syscalls** (`fs`, `llm`, `web`) <br>• **JS/TS SDK Compiler** | Wasmtime fuel limits, memory caps, and direct macOS Metal GPU mapping compile and run cleanly today. System calls (`fs_read`, `fs_write`, `eval_js`, `llm_infer`, `web_get`) run entirely in-memory with zero network overhead. JS/TS compilation via `nanos-compile.js` and Node sandbox dynamic permission routing is fully functional. |
| **🔧 In Progress** | • **Multi-Agent Fleet Orchestration**<br>• **MCP stdio JSON-RPC Client** | Local thread-based fleet execution and message queues work, but distributed scaling is in progress. The MCP Client spawns and routes tool calls to JSON-RPC servers but lacks full spec hooks. |
| **📋 Planned Roadmap** | • **NPM Registry Package (`nanos-sdk`)**<br>• **Time-Travel Debugger GUI**<br>• **Linux CUDA Backend** | Publishing `nanos-sdk` to NPM for easier global installation. The interactive time-travel debugger works in CLI mode inside the dashboard; a visual web GUI debugger is planned. Native CUDA GPU mapping for Linux is on the roadmap. |

---

## 💡 What is nanos?

**nanos** is a Rust-native, WebAssembly-powered micro-runtime for AI agents. By executing compiled agent binaries inside a hardware-isolated WebAssembly sandbox (Wasmtime), it cuts the typical runtime RAM footprint from **~450MB (Python/Docker) to < 15MB**, while booting the VM in **< 50ms**. 

Rather than deploying agents as bloated virtual machines that talk to tools over HTTP, `nanos` executes tool calls via direct, in-memory **Foreign Function Interface (FFI) pointer passing**. The host and the agent share a zero-copy memory boundary, eliminating JSON serialization latency and local TCP socket overhead.

---

## 📖 About & Philosophy

`nanos` was built to address the **agent deployment crisis**. 

As LLMs become smaller, faster, and capable of running locally (e.g., Llama 3, Qwen 2.5), the bottlenecks of agent execution have shifted. It is no longer just the model inference time that slows down applications—it is the glue code, the network roundtrips between isolated components, and the astronomical RAM overhead of running a Docker container for every single agent step.

### The Agent as a Micro-Kernel
We view an AI agent not as a web service, but as an **operating system process**. 
An agent is simply a loop that reads input, reasons, runs a tool, and updates its state. By compiling agent code to WebAssembly, `nanos` treats tool calls as standard OS system calls (syscalls). Wasmtime intercepts these calls, validates permission rules, and executes the tools natively on the host at hardware speeds with zero virtualization overhead.

### Local-First & Air-Gapped Philosophy
Agents should run where the data lives. `nanos` is designed to run entirely locally without requiring external cloud accounts or internet connectivity. By mapping local GPU hardware (Apple Metal and CUDA) and running local model providers, agents can perform private, secure, and low-latency work on-device.

---

## 🚨 The Problem with Current Agent Stacks

Every AI agent framework today suffers from massive latency, memory bloat, and security vulnerabilities. A typical stack looks like this:

> `Docker (200MB) → Python (2s boot) → pip install langchain (500MB) → MCP server (HTTP daemon) → LLM API (TCP socket, JSON serialize, wait, parse)`

Every arrow represents latency, memory consumption, and a larger attack surface. 

**nanos** throws out the entire stack:

> `nanos run agent.nano → WASM sandbox boots (< 50ms) → weights memory-mapped to GPU → tool calls via FFI pointer pass (zero copy) → done.`

One binary. One process. No network. No serialization tax.

<p align="center">
  <img src="assets/nanos_stack_comparison.png" alt="nanos Stack Comparison" width="750">
</p>

---

## 🏗️ Architecture

Instead of isolated HTTP servers, `nanos` uses WebAssembly linear memory isolation. Tool calls pass raw memory pointers across the WASM boundary. A 1MB document and a 10-byte string cost exactly the same: **one pointer offset**.

<p align="center">
  <b>High-Level Platform Overview</b><br>
  <img src="assets/nanos_architecture_overview.png" alt="nanos Architecture Overview" width="750">
</p>

<br>

<p align="center">
  <b>Detailed Runtime Architecture</b><br>
  <img src="assets/nanos_architecture_detail.png" alt="nanos Detailed Architecture" width="750">
</p>

---

## ⚡ Benchmarks

*qwen2.5-coder 0.5B on Apple M1 Pro, 24/24 Metal GPU layers offloaded:*

| Metric / Stack | ollama + python + docker | **nanos (WASM + FFI)** | **Delta** |
| :--- | :---: | :---: | :---: |
| **RAM Footprint** | ~450 MB | **< 15 MB** | 📉 **30x smaller** (Strongest Claim) |
| **Cold Start** | 29,562 ms | **12,420 ms** | 🚀 **2.3x faster** |
| **Warm Inference** | 1,166 ms | **992 ms** | ⚡ **15% faster** |

*Note: RAM footprint excludes loaded LLM weights, measuring only the container/runtime overhead.*

---

## ✨ Features

### 🔐 Hardware-Isolated WASM Sandbox (Working)
Every agent runs inside a strict `wasmtime` store:
- **Linear memory isolation:** Agents cannot access host memory beyond their sandbox bounds.
- **Fuel metering:** Execution budget is enforced directly at the VM instruction level to prevent infinite loops.
- **Memory caps:** `StoreLimits` enforce max WASM heap allocation from the manifest.
- **Permission-gated syscalls:** `fs_read` and `fs_write` require explicit directory paths in your manifest. Everything else is **deny-by-default**.

### 🎮 Apple Metal GPU Offload (Working)
Model weights are memory-mapped directly onto macOS graphics hardware via `llama.cpp`'s native Metal GPU layers.

### 🤖 Multi-Agent Fleet Orchestration (In Progress)
Orchestrate multiple agents concurrently sharing a single local `LlmEngine` instance. Agents communicate via thread-safe message queues.
* **Current state:** Implemented locally using OS threads and a mutex-guarded `MessageBus` queue (`src/orchestrator.rs`).
* **In progress:** Scaling this model to distributed nodes (e.g. over TCP/gRPC).

### 🕰️ Time-Travel Debugger (Prototype CLI)
Inspect any step's exact execution trace and replay the agent from that step with a modified environment or mock tool observations.
* **Current state:** Accessible via an interactive text-based CLI terminal when running in `nanos dashboard` mode.
* **In progress:** A web-based visual debugger interface.

### 🔌 Universal MCP Tool Proxy (In Progress)
Bridge standard Model Context Protocol (MCP) servers straight to WASM.
* **Current state:** Standard stdio JSON-RPC 2.0 communication is working (`src/mcp_client.rs`), spawning and managing server subprocesses.
* **In progress:** Complete implementation of full protocol capabilities (prompts, resources, validation).

### 🛡️ Sandboxed `eval_js` Syscall (Working)
WASM agents can execute dynamic JavaScript code safely via a dedicated host syscall.
* **Current state:** Enforced via Node.js `--permission` (or `--experimental-permission`) flags, denying filesystem, network, and child process access by default. Capped by execution timeouts and memory heap limits.

---

## 📡 The nanos JSON-RPC FFI Protocol Spec

When running JavaScript/TypeScript agents compiled via the SDK, the agent runs in an ultra-restricted Node.js subprocess that communicates with the `nanos` parent host process over synchronous stdout/stdin JSON-RPC 2.0. This allows running standard JS/TS code with zero capability leakage.

### System Calls (Syscalls)

#### 1. `fs_read` (File Read)
Reads a file's contents from the host filesystem. Subject to the manifest's `permissions.fs_read` whitelist.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "fs_read", "params": ["instruction.txt"], "id": 1 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "The secret code is 42.\n", "id": 1 }
  ```

#### 2. `fs_write` (File Write)
Writes contents to a file on the host filesystem. Subject to the manifest's `permissions.fs_write` whitelist.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "fs_write", "params": ["secret.txt", "42"], "id": 2 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "Successfully wrote to file.", "id": 2 }
  ```

#### 3. `llm_infer` (LLM Inference)
Triggers a local GPU/LLM inference request. The prompt token count and response generation speed are tracked on the TUI dashboard.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "llm_infer", "params": ["Summarize code: The secret code is 42."], "id": 3 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "The secret code is 42.", "id": 3 }
  ```

#### 4. `web_get` (Network HTTP Get)
Performs an HTTP client get request from the host. Subject to the manifest's `permissions.network` boolean.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "web_get", "params": ["https://api.github.com/repos/PandiaJason/nanos"], "id": 4 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "{ \"id\": ... }", "id": 4 }
  ```

#### 5. `done` (Execution Finished)
Notifies the host that the agent has accomplished its goal and supplies an execution summary.
* **Request:**
  ```json
  { "jsonrpc": "2.0", "method": "done", "params": ["Agent FFI Loop completed successfully."], "id": 5 }
  ```
* **Response:**
  ```json
  { "jsonrpc": "2.0", "result": "Done", "id": 5 }
  ```

---

## 🚀 Quick Start

### 1. Prerequisites
Ensure you have the following installed on your host:
* Rust & Cargo (MSRV 1.75+)
* Node.js (v18+ for compiling, v20+ with permission support is recommended for the JS sandbox runner)

### 2. Build the Nanos Engine
Clone and compile the native runtime binary:
```bash
# Clone the repository
git clone https://github.com/PandiaJason/nanos && cd nanos

# Build the nanos runtime CLI
cargo build --release
```

### 3. Option A: Run a Rust Agent
Build the default Rust agent core into WebAssembly:
```bash
# Compile core agent to WASM target
cd nanos-core-agent && cargo build --target wasm32-unknown-unknown && cd ..

# Setup example file
cp examples/instruction.txt .

# Execute the agent manifest
./target/release/nanos run examples/agent.nano
```
Upon run, the engine will boot the sandbox, map model weights, load the compiled Rust agent binary (`nanos-core-agent/target/wasm32-unknown-unknown/debug/nanos_core_agent.wasm`), and safely execute it.

---

### 4. Option B: Write, Compile & Run a JS/TS Agent
`nanos` provides a high-level SDK that lets you write agents in TypeScript/JavaScript, compile them into WASM dynamic bundles, and execute them under the sandboxed host router.

#### Write the agent code (`examples/test_agent.ts`):
```typescript
import { fs, llm, agent } from '../nanos-sdk/index.js';

export async function run() {
  console.log("TS Agent started!");
  const goal = await agent.getGoal();
  console.log("TS Goal received:", goal);

  const inputData = await fs.readFile("instruction.txt");
  console.log("TS Read instruction.txt:", inputData);

  const response = await llm.infer(`Summarize code: ${inputData}`);
  console.log("TS LLM Inference result:", response);

  await fs.writeFile("secret.txt", response);
  console.log("TS Wrote secret.txt");

  await agent.done("TS FFI Loop completed successfully.");
}

run().catch(err => {
  console.error("TS Agent execution failed:", err);
  process.exit(1);
});
```

#### Compile it:
Compile the TypeScript code to a WASM bundle package using the custom `nanos-compile.js` tool:
```bash
node nanos-sdk/bin/nanos-compile.js examples/test_agent.ts --out dist/test_agent.wasm --engine bundle
```

#### Run it:
Define the JS agent manifest configuration (`examples/agent_js.nano`):
```yaml
name: "nanos-js-agent"
model:
  provider: "ollama"
  model_name: "qwen2.5-coder:0.5b"
  context_window: 4096
resources:
  memory: "512MB"
  max_steps: 10
permissions:
  fs_read:
    - "instruction.txt"
  fs_write:
    - "secret.txt"
binary: "dist/test_agent.wasm"
goal: "Extract the secret key from instruction.txt and write it to secret.txt"
```

Then run the sandboxed JS/TS agent:
```bash
# Setup instruction file
cp examples/instruction.txt .

# Execute
./target/release/nanos run examples/agent_js.nano
```

---

### 5. Launch the Fleet Dashboard & Interactive TUI
If you want to view real-time multi-agent fleet orchestration or play with the Time-Travel Debugger, launch the interactive TUI dashboard:

```bash
./target/release/nanos dashboard examples/fleet.nano
```

<p align="center">
  <img src="assets/nanos_dashboard_showcase.png" alt="nanos TUI Dashboard" width="750">
</p>

Once execution finishes, choose a step index from the trace history to inject a mocked observation (e.g. mock a tool failure) and spawn a divergent execution replay!

---

## 🛠️ Manifest Reference (`.nano`)

Every agent is defined by a `.nano` YAML configuration file:

```yaml
name: "nanos-js-agent"       # Name of the agent instance
model:
  provider: "ollama"         # LLM Provider: 'ollama' | 'llama.cpp' (local GGUF)
  model_name: "qwen2.5..."   # Model name or file path
  context_window: 4096       # Context size limit
resources:
  memory: "512MB"            # Sandbox physical RAM heap limit
  max_steps: 10              # Maximum FFI syscall loop iterations allowed
permissions:
  fs_read:                   # Whitelist of files or glob patterns the agent can read
    - "instruction.txt"
  fs_write:                  # Whitelist of files or glob patterns the agent can write
    - "secret.txt"
  network: false             # Disable or enable external TCP socket access
binary: "dist/test_agent.wasm" # Target agent compilation binary
goal: "Extract the secret..." # Mission statement of the agent
```

---

## 🆚 Comparison Matrix: Why nanos?

| Feature | `nanos` ⚡ | E2B | LangChain | Docker + Python |
| :--- | :--- | :--- | :--- | :--- |
| **Cold Start** | **< 50ms** | ~2s | ~3s | ~30s |
| **RAM Overhead**| **< 15MB** | ~200MB | ~500MB | ~450MB |
| **Sandbox** | **WASM hardware-isolated** | Cloud VM container | None | Host container |
| **GPU Access** | **Direct Metal / CUDA** | ❌ None | ❌ None | Manual configuration |
| **Air-Gapped** | **✅ Yes** | ❌ No (Cloud only) | ❌ No | Partial |
| **Binary Size** | **Single ~15MB binary** | N/A | `pip install` | `docker pull` |

---

<div align="center">
  <b>nanos</b> — the agent doesn't need a cloud. it needs silicon.<br><br>
  <i>If you find this project valuable, please consider giving it a ⭐ on GitHub!</i>
</div>
