# Nanos Developer SDK (`nanos-sdk`)

The official software development kit and compilation toolchain to target `nanos`: the AI-Native WASM Agent OS.

With `nanos-sdk`, you can write your agent logic in **TypeScript** or **JavaScript**, leverage native host-bindings (syscalls), compile them into high-efficiency WebAssembly binaries with a single command, and deploy them on edge devices or air-gapped GPU servers.

---

## Installation

```bash
npm install -g nanos-sdk
```

---

## Writing an Agent in JavaScript

Create an `agent.js` file using standard ES modules and our FFI bindings:

```javascript
import { fs, llm, agent } from 'nanos-sdk';

export async function run() {
  // Fetch goal dynamically from the host process
  const goal = await agent.getGoal();
  console.log("Goal received:", goal);

  // Read raw instructions using standard fs read syscall (zero latency)
  const instructions = await fs.readFile('instruction.txt');
  
  // Call in-memory GPU LLM reasoning FFI
  const prompt = `System: You are an agent. Solve: ${goal}. Inputs: ${instructions}`;
  const response = await llm.infer(prompt);

  console.log("LLM thinking:", response);

  // Write summary back
  await fs.writeFile('secret.txt', response);

  // Signal clean exit
  await agent.done("Successfully replayed.");
}
```

---

## Compiling to Sandboxed WASM

To deploy your JavaScript/TypeScript agent, compile it into an optimized `.wasm` core image:

```bash
nanos-compile agent.js --out agent.wasm
```

This compiles your script, bundles our lightweight dynamic Javascript interpreter engine, optimizes tree-shaking using binaryen, and produces a single, isolated `< 2MB` WebAssembly binary.

---

## Running inside Nanos OS

Deploy using standard `agent.nano`:

```yaml
name: "my-compiled-agent"
model:
  provider: "ollama"
  model_name: "qwen2.5-coder:0.5b"
permissions:
  fs_read:
    - "instruction.txt"
  fs_write:
    - "secret.txt"
```

Then boot the sandbox instantly:

```bash
nanos run agent.nano
```
