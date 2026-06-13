use anyhow::{Context, Result};
use llama_cpp_2::context::params::KvCacheType;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use tracing::info;

use std::sync::mpsc;
use std::thread;

pub struct LlmResponse {
    pub response: String,
    pub prompt_tokens: u32,
    pub gen_tokens: u32,
    pub prompt_eval_ms: f64,
    pub gen_ms: f64,
}

pub struct LlmRequest {
    pub prompt: String,
    pub reply: mpsc::Sender<LlmResponse>,
}

pub enum EngineBackend {
    Local {
        tx: mpsc::Sender<LlmRequest>,
    },
    Mlx {
        tx: mpsc::Sender<LlmRequest>,
    },
    Http {
        api_url: String,
        api_key: Option<String>,
        model_name: String,
    },
    Rust {
        transformer: std::sync::Arc<crate::rust_llama::Transformer>,
        tokenizer: std::sync::Arc<crate::rust_llama::Tokenizer>,
    },
}

pub struct LlmEngine {
    backend: EngineBackend,
}

impl LlmEngine {
    pub fn new(config: &crate::manifest::ModelConfig) -> Result<Self> {
        let provider = config.provider.as_deref().unwrap_or("local");

        match provider {
            "local" | "local-cpu" | "local-hybrid" => {
                let model_path = config.path.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("Local GGUF model path is required when provider is local")
                })?;
                let context_window = config.context_window;

                let (tx, rx) = mpsc::channel::<LlmRequest>();

                let path = model_path.clone();
                let is_cpu = provider == "local-cpu";
                let is_hybrid = provider == "local-hybrid";
                info!(
                    "Spawning dedicated LLM background thread for GGUF model: {}...",
                    path
                );
                thread::spawn(move || {
                    let backend = LlamaBackend::init().expect("Failed to initialize llama backend");

                    // Detect platform GPU capability and offload all layers if available
                    let gpu_layers: u32 = if is_cpu {
                        info!("local-cpu provider specified — running on CPU (0 layers offloaded)");
                        0
                    } else if is_hybrid {
                        info!("local-hybrid provider specified — running on both Metal and CPU (12 layers offloaded)");
                        12
                    } else if cfg!(target_os = "macos") {
                        info!("Detected macOS — enabling Metal GPU offload (all layers)");
                        99 // Offload all transformer layers to Apple Metal
                    } else if cfg!(target_os = "linux") {
                        info!("Detected Linux — attempting CUDA GPU offload (all layers)");
                        99 // Offload all layers to CUDA (falls back to CPU if no GPU)
                    } else {
                        info!("No GPU acceleration available — running on CPU only");
                        0
                    };

                    let model_params = LlamaModelParams::default().with_n_gpu_layers(gpu_layers);
                    let model = LlamaModel::load_from_file(&backend, &path, &model_params)
                        .expect("Failed to load model in background thread");

                    info!(
                        "Background thread: Model loaded with {} GPU layers offloaded.",
                        gpu_layers
                    );

                    // ── OPTIMIZATION: Create context ONCE and reuse across requests ──
                    // This avoids per-request Metal pipeline compilation (~25ms),
                    // compute buffer allocation (298 MiB), and graph reservation.
                    let default_threads = if gpu_layers > 0 { 2 } else { 4 };
                    let n_cpu_threads = std::env::var("NANOS_THREADS")
                        .ok()
                        .and_then(|s| s.parse::<i32>().ok())
                        .unwrap_or(default_threads);
                    let mut ctx_params =
                        llama_cpp_2::context::params::LlamaContextParams::default();
                    if let Some(nz) = core::num::NonZeroU32::new(context_window) {
                        ctx_params = ctx_params.with_n_ctx(Some(nz));
                        ctx_params = ctx_params.with_n_batch(context_window);
                    }
                    // Force flash attention ON (not auto) for maximum Metal throughput
                    // LLAMA_FLASH_ATTN_TYPE_ENABLED = 1
                    ctx_params = ctx_params.with_flash_attention_policy(1);
                    // Optimal CPU threading for Apple Silicon
                    ctx_params = ctx_params.with_n_threads(n_cpu_threads);
                    ctx_params = ctx_params.with_n_threads_batch(n_cpu_threads);
                    // Disable internal perf timing collection to reduce overhead
                    ctx_params = ctx_params.with_no_perf(true);
                    // Keep KV cache as F16 — quantization overhead hurts small models
                    ctx_params = ctx_params.with_type_k(KvCacheType::F16);
                    ctx_params = ctx_params.with_type_v(KvCacheType::F16);

                    let mut ctx = model
                        .new_context(&backend, ctx_params)
                        .expect("Failed to create persistent context");

                    // Pre-allocate batch for reuse
                    let mut batch =
                        llama_cpp_2::llama_batch::LlamaBatch::new(context_window as usize, 1);

                    info!(
                        "Background thread: Persistent context created (threads={}, flash_attn=forced, kv=F16).",
                        n_cpu_threads
                    );

                    for req in rx {
                        info!("Background thread: LLM received native prompt from WASM queue.");

                        // ── OPTIMIZATION: Clear KV cache instead of recreating context ──
                        ctx.clear_kv_cache();

                        let all_tokens = model
                            .str_to_token(&req.prompt, llama_cpp_2::model::AddBos::Always)
                            .expect("Failed to tokenize");

                        // Reserve space for generation; truncate prompt if it exceeds budget
                        let max_gen_tokens: usize = 256;
                        let max_prompt_tokens =
                            (context_window as usize).saturating_sub(max_gen_tokens);
                        let tokens = if all_tokens.len() > max_prompt_tokens {
                            info!(
                                "Truncating prompt from {} to {} tokens",
                                all_tokens.len(),
                                max_prompt_tokens
                            );
                            &all_tokens[all_tokens.len() - max_prompt_tokens..]
                        } else {
                            &all_tokens[..]
                        };

                        // ── OPTIMIZATION: Reuse batch, just clear it ──
                        batch.clear();

                        let last_index = (tokens.len() - 1) as i32;
                        for (i, token) in (0_i32..).zip(tokens.iter()) {
                            let is_last = i == last_index;
                            batch
                                .add(*token, i, &[0], is_last)
                                .expect("Failed to add token to batch");
                        }

                        let prompt_start = std::time::Instant::now();
                        ctx.decode(&mut batch).expect("Failed to decode prompt batch");
                        let prompt_eval_ms = prompt_start.elapsed().as_secs_f64() * 1000.0;

                        let mut response = String::with_capacity(max_gen_tokens * 8);
                        let mut n_cur = batch.n_tokens();
                        let mut generated_tokens: usize = 0;
                        let gen_start = std::time::Instant::now();

                        // ── GENERATION LOOP: Maximum throughput path ──
                        while generated_tokens < max_gen_tokens {
                            // Direct raw logits access — bypasses LlamaTokenData iterator
                            // get_logits_ith returns &[f32] of length n_vocab (raw pointer)
                            let logits = ctx.get_logits_ith(batch.n_tokens() - 1);
                            let best_idx = argmax(logits);
                            let best_logit = logits[best_idx];
                            let best_id = llama_cpp_2::token::LlamaToken::new(best_idx as i32);

                            // NaN safety: if best_logit is NaN, all logits are corrupt
                            if best_logit.is_nan() {
                                info!("NaN logit detected — aborting generation");
                                break;
                            }

                            if best_id == model.token_eos() {
                                break;
                            }

                            // Detokenize
                            let mut buf_size = 32;
                            let piece = loop {
                                match model.token_to_piece_bytes(best_id, buf_size, true, None) {
                                    Ok(bytes) => {
                                        break String::from_utf8_lossy(&bytes).into_owned();
                                    }
                                    Err(
                                        llama_cpp_2::TokenToStringError::InsufficientBufferSpace(
                                            needed,
                                        ),
                                    ) => {
                                        buf_size = (-needed) as usize;
                                    }
                                    Err(_) => {
                                        break String::new();
                                    }
                                }
                            };
                            response.push_str(&piece);
                            // Stream to stdout — single write, no flush per token
                            print!("{}", piece);

                            batch.clear();
                            batch
                                .add(best_id, n_cur, &[0], true)
                                .expect("Failed to add generated token");
                            ctx.decode(&mut batch)
                                .expect("Failed to decode generated token");

                            n_cur += 1;
                            generated_tokens += 1;
                        }
                        // Flush once at the end of generation
                        println!();

                        let gen_ms = gen_start.elapsed().as_secs_f64() * 1000.0;

                        let _ = req.reply.send(LlmResponse {
                            response,
                            prompt_tokens: tokens.len() as u32,
                            gen_tokens: generated_tokens as u32,
                            prompt_eval_ms,
                            gen_ms,
                        });
                    }
                });

                Ok(Self {
                    backend: EngineBackend::Local { tx },
                })
            }
            "mlx" => {
                // ── MLX Backend: Apple Silicon native JIT-fused Metal inference ──
                // Spawns a persistent Python process that loads the model once and
                // remains active to serve subsequent inferences instantly via JSON over stdio.
                let model_repo = config.model_name.clone().unwrap_or_else(|| {
                    "mlx-community/Qwen2.5-Coder-0.5B-Instruct-4bit".to_string()
                });
                info!("MLX backend: starting persistent daemon for repo {}", model_repo);

                let python_bin = if std::path::Path::new("/tmp/venv-mlx/bin/python").exists() {
                    "/tmp/venv-mlx/bin/python"
                } else {
                    "python3"
                };

                let python_script = r#"
import sys, json, time
try:
    from mlx_lm import load, stream_generate
    model_repo = sys.argv[1]
    model, tok = load(model_repo)
    print("READY", flush=True)
    
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        req = json.loads(line)
        messages = req["messages"]
        max_tokens = req.get("max_tokens", 256)
        
        prompt_str = tok.apply_chat_template(messages, tokenize=False, add_generation_prompt=True)
        prompt_tokens = tok.encode(prompt_str)
        prompt_start = time.time()
        
        tokens_out = []
        first_token_time = None
        gen_start = None
        for chunk in stream_generate(model, tok, prompt=prompt_str, max_tokens=max_tokens):
            if gen_start is None:
                gen_start = time.time()
                first_token_time = gen_start - prompt_start
            text_val = chunk.text if hasattr(chunk, "text") else chunk
            tokens_out.append(text_val)
            sys.stderr.write(text_val)
            sys.stderr.flush()
            
        gen_end = time.time()
        response_text = "".join(tokens_out)
        gen_tokens = len(tok.encode(response_text))
        gen_ms = (gen_end - gen_start) * 1000 if gen_start else 0
        prompt_eval_ms = first_token_time * 1000 if first_token_time else 0
        
        sys.stderr.write("\n")
        sys.stderr.flush()
        
        result = {
            "response": response_text,
            "prompt_tokens": len(prompt_tokens),
            "gen_tokens": gen_tokens,
            "prompt_eval_ms": prompt_eval_ms,
            "gen_ms": gen_ms
        }
        print(json.dumps(result), flush=True)
except Exception as e:
    print(json.dumps({"error": str(e)}), flush=True)
    sys.exit(1)
"#;

                let mut child = std::process::Command::new(python_bin)
                    .args(["-c", python_script, &model_repo])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::inherit())
                    .spawn()
                    .context("Failed to spawn MLX inference daemon")?;

