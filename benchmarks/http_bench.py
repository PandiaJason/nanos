import time
import requests
import json

# This script benchmarks the traditional HTTP JSON REST API approach (e.g. LangChain -> Ollama)
# Make sure Ollama is running locally with: `ollama run tinyllama`

PROMPT = "Read the file /etc/passwd and summarize it. If it fails, output done with 'Failed'."
SYSTEM = """You are an AI agent. When you want to execute a tool, you MUST output a raw JSON object and nothing else.
Allowed tools:
- fs_read: reads a file. Args: absolute path.
- web_get: fetches a URL. Args: the URL.
- done: finishes the task. Args: result summary.

Example output:
{"action": "fs_read", "args": "/workspace/report.txt"}
"""

def bench_http():
    print("Benchmarking HTTP to Ollama (tinyllama)...")
    
    payload = {
        "model": "tinyllama",
        "prompt": f"<|system|>\n{SYSTEM}\n<|user|>\n{PROMPT}\n<|assistant|>\n",
        "stream": False
    }

    start_time = time.perf_counter()
    
    try:
        response = requests.post("http://localhost:11434/api/generate", json=payload)
        response.raise_for_status()
        data = response.json()
        
        end_time = time.perf_counter()
        
        latency_ms = (end_time - start_time) * 1000
        print(f"✅ HTTP Inference completed in: {latency_ms:.2f} ms")
        print(f"Output: {data.get('response', '').strip()}")
        
    except Exception as e:
        print(f"❌ Failed to reach Ollama: {e}")
        print("Please ensure Ollama is running with `ollama run tinyllama`")

if __name__ == "__main__":
    bench_http()
