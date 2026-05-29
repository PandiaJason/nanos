use anyhow::{Context, Result};
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
}

pub struct LlmRequest {
    pub prompt: String,
    pub reply: mpsc::Sender<LlmResponse>,
}

pub enum EngineBackend {
    Local {
        tx: mpsc::Sender<LlmRequest>,
    },
    Http {
        api_url: String,
        api_key: Option<String>,
        model_name: String,
    },
}

pub struct LlmEngine {
    backend: EngineBackend,
}

impl LlmEngine {
    pub fn new(config: &crate::manifest::ModelConfig) -> Result<Self> {
        let provider = config.provider.as_deref().unwrap_or("local");
        
        match provider {
            "local" => {
                let model_path = config.path.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Local GGUF model path is required when provider is local"))?;
                let context_window = config.context_window;
                
                let (tx, rx) = mpsc::channel::<LlmRequest>();
                
                let path = model_path.clone();
                info!("Spawning dedicated LLM background thread for GGUF model: {}...", path);
                thread::spawn(move || {
                    let backend = LlamaBackend::init().expect("Failed to initialize llama backend");
                    
                    // Detect platform GPU capability and offload all layers if available
                    let gpu_layers: u32 = if cfg!(target_os = "macos") {
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
                        
                    info!("Background thread: Model loaded with {} GPU layers offloaded.", gpu_layers);
                    
                    for req in rx {
                        info!("Background thread: LLM received native prompt from WASM queue.");
                        
                        let mut ctx_params = llama_cpp_2::context::params::LlamaContextParams::default();
                        if let Some(nz) = core::num::NonZeroU32::new(context_window) {
                            ctx_params = ctx_params.with_n_ctx(Some(nz));
                            ctx_params = ctx_params.with_n_batch(context_window);
                        }
                        
                        let mut ctx = model.new_context(&backend, ctx_params)
                            .expect("Failed to create context");
                            
                        let all_tokens = model.str_to_token(&req.prompt, llama_cpp_2::model::AddBos::Always)
                            .expect("Failed to tokenize");
                        
                        // Reserve space for generation; truncate prompt if it exceeds budget
                        let max_gen_tokens: usize = 256;
                        let max_prompt_tokens = (context_window as usize).saturating_sub(max_gen_tokens);
                        let tokens = if all_tokens.len() > max_prompt_tokens {
                            info!("Truncating prompt from {} to {} tokens", all_tokens.len(), max_prompt_tokens);
                            &all_tokens[all_tokens.len() - max_prompt_tokens..]
                        } else {
                            &all_tokens[..]
                        };
                        
                        let mut batch = llama_cpp_2::llama_batch::LlamaBatch::new(context_window as usize, 1);
                        
                        let last_index = (tokens.len() - 1) as i32;
                        for (i, token) in (0_i32..).zip(tokens.iter()) {
                            let is_last = i == last_index;
                            batch.add(*token, i, &[0], is_last).expect("Failed to add token to batch");
                        }
                        
                        info!("Prompt tokens: {:?}", &tokens[..std::cmp::min(20, tokens.len())]);
                        let decode_res = ctx.decode(&mut batch);
                        info!("Prompt decode result: {:?}", decode_res);
                        decode_res.expect("Failed to decode prompt batch");
                        
                        let mut response = String::new();
                        let mut n_cur = batch.n_tokens();
                        let mut generated_tokens: usize = 0;
                        
                        info!("Background thread: Generating response...");
                        while generated_tokens < max_gen_tokens {
                            let candidates_vec: Vec<_> = ctx.candidates_ith(batch.n_tokens() - 1).collect();
                            let has_nan = candidates_vec.iter().any(|c| c.logit().is_nan());
                            let min_logit = candidates_vec.iter().map(|c| c.logit()).fold(f32::INFINITY, f32::min);
                            let max_logit = candidates_vec.iter().map(|c| c.logit()).fold(f32::NEG_INFINITY, f32::max);
                            info!("Logits diagnostic: has_nan={}, min={}, max={}, count={}", has_nan, min_logit, max_logit, candidates_vec.len());

                            let best_token = candidates_vec.iter()
                                .max_by(|a, b| a.logit().partial_cmp(&b.logit()).unwrap_or(std::cmp::Ordering::Equal))
                                .unwrap()
                                .id();
                                
                            if best_token == model.token_eos() {
                                info!("Generated EOS token. Stopping.");
                                break;
                            }
                            
                            let mut buf_size = 32;
                            let piece = loop {
                                match model.token_to_piece_bytes(best_token, buf_size, true, None) {
                                    Ok(bytes) => {
                                        break String::from_utf8_lossy(&bytes).into_owned();
                                    }
                                    Err(llama_cpp_2::TokenToStringError::InsufficientBufferSpace(needed)) => {
                                        buf_size = (-needed) as usize;
                                    }
                                    Err(e) => {
                                        info!("token_to_piece_bytes error for token {:?}: {:?}", best_token, e);
                                        break String::new();
                                    }
                                }
                            };
                            response.push_str(&piece);
                            print!("{}", piece); // Stream to host stdout
                            std::io::Write::flush(&mut std::io::stdout()).unwrap();
                            
                            batch.clear();
                            batch.add(best_token, n_cur, &[0], true).expect("Failed to add generated token");
                            ctx.decode(&mut batch).expect("Failed to decode generated token");
                            
                            n_cur += 1;
                            generated_tokens += 1;
                        }
                        println!();
                        
                        let _ = req.reply.send(LlmResponse {
                            response,
                            prompt_tokens: tokens.len() as u32,
                            gen_tokens: generated_tokens as u32,
                        });
                    }
                });
                
                Ok(Self {
                    backend: EngineBackend::Local { tx },
                })
            }
            "openai" | "ollama" => {
                let api_url = if provider == "ollama" {
                    config.api_url.clone().unwrap_or_else(|| "http://localhost:11434/v1".to_string())
                } else {
                    config.api_url.clone().unwrap_or_else(|| "https://api.openai.com/v1".to_string())
                };
                
                let model_name = config.model_name.clone()
                    .unwrap_or_else(|| {
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
                    }
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
                }).context("Failed to send prompt to LLM thread")?;
                
                let response = reply_rx.recv().context("Failed to receive response from LLM thread")?;
                Ok(response)
            }
            EngineBackend::Http { api_url, api_key, model_name } => {
                info!("Sending API request to {} for model {}...", api_url, model_name);
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
                    
                let prompt_tokens = res_json["usage"]["prompt_tokens"].as_u64().unwrap_or(prompt.len() as u64 / 4) as u32;
                let completion_tokens = res_json["usage"]["completion_tokens"].as_u64().unwrap_or(text.len() as u64 / 4) as u32;
                
                // Stream to stdout for parity with local GGUF
                println!("{}", text);
                
                Ok(LlmResponse {
                    response: text,
                    prompt_tokens,
                    gen_tokens: completion_tokens,
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
