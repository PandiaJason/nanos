import urllib.request
import json
import time

prompt = "Write a 100-word story about a microsecond runtime VM."
model = "qwen2.5-coder:0.5b"

def run_benchmark(url, name):
    print(f"--- Benchmarking {name} ({url}) ---")
    data = json.dumps({
        "model": model,
        "prompt": prompt,
        "stream": False
    }).encode('utf-8')
    
    req = urllib.request.Request(
        url, 
        data=data, 
        headers={'Content-Type': 'application/json'}
    )
    
    start_time = time.time()
    try:
        with urllib.request.urlopen(req, timeout=120) as response:
            res_data = response.read().decode('utf-8')
            elapsed = time.time() - start_time
            res_json = json.loads(res_data)
            
            # Extract statistics
            total_duration = res_json.get("total_duration", 0) / 1e9 # to seconds
            load_duration = res_json.get("load_duration", 0) / 1e9
            prompt_eval_duration = res_json.get("prompt_eval_duration", 0) / 1e9
            eval_duration = res_json.get("eval_duration", 0) / 1e9
            eval_count = res_json.get("eval_count", 0)
            prompt_eval_count = res_json.get("prompt_eval_count", 0)
            
            # Print details
            print(f"Status: Success")
            print(f"Wall time: {elapsed:.3f} s")
            print(f"Total duration (Ollama): {total_duration:.3f} s")
            print(f"Load duration: {load_duration:.3f} s")
            print(f"Prompt eval count: {prompt_eval_count} tokens")
            print(f"Prompt eval duration: {prompt_eval_duration:.3f} s")
            print(f"Eval count: {eval_count} tokens")
            print(f"Eval duration: {eval_duration:.3f} s")
            
            if eval_duration > 0:
                tok_per_sec = eval_count / eval_duration
                print(f"Generation throughput: {tok_per_sec:.2f} tokens/sec")
            else:
                print("Generation throughput: N/A (eval_duration is 0)")
            
            print(f"Response:\n{res_json.get('response', '')}\n")
            return res_json
    except Exception as e:
        print(f"Failed to query {name}: {e}")
        return None

print("Starting LLM benchmark comparison...")
host_res = run_benchmark("http://localhost:11434/api/generate", "Host (Native MacOS GPU Metal)")
print("\n" + "="*40 + "\n")
docker_res = run_benchmark("http://localhost:11435/api/generate", "Docker Container (CPU-only Hypervisor)")