                let stdout = child.stdout.take().context("Failed to open MLX daemon stdout")?;
                let stdin = child.stdin.take().context("Failed to open MLX daemon stdin")?;

                let mut reader = std::io::BufReader::new(stdout);
                let mut line = String::new();
                use std::io::BufRead;
                
                // Wait for the READY signal
                loop {
                    line.clear();
                    let n = reader.read_line(&mut line).context("Failed to read ready signal from MLX daemon")?;
                    if n == 0 {
                        return Err(anyhow::anyhow!("MLX daemon exited prematurely during startup"));
                    }
                    let trimmed = line.trim();
                    if trimmed == "READY" {
                        break;
                    }
                    if trimmed.starts_with('{') {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                                return Err(anyhow::anyhow!("MLX daemon startup failed: {}", err));
                            }
                        }
                    }
                }
                
                info!("MLX daemon is READY.");

                let (tx, rx) = mpsc::channel::<LlmRequest>();
                
                std::thread::spawn(move || {
                    let mut reader = reader;
                    let mut stdin = stdin;
                    let mut line = String::new();
                    
                    for req in rx {
                        let messages = parse_chatml_to_json_messages(&req.prompt);
                        let payload = serde_json::json!({
                            "messages": messages,
                            "max_tokens": 256
                        });
                        
                        let payload_str = match serde_json::to_string(&payload) {
                            Ok(s) => s,
                            Err(e) => {
                                let _ = req.reply.send(LlmResponse {
                                    response: format!("Failed to serialize request: {}", e),
                                    prompt_tokens: 0,
                                    gen_tokens: 0,
                                    prompt_eval_ms: 0.0,
                                    gen_ms: 0.0,
                                });
                                continue;
                            }
                        };
                        
                        use std::io::Write;
                        if let Err(e) = writeln!(stdin, "{}", payload_str).and_then(|_| stdin.flush()) {
                            let _ = req.reply.send(LlmResponse {
                                response: format!("Failed to write to MLX daemon: {}", e),
                                prompt_tokens: 0,
                                gen_tokens: 0,
                                prompt_eval_ms: 0.0,
                                gen_ms: 0.0,
                            });
                            continue;
                        }
                        
                        let mut response_received = false;
                        while !response_received {
                            line.clear();
                            match reader.read_line(&mut line) {
                                Ok(0) => {
                                    let _ = req.reply.send(LlmResponse {
                                        response: "MLX daemon connection lost".to_string(),
                                        prompt_tokens: 0,
                                        gen_tokens: 0,
                                        prompt_eval_ms: 0.0,
                                        gen_ms: 0.0,
                                    });
                                    return;
                                }
                                Ok(_) => {
                                    let trimmed = line.trim();
                                    if trimmed.starts_with('{') {
                                        if let Ok(result) = serde_json::from_str::<serde_json::Value>(trimmed) {
                                            if let Some(err) = result.get("error").and_then(|e| e.as_str()) {
                                                let _ = req.reply.send(LlmResponse {
                                                    response: format!("MLX error: {}", err),
                                                    prompt_tokens: 0,
                                                    gen_tokens: 0,
                                                    prompt_eval_ms: 0.0,
                                                    gen_ms: 0.0,
                                                });
                                            } else {
                                                let response_text = result["response"].as_str().unwrap_or("").to_string();
                                                let _ = req.reply.send(LlmResponse {
                                                    response: response_text,
                                                    prompt_tokens: result["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                                                    gen_tokens: result["gen_tokens"].as_u64().unwrap_or(0) as u32,
                                                    prompt_eval_ms: result["prompt_eval_ms"].as_f64().unwrap_or(0.0),
                                                    gen_ms: result["gen_ms"].as_f64().unwrap_or(0.0),
                                                });
                                            }
                                            response_received = true;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = req.reply.send(LlmResponse {
                                        response: format!("Failed to read from MLX daemon: {}", e),
                                        prompt_tokens: 0,
                                        gen_tokens: 0,
                                        prompt_eval_ms: 0.0,
                                        gen_ms: 0.0,
                                    });
                                    response_received = true;
                                }
                            }
                        }
                    }
                    
                    let _ = child.kill();
                });

                Ok(Self {
                    backend: EngineBackend::Mlx { tx },
                })
            }
            "rust" => {
                let model_path = config.path.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("Rust model path is required when provider is rust")
                })?;
                
                let model_path_buf = std::path::Path::new(model_path);
                let parent_dir = model_path_buf.parent().unwrap_or_else(|| std::path::Path::new("."));
                let tokenizer_path = parent_dir.join("tokenizer.bin");

                info!("Rust native engine: loading Llama transformer from {}...", model_path);
                let transformer = crate::rust_llama::Transformer::load(model_path)?;
                
                info!("Rust native engine: loading tokenizer from {}...", tokenizer_path.display());
                let tokenizer = crate::rust_llama::Tokenizer::load(&tokenizer_path, transformer.config.vocab_size)?;

                Ok(Self {
                    backend: EngineBackend::Rust {
                        transformer: std::sync::Arc::new(transformer),
                        tokenizer: std::sync::Arc::new(tokenizer),
                    },
                })
            }
            "openai" | "ollama" => {
                let api_url = if provider == "ollama" {
                    config
                        .api_url
                        .clone()
                        .unwrap_or_else(|| "http://localhost:11434/v1".to_string())
                } else {
                    config
                        .api_url
                        .clone()
                        .unwrap_or_else(|| "https://api.openai.com/v1".to_string())
                };

                let model_name = config.model_name.clone().unwrap_or_else(|| {
                    if provider == "ollama" {
                        "qwen2.5-coder:latest".to_string()
                    } else {
                        "gpt-4o-mini".to_string()
                    }
                });

                let api_key = config.api_key.clone();

                Ok(Self {
                    backend: EngineBackend::Http {
                        api_url,
                        api_key,
                        model_name,
                    },
                })
            }
            p => Err(anyhow::anyhow!("Unknown LLM provider: {}", p)),
        }
    }

    pub fn infer(&self, prompt: &str) -> Result<LlmResponse> {
        match &self.backend {
            EngineBackend::Local { tx } => {
                let (reply_tx, reply_rx) = mpsc::channel();
                tx.send(LlmRequest {
                    prompt: prompt.to_string(),
                    reply: reply_tx,
                })
                .context("Failed to send prompt to LLM thread")?;

                let response = reply_rx
                    .recv()
                    .context("Failed to receive response from LLM thread")?;
                Ok(response)
            }
            EngineBackend::Mlx { tx } => {
                let (reply_tx, reply_rx) = mpsc::channel();
                tx.send(LlmRequest {
                    prompt: prompt.to_string(),
                    reply: reply_tx,
                })
                .context("Failed to send prompt to MLX daemon thread")?;

                let response = reply_rx
                    .recv()
                    .context("Failed to receive response from MLX daemon thread")?;
                Ok(response)
            }
            EngineBackend::Rust { transformer, tokenizer } => {
                let start_prompt = std::time::Instant::now();
                let mut state = crate::rust_llama::RunState::new(&transformer.config);

                let mut tokens = Vec::new();
                tokens.push(1); // BOS token

                let mut remaining = prompt.to_string();
                if remaining.contains("<|im_start|>") {
                    let messages = parse_chatml_to_json_messages(prompt);
                    let mut cleaned = String::new();
                    if let Some(msg_array) = messages.as_array() {
                        for msg in msg_array {
                            if let Some(content) = msg["content"].as_str() {
                                cleaned.push_str(content);
                                cleaned.push(' ');
                            }
                        }
                    }
                    remaining = cleaned.trim().to_string();
                }

                // Greedy tokenization
                let max_token_len = tokenizer.max_token_length;
                let mut current_pos = 0;
                while current_pos < remaining.len() {
                     let mut best_token = None;
                     let mut best_len = 0;

                     for len in (1..=std::cmp::min(max_token_len, remaining.len() - current_pos)).rev() {
                         let substr = &remaining[current_pos .. current_pos + len];
                         if let Some(&pos) = tokenizer.vocab_map.get(substr) {
                             best_token = Some(pos);
                             best_len = len;
                             break;
                         }
                     }

                    if let Some(tok_id) = best_token {
                        tokens.push(tok_id);
                        current_pos += best_len;
                    } else {
                        current_pos += 1;
                    }
                }

                let prompt_eval_ms = start_prompt.elapsed().as_secs_f64() * 1000.0;
                
                let mut response = String::new();
                let start_gen = std::time::Instant::now();
                let mut gen_tokens = 0;

                let mut next_token = 1;
                let mut pos = 0;
                for &t in &tokens {
                    transformer.forward(t, pos, &mut state);
                    next_token = t;
                    pos += 1;
                }

                let max_gen_tokens = 128;
                let mut prev_token = next_token;
                
                for _ in 0..max_gen_tokens {
                    transformer.forward(next_token, pos, &mut state);
                    pos += 1;

                    let best_idx = argmax(&state.logits);

                    if best_idx == 2 || pos >= transformer.config.seq_len {
                        break;
                    }

                    let piece = tokenizer.decode(prev_token, best_idx);
                    print!("{}", piece);
                    std::io::Write::flush(&mut std::io::stdout()).unwrap();
                    response.push_str(&piece);
                    
                    prev_token = next_token;
                    next_token = best_idx;
                    gen_tokens += 1;
                }
                println!();

                let gen_ms = start_gen.elapsed().as_secs_f64() * 1000.0;

                Ok(LlmResponse {
                    response,
                    prompt_tokens: tokens.len() as u32,
                    gen_tokens,
                    prompt_eval_ms,
                    gen_ms,
                })
            }
            EngineBackend::Http {
                api_url,
                api_key,
                model_name,
            } => {
                info!(
                    "Sending API request to {} for model {}...",
                    api_url, model_name
                );
                let payload = serde_json::json!({
                    "model": model_name,
                    "messages": parse_chatml_to_json_messages(prompt),
                    "temperature": 0.0
                });

                let mut request = ureq::post(&format!("{}/chat/completions", api_url))
                    .set("Content-Type", "application/json");

                if let Some(key) = api_key {
                    request = request.set("Authorization", &format!("Bearer {}", key));
                }

                let res = request.send_json(payload)?;
                let res_json: serde_json::Value = res.into_json()?;

                let text = res_json["choices"][0]["message"]["content"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Invalid response format: {:?}", res_json))?
                    .to_string();

                let prompt_tokens = res_json["usage"]["prompt_tokens"]
                    .as_u64()
                    .unwrap_or(prompt.len() as u64 / 4) as u32;
                let completion_tokens = res_json["usage"]["completion_tokens"]
                    .as_u64()
                    .unwrap_or(text.len() as u64 / 4)
                    as u32;

                // Stream to stdout for parity with local GGUF
                println!("{}", text);

                Ok(LlmResponse {
                    response: text,
                    prompt_tokens,
                    gen_tokens: completion_tokens,
                    prompt_eval_ms: 0.0,
                    gen_ms: 0.0,
                })
            }
        }
    }
}

