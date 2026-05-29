<div align="center">
  <h1>⚡ nanos</h1>
  <p><b>The AI agent runtime that doesn't need your cloud.</b></p>

  <p>
    <a href="https://github.com/PandiaJason/nanos/actions"><img src="https://img.shields.io/badge/build-passing-success?style=for-the-badge&logo=github" alt="Build Status"></a>
    <a href="https://crates.io/crates/nanos"><img src="https://img.shields.io/badge/language-Rust-orange?style=for-the-badge&logo=rust" alt="Rust"></a>
    <a href="https://webassembly.org/"><img src="https://img.shields.io/badge/runtime-WASM-blueviolet?style=for-the-badge&logo=webassembly" alt="WASM"></a>
    <img src="https://img.shields.io/badge/sandbox-hardware--isolated-00cc88?style=for-the-badge" alt="Sandboxed">
    <img src="https://img.shields.io/badge/GPU-Metal%20%2F%20CUDA-ff6b6b?style=for-the-badge" alt="GPU">
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=for-the-badge" alt="License"></a>
  </p>

  <br>
  <img src="assets/nanos_logo.png" alt="nanos Logo" width="150">
  <br>

  <p><i>📉 30x RAM Reduction (< 15MB RSS vs ~450MB) · ⚡ < 50ms Sandbox Boot · zero docker · zero python</i><br><b>just the agent, the weights, and the silicon.</b></p>
</div>

---

## 🚨 Project Status: Research Prototype

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
* **Current state:** Enforced via Node.js `--experimental-permission` flags, denying filesystem, network, and child process access by default. Capped by execution timeouts and memory heap limits.

---

## 🚀 Quick Start

### 1. Install & Build
First, compile the `nanos` runtime and the core Rust-based agent WASM binary:

```bash
# Clone the repository
git clone https://github.com/PandiaJason/nanos && cd nanos

# Build the nanos runtime binary
cargo build --release

# Build the default Rust agent core into WebAssembly
cd nanos-core-agent && cargo build --target wasm32-unknown-unknown && cd ..
```

### 2. Configure the Manifest
The example agent configuration is located in `examples/agent.nano`:

```yaml
name: "nanos-agent"
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
    - "/workspace/**"
  fs_write:
    - "secret.txt"
    - "/workspace/**"
  network: false
tools:
  - "fs_read"
  - "fs_write"
goal: "Read the file instruction.txt, find the secret code inside it, and write ONLY the secret code into a new file called secret.txt. Then call done."
```

Create a dummy input file:
```bash
echo "The secret code is: silicon-rules-123" > instruction.txt
```

### 3. Run the Agent
Execute the agent inside the sandboxed runtime:

```bash
./target/release/nanos run examples/agent.nano
```

Upon run, the engine will boot the sandbox, map model weights, load the compiled Rust agent binary (`nanos-core-agent/target/wasm32-unknown-unknown/debug/nanos_core_agent.wasm`), and safely execute it. The output will be written to `secret.txt`.

### 4. Open the Interactive Dashboard & Debugger
If you want to view real-time fleet orchestration or play with the Time-Travel Debugger, launch the dashboard:

```bash
./target/release/nanos dashboard examples/fleet.nano
```

Once execution finishes, choose a step index from the trace history to inject a mocked observation (e.g. mock a tool failure) and spawn a divergent execution replay!

### 5. Write, Compile, and Run a JS/TS Agent (Optional)
You can write your agents in TypeScript/JavaScript using our SDK. 

Compile the provided `examples/test_agent.ts` script into an encapsulated WASM dynamic bundle:
```bash
node nanos-sdk/bin/nanos-compile.js examples/test_agent.ts --out dist/test_agent.wasm --engine bundle
```

Define the JS-based agent manifest (`examples/agent_js.nano`):
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

Then run it:
```bash
./target/release/nanos run examples/agent_js.nano
```

---

## 🆚 Why Not [X]?

| Feature | `nanos` ⚡ | E2B | LangChain | Docker + Python |
| :--- | :--- | :--- | :--- | :--- |
| **Cold Start** | **< 50ms** | ~2s | ~3s | ~30s |
| **RAM Overhead**| **< 15MB** | ~200MB | ~500MB | ~450MB |
| **Sandbox** | **WASM isolation** | Cloud VM container | None | Host container |
| **GPU Access** | **Direct Metal / CPU** | ❌ None | ❌ None | Manual setup |
| **Air-Gapped** | **✅ Yes** | ❌ No (Cloud only) | ❌ No | Partial |
| **Binary Size** | **Single ~15MB binary** | N/A | `pip install` | `docker pull` |

---

<div align="center">
  <b>nanos</b> — the agent doesn't need a cloud. it needs silicon.<br><br>
  <i>If you find this project valuable, please consider giving it a ⭐ on GitHub!</i>
</div>
