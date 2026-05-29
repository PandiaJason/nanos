# Nanos: The Zero-Latency AI Agent OS

**The era of heavy, slow, Python-based AI agents is over.**

`nanos` is a revolutionary, single-binary, zero-dependency bare-metal runtime for AI agents. It eliminates the fragile stack of Docker containers, Python interpreters, HTTP servers, and JSON parsers, replacing them with a secure, highly optimized WebAssembly (WASM) environment where the Agent, the Tools, and the LLM live in the exact same memory space.

## Why Nanos?
Every modern AI agent stack (LangChain, AutoGPT) suffers from compounding latency. Booting a container takes seconds; serializing JSON and pushing it over local HTTP to an MCP server or LLM adds hundreds of milliseconds of overhead per loop.

`nanos` throws out the entire stack:
1. **Insanely Fast:** Boots a sandboxed agent in under 50ms. Inference is executed via native FFI pointer passing directly to the GPU—zero HTTP overhead.
2. **Hardened Security:** Agents execute in a default-deny WASM sandbox. They cannot touch your filesystem or network unless explicitly authorized by the `agent.nano` manifest.
3. **Hyper-Mobile State:** Because the entire agent runs in WASM, you can take a byte-for-byte snapshot of a running agent mid-thought, serialize it to disk, and resume it on another machine instantly.

## The `agent.nano` Manifest
Agents are defined by a simple declarative manifest. You deploy this file, not a 2GB Docker image.

```yaml
model: models/tinyllama-1.1b-q4.gguf
goal: |
  Analyze the codebase and write a summary.

tools:
  - fs_read: /workspace/**
  - web_get: https://api.github.com/**

memory:
  ram: 512mb
  context_window: 8192
```

## CLI Usage

```bash
# Boot an autonomous agent using its manifest
nanos run agent.nano

# Run a quick, one-shot task
nanos ask --model tinyllama-q4 --tools fs_read "Summarize the log file"

# Check running processes
nanos ps
```

## Roadmap
- [x] WASM Sandboxing & FFI Syscall ABI
- [x] Native `llama.cpp` integration (Metal / CUDA offload)
- [x] Multi-step autonomous ReAct loop
- [x] Strict Security Rails (Manifest boundary enforcement)
- [x] MPSC Continuous Batching for multi-agent scheduling
- [ ] Fleet orchestration (`nanos fleet`)
- [ ] Cross-platform (Linux/x86 binaries)
- [ ] Persistent SQLite-backed memory store (`memory_store` / `memory_recall`)
