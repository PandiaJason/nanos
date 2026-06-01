# nanos Benchmarks & Reproduction Methodology

This directory details the exact methodology, hardware configuration, and measurements used to establish the benchmark comparisons in the `nanos` project.

---

## 💻 Benchmark Environment

*   **Machine**: Apple M1 Pro (8 CPU Cores, 14 GPU Cores)
*   **Memory**: 16 GB Unified Memory (UMA)
*   **Operating System**: macOS Ventura / Sonoma (Darwin arm64)
*   **Model Under Test**: `qwen2.5-coder:0.5b` (GGUF Q4_K_M, 397 MB weights)
*   **Ollama Version**: `0.1.48`

---

## 📈 Metric Explanations & Methodology

### 1. WASM Sandbox Boot Time (`< 3ms` vs `~7,500ms` Docker VM)
*   **How it was measured**: 
    *   **nanos WASM Boot**: Measured the elapsed time from loading the pre-compiled guest agent WASM bytes to the invocation of the first guest system call (FFI `console.log` or file check).
    *   **Docker VM Boot**: Measured the time elapsed from invoking `docker run` until the containerized process responded to its first HTTP/API check.
*   **💡 HN Pre-emption (Warm vs. Cold Engine)**:
    *   **Warm Engine (Instance Instantiation)**: **`< 3ms`** (often `1.2ms - 2.5ms`). This represents the time required to create a new `wasmtime::Store` and instantiate the `wasmtime::Instance` from the pre-loaded module.
    *   **Cold Engine (Engine Creation)**: **`~18ms`**. This includes calling `wasmtime::Engine::new()` with compilation settings. In production, `nanos` initializes the `Engine` once on host boot (as a long-lived static singleton) and instantiates sandbox instances within it, so the effective runtime agent boot latency is indeed `<3ms`.

### 2. Model Load/Warmup Duration (`112ms` vs `1,137ms` Docker)
*   **How it was measured**: Measured the `load_duration` field returned in the JSON payload of Ollama's `/api/generate` endpoint.
*   **The Difference**:
    *   **Native Host (Metal)**: **`112ms`**. Uses standard `llama.cpp` file memory-mapping (`mmap`) directly to macOS Unified Memory (UMA), offloading all 24 layers of the model directly to the Apple GPU.
    *   **Docker Container (CPU)**: **`1,137ms`**. Because Docker on Mac runs inside a Linux virtual machine hypervisor, direct file mapping to native UMA is blocked by the hypervisor. The model weights must be copied through the VM disk-translation layer into virtual RAM and parsed entirely on the CPU.

### 3. Inference & Generation Throughput (`154.5 tok/sec` vs `17.5 tok/sec`)
*   **How it was measured**: Extracted the generated token count (`eval_count`) and token generation duration (`eval_duration`) from the Ollama API response:
    $$\text{Throughput} = \frac{\text{eval\_count}}{\text{eval\_duration (seconds)}}$$
*   **The Difference**:
    *   **Native Host (Metal)**: **`154.54 tokens/sec`**. Uses Apple Silicon unified memory cores and GPU/Neural Engine.
    *   **Docker Container (CPU)**: **`17.48 tokens/sec`**. Forced to use CPU-only vector calculations via the hypervisor.

---

## 🛠️ Step-by-Step Reproduction

### 1. Host LLM Preparation
Ensure Ollama is running natively on your macOS host:
```bash
# Pull the target model
ollama pull qwen2.5-coder:0.5b
```

### 2. Docker LLM Preparation
Start the test Docker container mapping port `11435` to avoid port collisions with the host:
```bash
# Spin up Docker Ollama container
docker run -d -p 11435:11434 --name ollama-docker-test ollama/ollama:latest

# Pull the model inside the Docker container
docker exec ollama-docker-test ollama pull qwen2.5-coder:0.5b
```

### 3. Run the Automated Benchmark
Execute the comparison Python script included in this directory to query both endpoints and print the performance stats:
```bash
python3 docker_vs_host.py
```

### 4. Clean Up
Tear down the test container:
```bash
docker stop ollama-docker-test && docker rm ollama-docker-test
```
