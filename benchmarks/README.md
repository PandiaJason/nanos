# nanos Benchmarks

This directory contains scripts to benchmark the latency of `nanos` (Native FFI) against traditional AI agent stacks (HTTP JSON REST APIs).

### The Hypothesis
Frameworks like LangChain, AutoGPT, and CrewAI suffer from massive overhead. Every step of their `Think -> Act -> Observe` loop requires:
1. Python building a prompt string
2. Serializing it to JSON
3. Opening a TCP socket
4. Executing an HTTP POST to Ollama / vLLM / OpenAI
5. The daemon parsing the JSON and queueing the request
6. Generating the response
7. Serializing the response to JSON
8. Returning it over HTTP to Python
9. Python parsing the JSON response

`nanos` eliminates all of this. The agent is a WASM sandboxed module that lives in the same memory space as the LLM Host. It simply executes an FFI Syscall (`llm_infer`), passing a raw memory pointer.

### How to Run

**1. Run the HTTP Baseline (Python + Ollama)**
Ensure you have Ollama installed and running the model locally:
```bash
ollama run tinyllama
```
Then run the Python script:
```bash
python3 http_bench.py
```

**2. Run the nanos FFI Benchmark (Rust)**
Run the benchmark subcommand natively via `nanos`:
```bash
cargo run --release -- bench ../agent.nano
```

### Expected Results
You should observe that `nanos` time-to-first-token and overhead-per-loop is consistently lower, often by orders of magnitude for the communication layer. This is why `nanos` is a fundamentally different paradigm for AI agents.
