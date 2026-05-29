# nanos

A single-binary, zero-dependency AI agent runtime.

`nanos` replaces the entire Docker + Python + HTTP server + JSON API stack typically used to run AI agents. It is a bare-metal Rust runtime where the agent, the tools, and the LLM all live in the same memory space.

## The problem
Every modern AI agent stack looks like this:
Docker container (200MB+)
  → Python interpreter boot (~2s)
    → pip imports (langchain, transformers, openai...)
      → MCP tool server (separate HTTP process)
        → LLM API call (network, serialize, wait, parse)

Each arrow is latency, memory, and a failure surface. The agent does not control any of it.

## What nanos does instead
`nanos run agent.nano`
  → WASM sandbox boots (<50ms)
    → model mmap'd to GPU (Metal / CUDA)
      → fs_read: native syscall, zero network
        → llm_infer: pointer pass, in-memory
          → agent loop: think → act → observe → repeat
            → clean exit

No daemon. No interpreter. No HTTP. No serialization.

## Why this is fundamentally different

### The Status Quo
If you build an AI agent today using standard tools (LangChain, AutoGPT, MCP), the architecture is a massive, latency-heavy stack:
1. **The Container:** You boot a heavy Docker container (200MB+).
2. **The Runtime:** You boot a Python interpreter (takes ~1–2 seconds).
3. **The LLM Call:** The agent serializes its prompt into JSON, opens a network socket, makes a REST API call to OpenAI or vLLM, waits for the network, and parses the JSON back.
4. **The Tool Call:** When the agent wants to use a tool, it serializes another JSON payload, sends it over HTTP to an MCP server daemon, and waits for the response.

Every arrow is latency, memory overhead, and a failure point. The agent is just a script passing JSON back and forth over HTTP.

### The `nanos` Paradigm Shift
We threw out the entire stack. `nanos` is an "Operating System" for AI agents where the Agent, the Tools, and the LLM all live in the **exact same memory space**.
1. **The Kernel (Rust):** A violently fast, bare-metal Rust host replaces Python and Docker.
2. **The LLM (`llama.cpp`):** The LLM isn't behind an API. The Rust host memory-maps the model weights *directly* into your machine's GPU (Apple Metal M1) in the same process.
3. **The Agent Sandbox (WASM):** The agent's logic is compiled to WebAssembly (WASM), booting in <50ms inside a secure sandbox.
4. **Zero-Latency Syscalls:** When the agent uses a tool, it doesn't make an HTTP request. It executes a native FFI function call (a "Syscall") across the WASM boundary. It passes a raw memory pointer directly to the LLM on the GPU, or to the Rust host to read a file or fetch a URL. 

The result is a single-binary, zero-dependency runtime that cannot be stopped by network outages, requires no Docker orchestration, and executes ReAct loops faster than any Python-based framework on the market.

## The Benchmark: Nanos vs. HTTP
By eliminating Docker, Python, JSON serialization, and HTTP daemons (like Ollama or MCP), `nanos` radically reduces inference and tool-calling latency. 

We benchmarked a standard HTTP REST API request to Ollama against a `nanos` native FFI syscall using TinyLlama (1.1B) on an Apple M1 Pro (Metal). 

| Architecture | Cold Start Inference | Warm Inference (Cached) |
|--------------|----------------------|-------------------------|
| **Ollama (HTTP/JSON REST API)** | 29,562 ms | 1,166 ms |
| **Nanos (WASM FFI Syscall)**| **12,420 ms** | **992 ms** |

`nanos` cuts cold-start bootup time by over 50% and reduces warm inference latency by ~15% on localhost simply by removing the network/serialization tax.

## The Killer Feature: Universal Snapshotability
Because the entire agent runs inside a WebAssembly sandbox, its complete state—the stack, the heap, the instruction pointer, and the registers—is simply a flat, inspectable byte array. 

**You can pause a running agent mid-thought, serialize its exact memory state to disk, transmit it over the network to an edge device, and resume it seamlessly.** No existing Python or container-based framework can achieve this. You can start an agent on a massive cloud GPU to handle heavy reasoning, pause it, send its 2MB memory snapshot to a mobile phone, and let it resume execution on the edge.

## Architecture

### 1. Host engine (Rust)
The `nanos` binary is the OS kernel for agents. It owns memory, hardware, and the process boundary.
* **Manifest parser** — `serde_yaml` deserializes `agent.nano` into a strongly-typed `AgentManifest` struct defining model path, allowed tools, step budget, and memory limits.
* **Execution engine** — `wasmtime::Engine` instantiated without Component Model overhead, using raw core WASM for maximum performance.
* **State injection** — `AgentState` struct passed into `wasmtime::Store`, holding a mutable reference to the initialized LLM engine so syscalls can safely reach the neural weights from inside the sandbox.

### 2. Neural co-processor (llama-cpp-2)
`nanos` binds directly to the C++ inference engine via the `llama-cpp-2` crate, bypassing HTTP servers like Ollama or vLLM entirely.
* **Metal GPU offload** — on boot, `nanos` detects Apple Silicon and memory-maps GGUF weights directly to the GPU. TinyLlama 1.1B: 23/23 layers offloaded to MTL0, 636 MB resident.
* **Native generation loop** — a Rust autoregressive decoding loop allocates a `LlamaContext` (512+ tokens) and a `LlamaBatch`. The host tokenizes the prompt into `LlamaToken` IDs, pushes them into the compute graph (`ctx.decode`), and runs a greedy sampling loop (`candidates.max_by`) streaming tokens until EOS or max_tokens.
* **Zero Python** — no interpreter, no bindings layer, no subprocess. The weights answer directly.

