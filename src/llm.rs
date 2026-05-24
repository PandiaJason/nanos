use anyhow::{Context, Result};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use tracing::info;

pub struct LlmEngine {
    _backend: LlamaBackend,
    pub model: LlamaModel,
}

impl LlmEngine {
    pub fn new(model_path: &str) -> Result<Self> {
        info!("Initializing llama.cpp backend...");
        let backend = LlamaBackend::init().context("Failed to initialize llama backend")?;
        
        info!("Loading model weights from {} (mmap)...", model_path);
        let model_params = LlamaModelParams::default();
        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .with_context(|| format!("Failed to load model from {}", model_path))?;
            
        info!("Model loaded successfully.");
        
        Ok(Self {
            _backend: backend,
            model,
        })
    }
    
    pub fn infer(&mut self, prompt: &str) -> Result<String> {
        info!("LLM received native prompt from WASM: {}", prompt);
        
        let mut ctx = self.model.new_context(&self._backend, llama_cpp_2::context::params::LlamaContextParams::default())
            .context("Failed to create context")?;
            
        let tokens = self.model.str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .context("Failed to tokenize prompt")?;
            
        let mut batch = llama_cpp_2::llama_batch::LlamaBatch::new(1024, 1);
        
        // Add prompt tokens to batch
        let last_index = (tokens.len() - 1) as i32;
        for (i, token) in (0_i32..).zip(tokens.iter()) {
            let is_last = i == last_index;
            batch.add(*token, i, &[0], is_last).context("Failed to add token to batch")?;
        }
        
        ctx.decode(&mut batch).context("Failed to decode prompt batch")?;
        
        let mut response = String::new();
        let mut n_cur = batch.n_tokens();
        let max_tokens = 128; // Decided on 128
        let mut generated_tokens = 0;
        
        info!("Generating response...");
        while generated_tokens < max_tokens {
            let candidates = ctx.candidates_ith(batch.n_tokens() - 1);
            let best_token = candidates
                .max_by(|a, b| a.logit().partial_cmp(&b.logit()).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap()
                .id();
                
            if best_token == self.model.token_eos() {
                info!("Generated EOS token. Stopping.");
                break;
            }
            
            #[allow(deprecated)]
            let piece = self.model.token_to_str(best_token, llama_cpp_2::model::Special::Tokenize).unwrap_or_default();
            response.push_str(&piece);
            print!("{}", piece); // Stream to host stdout
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            
            batch.clear();
            batch.add(best_token, n_cur, &[0], true).context("Failed to add generated token to batch")?;
            ctx.decode(&mut batch).context("Failed to decode token batch")?;
            
            n_cur += 1;
            generated_tokens += 1;
        }
        println!();
        
        Ok(response)
    }
}
