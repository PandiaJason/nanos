<div align="center">
  <img src="assets/nanos_logo.png" alt="nanos Logo" width="180">
  <h1>⚡ nanos</h1>
  <p><b>The lightweight, secure, and ultra-fast WebAssembly micro-runtime for sandboxed AI agents.</b></p>

  <p>
    <a href="https://github.com/PandiaJason/nanos/actions"><img src="https://github.com/PandiaJason/nanos/actions/workflows/ci.yml/badge.svg" alt="Build Status"></a>
    <a href="https://webassembly.org/"><img src="https://img.shields.io/badge/runtime-WASM-blueviolet?style=for-the-badge&logo=webassembly" alt="WASM"></a>
    <img src="https://img.shields.io/badge/sandbox-hardware--isolated-00cc88?style=for-the-badge" alt="Sandboxed">
    <img src="https://img.shields.io/badge/GPU-Metal%20%2F%20CUDA-ff6b6b?style=for-the-badge" alt="GPU">
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=for-the-badge" alt="License"></a>
  </p>

  <h3>📉 50x RAM Reduction (~39MB RSS vs 2GB+ VM) · ⚡ &lt; 3ms Sandbox Boot · Zero Docker · Zero Python</h3>
  <p><b>Just the agent, the weights, and the silicon. Serving WASM-sandboxed agents via CLI, HTTP API, TUI, or Web Debugger.</b></p>
</div>

---

## 💡 What is nanos?

**nanos** is a Rust-native, WebAssembly-powered micro-runtime for AI agents. By executing compiled agent binaries inside a hardware-isolated WebAssembly sandbox (Wasmtime), it cuts the typical runtime RAM footprint from **2GB+ (Docker Desktop VM on macOS) to ~39MB**, while booting the VM in **< 3ms**. 

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

## ✨ Features

*   **🔐 Hardware-Isolated WASM Sandbox**: Every agent runs inside a strict, metered `wasmtime` store with WASM linear memory isolation, fuel limits to prevent infinite loops, and strict memory caps.
*   **🎮 Native Metal & CUDA GPU Offload**: Model weights are memory-mapped directly onto Apple Metal or Linux CUDA graphics hardware via native `llama.cpp` layers (`--features gpu-cuda`).
*   **🤖 Multi-Agent Fleet Orchestration**: Orchestrate cooperative multi-agent fleets concurrently sharing a single `LlmEngine` locally via threads or across networks using distributed TCP message bus client/server connections.
*   **🔌 Universal MCP Tool Proxy**: Bridge standard Model Context Protocol (MCP) servers straight to WASM. Query tools, discover resources, pull prompts, and validate schemas dynamically.
*   **🕰️ Time-Travel Visual Web Debugger**: Inspect step execution traces, RAM consumption, tokens, and FFI latency. Click to edit observations or prompt variables, and launch divergent replays.
*   **🛡️ Sandboxed JS/TS SDK Runtime**: Write agents in TypeScript/JavaScript, compile them into WASM dynamic bundles via `nanos-compile.js`, and execute them safely with dynamic host permission rules.

---

## 🏗️ Architecture: The Microkernel Paradigm

`nanos` achieves its unique combination of sandbox isolation and native GPU speed by using a **Microkernel-inspired architecture**. Instead of virtualizing the host hardware (like a VM or container), `nanos` virtualizes only the agent's application code space using WebAssembly (WASM).

<p align="center">
  <img src="assets/nanos_architecture_detail.png" alt="nanos Architecture" width="750">
</p>

### 1. Separation of Concerns: Guest vs. Host
The runtime is split into two strictly separated execution spaces:
*   **User Space (Guest Sandbox)**: This is where the agent logic runs. Guest code is compiled to WebAssembly (JS/TS agents compile along with an optimized QuickJS virtual machine into a single `.wasm` binary). The sandbox has **zero native access** to files, network, or hardware.
*   **Kernel Space (Host Runtime)**: The native Rust engine. It compiles natively for your specific processor architecture (Apple Silicon ARM64, Linux x86_64, etc.) and has direct access to **Apple Metal APIs**, **Linux CUDA drivers**, and local filesystem/network resources.