### 3. Process boundary (WASM linear memory)
Agent logic is compiled to `wasm32-unknown-unknown` and loaded as an isolated module.
* **Memory isolation** — the agent has zero access to the host filesystem, network, or OS threading. The sandbox enforces this at the hardware instruction level, not by policy.
* **Zero-serialization data transfer** — traditional agents serialize prompts to JSON, send over TCP, wait for HTTP, parse the response. `nanos` transfers data by passing raw memory pointers (i32 offsets) across the WASM boundary. The prompt never becomes a string on the wire.

### 4. Tool ABI (native syscalls)
Host functions are mapped into the WASM sandbox via `linker.func_wrap`. These are syscalls — the agent's only interface to the outside world.
* `fs_read(path_ptr, path_len, out_buf, out_buf_len) -> i32`
The WASM module passes a pointer to a virtual file path. The host reads those bytes from WASM memory, performs the filesystem read, and writes the result directly into the guest's `out_buf`. No copies through userspace. No network hop.
* `llm_infer(prompt_ptr, prompt_len, out_buf, out_buf_len) -> i32`
The WASM module passes a pointer to its prompt. The host intercepts it, triggers the native `llama-cpp-2` evaluation graph on the GPU, and writes the generated string back into guest memory. First token in milliseconds.

Every syscall is declared in `agent.nano`. Anything not declared is denied at the boundary — the WASM module cannot call what the host has not registered.

### 5. Autonomous agent (WASM guest loop)
The agent (`nanos-core-agent`) is a lightweight Rust binary compiled to WASM. It runs the ReAct loop natively inside the sandbox.
* **ReAct state machine** — maintains a mutable `String` context buffer across steps.
* **Dispatch** — calls `llm_infer`, decodes the resulting UTF-8 bytes, scans for trigger strings like `ACTION: fs_read`.
* **Cognitive loop** — when an action is parsed, immediately triggers the corresponding syscall, appends the returned bytes under an `<|observation|>` tag, and loops back to inference. No external scheduler. No async runtime. Just a loop.

## The agent.nano manifest
```yaml
model: models/tinyllama-1.1b-q4.gguf
goal: |
  Read /workspace/report.txt and summarise it in three bullet points.

tools:
  - fs_read: /workspace/**
  - fs_write: /workspace/summary.md
  - shell: false

memory:
  ram: 512mb
  context_window: 512

limits:
  steps: 20
  wall_time: 60s

output: stdout
```
The manifest is the unit of deployment. It describes what the agent is allowed to do — not how to build an environment. You ship the manifest, not a container image.

## CLI
```bash
# run an agent
nanos run agent.nano

# ask a one-shot question (no manifest)
nanos ask --model tinyllama-q4 --tools fs_read "summarise report.txt"

# serve an MCP tool server
nanos mcp serve tools/github.nano --port 3000

# inspect running processes
nanos ps

# hard stop
nanos kill <pid>
```

## What this eliminates
| Traditional stack | nanos |
|-------------------|-------|
| Docker image (200MB+) | WASM binary (<1MB) |
| Python interpreter (~2s boot) | Compiled Rust, boots in <50ms |
| HTTP MCP server (separate process) | Native host functions |
| JSON serialization per tool call | Raw memory pointer pass |
| GPU config (manual) | Auto-detected, Metal/CUDA offload |
| Orphan processes on crash | Guaranteed clean exit |

## What the MVP proves
- [x] WASM sandbox boots and isolates agent code
- [x] 23/23 LLM layers offloaded to Metal GPU automatically
- [x] `fs_read` and `fs_write` work as native in-memory syscalls with path-prefix security
- [x] `llm_infer` crosses the WASM boundary with pointer semantics
- [x] Full ReAct step loop runs cleanly inside WASM
- [x] Multi-agent orchestration fleet with thread concurrency and blocking message queues
- [x] Real-time visual terminal dashboard and time-travel debugger console
- [x] Sandboxed dynamic JS interpreter FFI syscall (`eval_js`)

## Roadmap
- [x] Multi-step ReAct loop with N tool calls
- [x] `web_get` syscall (HTTP fetch, sandboxed to allowlist)
- [x] `memory_store` / `memory_recall` (sqlite native built-in memory store)
- [x] Multi-backend flexibility (local, openai, ollama)
- [x] Dynamic stdio-based MCP client integration proxy
- [x] Concurrent multi-agent orchestration fleet (`nanos orchestrate`)
- [x] Interactive Terminal Dashboard & Time-Travel Debugger (`nanos dashboard`)
- [x] JS Sandbox interpreter (`eval_js`) and compilation toolchain (`nanos-sdk`)
- [ ] `agent.nano` manifest enforces cgroup RAM limits
- [ ] Linux + x86 cross-platform (currently Mac / Metal)
- [ ] Larger models: 7B, 13B with quantization
- [ ] Rust embed API: `nanos_spawn()` for library use

---
Built in Rust. No Python. No Docker. No HTTP. Just the agent, the model, and the hardware.