fn parse_chatml_to_json_messages(prompt: &str) -> serde_json::Value {
    let mut messages = Vec::new();
    let mut remaining = prompt;

    while let Some(start_idx) = remaining.find("<|im_start|>") {
        let content_start = start_idx + "<|im_start|>".len();
        if let Some(end_idx) = remaining[content_start..].find("<|im_end|>") {
            let block = &remaining[content_start..content_start + end_idx];
            if let Some(newline_idx) = block.find('\n') {
                let raw_role = block[..newline_idx].trim();
                let content = block[newline_idx + 1..].trim();

                let role = if raw_role.starts_with("system") {
                    "system"
                } else if raw_role.starts_with("user") {
                    "user"
                } else if raw_role.starts_with("assistant") {
                    "assistant"
                } else {
                    raw_role
                };

                messages.push(serde_json::json!({
                    "role": role,
                    "content": content
                }));
            }
            remaining = &remaining[content_start + end_idx + "<|im_end|>".len()..];
        } else {
            break;
        }
    }

    if messages.is_empty() {
        serde_json::json!([
            {
                "role": "user",
                "content": prompt
            }
        ])
    } else {
        serde_json::json!(messages)
    }
}

#[inline(always)]
fn argmax(slice: &[f32]) -> usize {
    let chunks = slice.chunks_exact(8);
    let remainder = chunks.remainder();
    
    let mut max_val = [f32::NEG_INFINITY; 8];
    let mut max_idx = [0usize; 8];
    
    for (chunk_idx, chunk) in chunks.enumerate() {
        let base_idx = chunk_idx * 8;
        for j in 0..8 {
            let val = chunk[j];
            if val > max_val[j] {
                max_val[j] = val;
                max_idx[j] = base_idx + j;
            }
        }
    }
    
    let mut best_idx = 0usize;
    let mut best_logit = f32::NEG_INFINITY;
    for j in 0..8 {
        if max_val[j] > best_logit {
            best_logit = max_val[j];
            best_idx = max_idx[j];
        }
    }
    
    let base_idx = slice.len() - remainder.len();
    for (j, &val) in remainder.iter().enumerate() {
        if val > best_logit {
            best_logit = val;
            best_idx = base_idx + j;
        }
    }
    
    best_idx
}