### 2. In-Process Syscall Loop (FFI Memory Boundary)
In traditional agent stacks, tool execution requires local TCP loops, loopback routing, and HTTP JSON serialization. `nanos` treats tool calls like Operating System **syscalls**:
1.  **Shared Memory**: The host allocates a linear segment of RAM for the WASM guest sandbox. Since they share the same physical address space, the host reads and writes directly to the guest's sandbox memory.
2.  **Pointer Passing**: When the agent calls a tool like `fs.readFile("data.txt")`, the WASM guest writes the path into its linear memory and executes an FFI syscall (`nanos_fs_read(ptr, len)`).
3.  **Instant Execution**: The host intercepts the syscall, reads the arguments directly from the sandbox memory offset, validates the manifest permissions, executes the tool natively, writes the result back to WASM memory, and resumes guest execution. 
4.  **Zero-Copy Speed**: This whole process completes in **microseconds (< 1ms)** because no network sockets are opened and no JSON serialization occurs.

### 3. Native GPU Inference Bridge
Instead of compiling the matrix math of heavy LLM runtimes into WASM (which adds compiler layers and degrades performance), `nanos` keeps the inference engine native to the host:
1.  When the agent writes `llm.infer("...")`, the WASM guest triggers an FFI syscall: `llm_infer(prompt_ptr, prompt_len)`.
2.  The Rust host reads the prompt from the shared WASM memory segment.
3.  The host passes the prompt to its native `LlmEngine` (linked directly to the host's Apple Metal or CUDA drivers).
4.  The GPU executes the generation natively (**154 tokens/sec** on Metal for Qwen 0.5B) and streams the generated response directly back into the guest's memory.

---

## ⚡ Benchmarks

Here is the empirical proof of why the `nanos` architecture is a game-changer for agent deployment.

### 1. Runtime Overhead & Boot Latency
*Measured on Apple M1 Pro (macOS), qwen2.5-coder 1.5B, Metal GPU layers offloaded:*

| Metric / Stack | Docker Desktop VM + Python | **nanos (WASM + Host FFI)** | **Delta** | **How Verified** |
| :--- | :---: | :---: | :---: | :--- |
| **RAM Footprint** | ~2,000+ MB | **~39 MB** | 📉 **50x smaller** | Checked peak RSS via `ps` on host vs Docker Desktop minimum VM allocation. |
| **Cold Start** | ~7,500 ms | **< 3 ms** | 🚀 **2500x faster** | Measured sandbox configuring + boot time from instant of launch. |
| **Tool Execution** | ~348 ms | **< 1 ms** | ⚡ **300x faster** | WASM FFI syscall invocation (e.g. `fs_read`) vs Docker container routing. |

*Note: RAM footprint excludes loaded LLM weights, measuring only the container/runtime overhead. nanos has zero background daemon overhead.*

### 2. Local LLM Inference Performance (Metal GPU vs. Virtualized CPU)
To demonstrate why container-based agent platforms underperform on consumer hardware, we benchmarked the exact same `qwen2.5-coder:0.5b` model on the native Apple Silicon Host (representing `nanos`' native host FFI GPU offload pipeline) vs. a standard Docker container running via virtualized CPU:

| Metric | Docker Container (CPU-only VM) | Native Host (Metal GPU / Nanos) | Speedup / Impact |
| :--- | :---: | :---: | :---: |
| **Generation Throughput** | 17.48 tokens/sec | **154.54 tokens/sec** | 🚀 **8.84x faster** |
| **Prompt Evaluation Speed** | 70.62 tokens/sec | **1128.20 tokens/sec** | 🚀 **16.00x faster** |
| **Model Load / Warmup Time** | 1.137 seconds | **0.112 seconds** | 🚀 **10.15x faster** |
| **Hardware Offload** | None (Linux CPU Emulation) | **Apple Metal GPU / Unified Memory** | Native UMA speed |
| **Battery & Thermal Cost** | High (Virtual CPUs pegged at 100%) | **Extremely Low** (Metal offloaded) | 🔋 High power efficiency |

### 3. The macOS Local Agent Dilemma (Security vs. Performance)
For developers building and running AI agents locally on MacBooks (Apple Silicon M1/M2/M3/M4), executing agents has historically forced a compromised choice:

*   **🐢 The Docker Route (Secure but Slow)**: Running the agent inside a Docker container isolates the process but forces the LLM to run on CPU-only emulation (since Docker Desktop lacks Metal pass-through). This results in extremely slow speeds (**~17 tokens/sec**), high heat, loud fans, and rapid battery drain.
*   **⚠️ The Bare-Metal Route (Fast but Dangerous)**: Running the agent directly in your host macOS terminal grants full GPU Metal speeds (**~154 tokens/sec**), but provides zero isolation. A buggy or malicious command generated by the LLM can access your private ssh keys, steal documents, or corrupt your system.
*   **⚡ The nanos Route (Secure AND Fast)**: By running agent logic in a lightweight WASM sandbox and delegating LLM inference natively to the host's Metal/CUDA drivers, `nanos` delivers the security of a container with the raw hardware speed and power efficiency of native execution.

---

## 💻 CLI Command Reference

`nanos` is packaged as a single, compiled binary that manages everything from local runs to multi-agent fleets and network services.

```bash
# General usage structure
nanos <COMMAND> [OPTIONS]
```

### Subcommands

| Command | Description | Example Usage |
| :--- | :--- | :--- |
| **`run`** | Run a single AI agent from a `.nano` manifest | `nanos run examples/agent.nano` |
| **`serve`** | Serve the agent runtime background daemon and Visual Web Debugger over HTTP | `nanos serve --port 8080` |
| **`orchestrate`** | Orchestrate cooperative multi-agent fleets locally or as a TCP server | `nanos orchestrate examples/fleet.nano --network --port 9090` |
| **`node`** | Connect a remote fleet node client back to the distributed server orchestrator | `nanos node --connect 127.0.0.1:9090 --name writer` |
| **`dashboard`** | Launch the real-time TUI dashboard and Time-Travel debug console | `nanos dashboard examples/fleet.nano` |
| **`bench`** | Run a native FFI latency benchmark against the LLM model | `nanos bench examples/agent.nano` |

---

## 🚀 Quick Start

### 1. Prerequisites
Ensure you have the following installed on your host:
*   Rust & Cargo (MSRV 1.75+)
*   Node.js (v18+ for compiling, v20+ for the JS sandbox runner)
*   **Ollama** running locally. Pull the model before running:
    ```bash
    ollama pull qwen2.5-coder:1.5b
    ```

### 2. Build the Nanos Engine
Clone and compile the native runtime binary:
```bash
git clone https://github.com/PandiaJason/nanos && cd nanos
cargo build --release
```

### 3. Option A: Run a Rust Agent
Build the default Rust agent core into WebAssembly:
```bash
# Compile core agent to WASM target
cd nanos-core-agent && cargo build --target wasm32-unknown-unknown && cd ..

# Setup example file and execute
cp examples/instruction.txt .
./target/release/nanos run examples/agent.nano
```

---

### 4. Option B: Write, Compile & Run a JS/TS Agent

Use the custom compiler toolchain and TypeScript SDK (`nanos-sdk`) to bundle your TS scripts into secure WebAssembly.

#### Write the agent code (`examples/test_agent.ts`):
```typescript
import { fs, llm, agent } from '../nanos-sdk/index.js';

export async function run() {
  console.log("TS Agent started!");
  const goal = await agent.getGoal();
  
  const inputData = await fs.readFile("instruction.txt");
  const response = await llm.infer(`Summarize code: ${inputData}`);
  await fs.writeFile("secret.txt", response);
  
  await agent.done("TS FFI Loop completed successfully.");
}

run().catch(err => {
  console.error("TS Agent execution failed:", err);
  process.exit(1);
});
```

#### Compile and execute it:
```bash
# Compile TS to WASM
node nanos-sdk/bin/nanos-compile.js examples/test_agent.ts --out dist/test_agent.wasm --engine bundle

# Run under the sandbox manifest configuration
./target/release/nanos run examples/agent_js.nano
```

---

### 5. Launch the Visual Web Debugger
Expose `nanos` as an HTTP daemon and launch the premium visual dashboard companion:
```bash
./target/release/nanos serve --port 8080 --host 127.0.0.1
```
Open `http://localhost:8080` in your browser. Inspect running statuses, step latencies, peak memory consumption, and **click on any step to trigger a Time-Travel Divergent Replay**!
---

## 🛠️ Manifest Reference (`.nano`)

Every agent is defined by a `.nano` YAML configuration file:

```yaml
name: "nanos-js-agent"       # Name of the agent instance
model:
  provider: "ollama"         # LLM Provider: 'ollama' | 'openai' | 'local' (native GGUF)
  model_name: "qwen2.5-coder:0.5b" # Model name (for ollama/openai)
  path: "models/qwen.gguf"   # GGUF local model path (required for 'local' provider)
  context_window: 4096       # Context size limit
  api_url: "http://..."      # Custom API URL (optional)
  api_key: "sk-..."          # Custom API Key (optional)
resources:
  memory: "512MB"            # Sandbox physical RAM heap limit
  max_steps: 10              # Maximum FFI syscall loop iterations allowed
permissions:
  fs_read:                   # Whitelist of files or glob patterns the agent can read
    - "instruction.txt"
  fs_write:                  # Whitelist of files or glob patterns the agent can write
    - "secret.txt"
  network: false             # Disable or enable external TCP socket access
mcp_servers:                 # Whitelist of external Model Context Protocol stdio servers
  - name: "ping-server"
    command: "node"
    args:
      - "path/to/server.js"
tools:                       # List of tools permitted for the agent (e.g. fs_read, fs_write, mcp_call, done)
  - "fs_read"
  - "fs_write"
  - "mcp_call"
binary: "dist/test_agent.wasm" # Target agent compilation binary
goal: "Extract the secret..." # Mission statement of the agent
```

For the complete JSON-RPC FFI Protocol specification, see the [FFI Specification Document](docs/ffi-spec.md).

---

## 🆚 Architectural Comparison: nanos vs. LlamaEdge / WasmEdge

Unlike WebAssembly projects like **LlamaEdge** or **WasmEdge** which package the LLM itself into WASM to expose it as an HTTP web server, `nanos` focuses entirely on sandboxing the **agent logic** while letting inference run natively on host silicon.

| Dimension | **LlamaEdge / WasmEdge** 🌐 | **nanos** ⚡ |
| :--- | :--- | :--- |
| **Core Paradigm** | **"LLM-as-a-Service"** (Web Server Model) | **"Microkernel OS"** (In-Process Syscall Model) |
| **Interface Boundary** | Localhost HTTP REST Sockets (JSON-RPC) | Memory Boundary (Direct FFI Pointer Passing) |
| **Agent / LLM Relation** | Agent runs on the host, querying the LLM running inside WasmEdge over HTTP. | Agent runs inside the WASM sandbox, calling the host LLM via in-process FFI. |
| **Tool Execution Latency**| **~348ms** (TCP stack, serialization, loopback routing) | **< 1ms** (Zero-copy memory pointer sharing) |
| **Target Use Case** | Serving LLMs as isolated cloud web backends. | Executing local, secure, low-latency AI agents. |

### 🛠️ The Architectural Difference

1. **The Web Server Model (LlamaEdge)**:
   ```
   +------------+                  +------------------+                  +-------------+
   | Host Agent | --(HTTP/JSON)--> |  LlamaEdge WASM  | --(WASI-NN API)--> | host GPU/C+ |
   | (Py / JS)  | <-- (REST API) --|  (HTTP Server)   |                    +-------------+
   +------------+                  +------------------+
   ```
   Every step of the agent's action loop requires network translation, JSON parsing, and HTTP overhead.

2. **The Microkernel Syscall Model (nanos)**:
   ```
   +-------------------------------------------------------+
   |                     NANOS PROCESS                     |
   |                                                       |
   |  +---------------------+                              |
   |  |  WASM Agent Sandbox | (User Space Agent)            |
   |  +----------+----------+                              |
   |             |                                         |
   |             | In-Process FFI Pointer Pass (`llm_infer`)|
   |             v                                         |
   |  +---------------------+                              |
   |  |     Rust Host       | (Kernel Space Services)       |
   |  |  (Metal/CUDA/Tool)  |                              |
   |  +---------------------+                              |
   +-------------------------------------------------------+
   ```
   The agent logic is isolated in user space, but LLM inference and tool execution run in kernel space on native host bindings. The boundary is crossed in microseconds via direct pointer passing, completely bypassing loopback network stacks.

---

## 🆚 General Comparison Matrix

| Feature | `nanos` ⚡ | E2B | LangChain | Docker + Python |
| :--- | :--- | :--- | :--- | :--- |
| **Cold Start** | **< 3ms** | ~2s | ~3s | ~30s |
| **RAM Overhead**| **~39MB** | ~200MB | ~500MB | ~450MB |
| **Sandbox** | **WASM process-isolated** | Cloud VM container | None | Host container |
| **GPU Access** | **Direct Metal / CUDA** | ❌ None | ❌ None | Manual configuration |
| **Air-Gapped** | **✅ Yes** | ❌ No (Cloud only) | ❌ No | Partial |
| **Binary Size** | **Single ~23MB binary** | N/A | `pip install` | `docker pull` |

---

<div align="center">
  <b>nanos</b> — the agent doesn't need a cloud. it needs silicon.<br><br>
  <i>If you find this project valuable, please consider giving it a ⭐ on GitHub!</i>
</div>
