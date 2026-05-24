use anyhow::{Context, Result};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use tracing::info;

use std::sync::mpsc;
use std::thread;

pub struct LlmRequest {
    pub prompt: String,
    pub reply: mpsc::Sender<String>,
}

pub struct LlmEngine {
    tx: mpsc::Sender<LlmRequest>,
}

impl LlmEngine {
    pub fn new(model_path: &str, context_window: u32) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<LlmRequest>();
        
        let path = model_path.to_string();
        info!("Spawning dedicated LLM background thread...");
        thread::spawn(move || {
            let backend = LlamaBackend::init().expect("Failed to initialize llama backend");
            let model_params = LlamaModelParams::default();
            let model = LlamaModel::load_from_file(&backend, &path, &model_params)
                .expect("Failed to load model in background thread");
                
            info!("Background thread: Model loaded successfully.");
            
            for req in rx {
                info!("Background thread: LLM received native prompt from WASM queue.");
                
                let mut ctx_params = llama_cpp_2::context::params::LlamaContextParams::default();
                if let Some(nz) = core::num::NonZeroU32::new(context_window) {
                    ctx_params = ctx_params.with_n_ctx(Some(nz));
                }
                
                let mut ctx = model.new_context(&backend, ctx_params)
                    .expect("Failed to create context");
                    
                let tokens = model.str_to_token(&req.prompt, llama_cpp_2::model::AddBos::Always)
                    .expect("Failed to tokenize");
                    
                let mut batch = llama_cpp_2::llama_batch::LlamaBatch::new(1024, 1);
                
                let last_index = (tokens.len() - 1) as i32;
                for (i, token) in (0_i32..).zip(tokens.iter()) {
                    let is_last = i == last_index;
                    batch.add(*token, i, &[0], is_last).expect("Failed to add token to batch");
                }
                
                ctx.decode(&mut batch).expect("Failed to decode prompt batch");
                
                let mut response = String::new();
                let mut n_cur = batch.n_tokens();
                let max_tokens = 128;
                let mut generated_tokens = 0;
                
                info!("Background thread: Generating response...");
                while generated_tokens < max_tokens {
                    let candidates = ctx.candidates_ith(batch.n_tokens() - 1);
                    let best_token = candidates
                        .max_by(|a, b| a.logit().partial_cmp(&b.logit()).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap()
                        .id();
                        
                    if best_token == model.token_eos() {
                        info!("Generated EOS token. Stopping.");
                        break;
                    }
                    
                    #[allow(deprecated)]
                    let piece = model.token_to_str(best_token, llama_cpp_2::model::Special::Tokenize).unwrap_or_default();
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
                
                let _ = req.reply.send(response);
            }
        });
        
        Ok(Self { tx })
    }
    
    pub fn infer(&self, prompt: &str) -> Result<String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx.send(LlmRequest {
            prompt: prompt.to_string(),
            reply: reply_tx,
        }).context("Failed to send prompt to LLM thread")?;
        
        let response = reply_rx.recv().context("Failed to receive response from LLM thread")?;
        Ok(response)
    }
}
