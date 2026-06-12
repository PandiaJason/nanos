use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub dim: usize,
    pub hidden_dim: usize,
    pub n_layers: usize,
    pub n_heads: usize,
    pub n_kv_heads: usize,
    pub vocab_size: usize,
    pub seq_len: usize,
    pub shared_weights: bool,
}

impl Config {
    pub fn head_size(&self) -> usize {
        self.dim / self.n_heads
    }
}

pub struct Weights {
    // token_embedding_table: [vocab_size, dim]
    pub token_embedding_table: Vec<f32>,
    // rms_att_weight: [n_layers, dim]
    pub rms_att_weight: Vec<f32>,
    // wq: [n_layers, dim, dim]
    pub wq: Vec<f32>,
    // wk: [n_layers, dim, dim_kv]
    pub wk: Vec<f32>,
    // wv: [n_layers, dim, dim_kv]
    pub wv: Vec<f32>,
    // wo: [n_layers, dim, dim]
    pub wo: Vec<f32>,
    // rms_ffn_weight: [n_layers, dim]
    pub rms_ffn_weight: Vec<f32>,
    // w1: [n_layers, hidden_dim, dim]
    pub w1: Vec<f32>,
    // w2: [n_layers, dim, hidden_dim]
    pub w2: Vec<f32>,
    // w3: [n_layers, hidden_dim, dim]
    pub w3: Vec<f32>,
    // rms_final_weight: [dim]
    pub rms_final_weight: Vec<f32>,
    // wcls: [vocab_size, dim] (optional)
    pub wcls: Option<Vec<f32>>,
}

pub struct RunState {
    pub x: Vec<f32>,      // activation at current time stamp [dim]
    pub xb: Vec<f32>,     // helper buffer for RMSNorm [dim]
    pub xb2: Vec<f32>,    // second helper buffer [dim]
    pub hb: Vec<f32>,     // activation in FFN [hidden_dim]
    pub hb2: Vec<f32>,    // second activation in FFN [hidden_dim]
    pub q: Vec<f32>,      // query [dim]
    pub k: Vec<f32>,      // key [dim]
    pub v: Vec<f32>,      // value [dim]
    pub att: Vec<f32>,    // attention scores [n_heads, seq_len]
    pub logits: Vec<f32>, // output logits [vocab_size]
    // KV Cache
    pub key_cache: Vec<f32>,   // [n_layers, seq_len, dim_kv]
    pub value_cache: Vec<f32>, // [n_layers, seq_len, dim_kv]
}

impl RunState {
    pub fn new(config: &Config) -> Self {
        let kv_dim = (config.dim * config.n_kv_heads) / config.n_heads;
        Self {
            x: vec![0.0; config.dim],
            xb: vec![0.0; config.dim],
            xb2: vec![0.0; config.dim],
            hb: vec![0.0; config.hidden_dim],
            hb2: vec![0.0; config.hidden_dim],
            q: vec![0.0; config.dim],
            k: vec![0.0; kv_dim],
            v: vec![0.0; kv_dim],
            att: vec![0.0; config.n_heads * config.seq_len],
            logits: vec![0.0; config.vocab_size],
            key_cache: vec![0.0; config.n_layers * config.seq_len * kv_dim],
            value_cache: vec![0.0; config.n_layers * config.seq_len * kv_dim],
        }
    }
}

pub struct Transformer {
    pub config: Config,
    pub weights: Weights,
    pub rope_cos: Vec<f32>,
    pub rope_sin: Vec<f32>,
}

impl Transformer {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path).context("Failed to open Llama model file")?;

        // Read Config
        let mut header = [0u8; 28];
        file.read_exact(&mut header).context("Failed to read header")?;
        
        let read_i32 = |offset: usize| -> i32 {
            let bytes = [header[offset], header[offset + 1], header[offset + 2], header[offset + 3]];
            i32::from_le_bytes(bytes)
        };

        let dim = read_i32(0) as usize;
        let hidden_dim = read_i32(4) as usize;
        let n_layers = read_i32(8) as usize;
        let n_heads = read_i32(12) as usize;
        let n_kv_heads = read_i32(16) as usize;
        let vocab_size_raw = read_i32(20);
        let vocab_size = vocab_size_raw.abs() as usize;
        let seq_len = read_i32(24) as usize;
        let shared_weights = vocab_size_raw > 0;

        let config = Config {
            dim,
            hidden_dim,
            n_layers,
            n_heads,
            n_kv_heads,
            vocab_size,
            seq_len,
            shared_weights,
        };

        // Determine if model has separate classifier weights
        let file_len = file.metadata()?.len();
        let kv_dim = (dim * n_kv_heads) / n_heads;

        // Calculate expected file size without wcls to check if shared
        let size_without_wcls = 28 + (
            vocab_size * dim + // token_embedding_table
            n_layers * dim + // rms_att_weight
            n_layers * dim * dim + // wq
            n_layers * dim * kv_dim + // wk
            n_layers * dim * kv_dim + // wv
            n_layers * dim * dim + // wo
            n_layers * dim + // rms_ffn_weight
            n_layers * hidden_dim * dim + // w1
            n_layers * dim * hidden_dim + // w2
            n_layers * hidden_dim * dim + // w3
            dim // rms_final_weight
        ) * 4;

        let has_wcls = !shared_weights && file_len > size_without_wcls as u64;

        // Helper to read float vector
        let read_floats = |f: &mut File, count: usize| -> Result<Vec<f32>> {
            let mut buf = vec![0u8; count * 4];
            f.read_exact(&mut buf).context("Failed to read floats")?;
            let mut floats = vec![0.0f32; count];
            for i in 0..count {
                let bytes = [buf[i*4], buf[i*4+1], buf[i*4+2], buf[i*4+3]];
                floats[i] = f32::from_le_bytes(bytes);
            }
            Ok(floats)
        };

        // Load weights
        let token_embedding_table = read_floats(&mut file, vocab_size * dim)?;
        let rms_att_weight = read_floats(&mut file, n_layers * dim)?;
        let wq = read_floats(&mut file, n_layers * dim * dim)?;
        let wk = read_floats(&mut file, n_layers * dim * kv_dim)?;
        let wv = read_floats(&mut file, n_layers * dim * kv_dim)?;
        let wo = read_floats(&mut file, n_layers * dim * dim)?;
        let rms_ffn_weight = read_floats(&mut file, n_layers * dim)?;
        let w1 = read_floats(&mut file, n_layers * hidden_dim * dim)?;
        let w2 = read_floats(&mut file, n_layers * dim * hidden_dim)?;
        let w3 = read_floats(&mut file, n_layers * hidden_dim * dim)?;
        let rms_final_weight = read_floats(&mut file, dim)?;

        // Skip freq_cis_real and freq_cis_imag (which are often in the file but not used by simple RoPE)
        let head_size = dim / n_heads;
        let freq_cis_size = seq_len * (head_size / 2);
        file.seek(SeekFrom::Current((freq_cis_size * 2 * 4) as i64))?;

        let wcls = if has_wcls {
            Some(read_floats(&mut file, vocab_size * dim)?)
        } else {
            None
        };

        let weights = Weights {
            token_embedding_table,
            rms_att_weight,
            wq,
            wk,
            wv,
            wo,
            rms_ffn_weight,
            w1,
            w2,
            w3,
            rms_final_weight,
            wcls,
        };

        // Precompute RoPE cos/sin tables for all positions and dimensions
        let head_size = dim / n_heads;
        let half_head = head_size / 2;
        let mut rope_cos = Vec::with_capacity(seq_len * half_head);
        let mut rope_sin = Vec::with_capacity(seq_len * half_head);
        for pos in 0..seq_len {
            for i in (0..head_size).step_by(2) {
                let val = (pos as f32) / 10000.0f32.powf((i as f32) / (head_size as f32));
                rope_cos.push(val.cos());
                rope_sin.push(val.sin());
            }
        }

        Ok(Self { config, weights, rope_cos, rope_sin })
    }

    pub fn forward(&self, token: usize, pos: usize, state: &mut RunState) {
        let cfg = &self.config;
        let w = &self.weights;
        let dim = cfg.dim;
        let kv_dim = (dim * cfg.n_kv_heads) / cfg.n_heads;
        let kv_mul = cfg.n_heads / cfg.n_kv_heads;
        let head_size = cfg.head_size();

        // 1. Copy embedding into x
        let content_row = &w.token_embedding_table[token * dim .. (token + 1) * dim];
        state.x.copy_from_slice(content_row);

        // 2. Loop over layers
        for l in 0..cfg.n_layers {
            // Apply RMSNorm to x -> xb
            rmsnorm(&mut state.xb, &state.x, &w.rms_att_weight[l * dim .. (l + 1) * dim]);

            // Query, Key, Value Projections (parallelized across Rayon worker threads)
            {
                let q = &mut state.q;
                let k = &mut state.k;
                let v = &mut state.v;
                let xb = &state.xb;
                let wq_slice = &w.wq[l * dim * dim .. (l + 1) * dim * dim];
                let wk_slice = &w.wk[l * dim * kv_dim .. (l + 1) * dim * kv_dim];
                let wv_slice = &w.wv[l * dim * kv_dim .. (l + 1) * dim * kv_dim];
                
                rayon::join(
                    || matmul(q, xb, wq_slice, dim, dim),
                    || rayon::join(
                        || matmul(k, xb, wk_slice, dim, kv_dim),
                        || matmul(v, xb, wv_slice, dim, kv_dim),
                    )
                );
            }

            // RoPE Rotary Position Embedding (retrieved from precomputed cache tables)
            let half_head = head_size / 2;
            let cos_offset = pos * half_head;
            let rope_cos = &self.rope_cos[cos_offset .. cos_offset + half_head];
            let rope_sin = &self.rope_sin[cos_offset .. cos_offset + half_head];

            for h in 0..cfg.n_heads {
                for i in (0..head_size).step_by(2) {
                    let f_cos = rope_cos[i / 2];
                    let f_sin = rope_sin[i / 2];

                    // Query RoPE
                    let q_idx = h * head_size + i;
                    let q0 = state.q[q_idx];
                    let q1 = state.q[q_idx + 1];
                    state.q[q_idx] = q0 * f_cos - q1 * f_sin;
                    state.q[q_idx + 1] = q0 * f_sin + q1 * f_cos;

                    // Key RoPE (if KV head index falls within range)
                    let kv_h = h / kv_mul;
                    let k_idx = kv_h * head_size + i;
                    if k_idx + 1 < kv_dim {
                        let k0 = state.k[k_idx];
                        let k1 = state.k[k_idx + 1];
                        state.k[k_idx] = k0 * f_cos - k1 * f_sin;
                        state.k[k_idx + 1] = k0 * f_sin + k1 * f_cos;
                    }
                }
            }

            // Save Key & Value in KV Cache
            let cache_offset = l * cfg.seq_len * kv_dim + pos * kv_dim;
            state.key_cache[cache_offset .. cache_offset + kv_dim].copy_from_slice(&state.k);
            state.value_cache[cache_offset .. cache_offset + kv_dim].copy_from_slice(&state.v);

            // Multi-Head Attention calculation
            let sqrt_head_size = (head_size as f32).sqrt();

            for h in 0..cfg.n_heads {
                let att_offset = h * cfg.seq_len;
                let q_offset = h * head_size;
                let kv_h = h / kv_mul;

                let q_slice = &state.q[q_offset .. q_offset + head_size];

                // Score query against key cache
                for t in 0..=pos {
                    let cache_key_offset = l * cfg.seq_len * kv_dim + t * kv_dim;
                    let k_offset = cache_key_offset + kv_h * head_size;

                    let mut score = 0.0f32;
                    let k_slice = &state.key_cache[k_offset .. k_offset + head_size];

                    for (&a, &b) in q_slice.iter().zip(k_slice.iter()) {
                        score += a * b;
                    }
                    score /= sqrt_head_size;
                    state.att[att_offset + t] = score;
                }

                // Softmax attention scores
                softmax(&mut state.att[att_offset .. att_offset + pos + 1]);

                // Zero out this head's portion in xb2 and compute weighted sum of values
                let xb_offset = h * head_size;
                let xb2_slice = &mut state.xb2[xb_offset .. xb_offset + head_size];
                for i in 0..head_size {
                    xb2_slice[i] = 0.0;
                }

                // Weighted sum of values -> xb2
                for t in 0..=pos {
                    let cache_val_offset = l * cfg.seq_len * kv_dim + t * kv_dim;
                    let v_offset = cache_val_offset + kv_h * head_size;
                    let a = state.att[att_offset + t];
                    let v_slice = &state.value_cache[v_offset .. v_offset + head_size];
                    for i in 0..head_size {
                        xb2_slice[i] += a * v_slice[i];
                    }
                }
            }

            // Output projection w.wo
            matmul(&mut state.xb, &state.xb2, &w.wo[l * dim * dim .. (l + 1) * dim * dim], dim, dim);

            // Residual connection: x += xb
            for i in 0..dim {
                state.x[i] += state.xb[i];
            }

            // ── Feed-Forward Network (FFN) ──
            // Apply RMSNorm to x -> xb
            rmsnorm(&mut state.xb, &state.x, &w.rms_ffn_weight[l * dim .. (l + 1) * dim]);

            // Gate projections w1 and w3 (parallelized across Rayon worker threads)
            {
                let hb = &mut state.hb;
                let hb2 = &mut state.hb2;
                let xb = &state.xb;
                let w1_slice = &w.w1[l * cfg.hidden_dim * dim .. (l + 1) * cfg.hidden_dim * dim];
                let w3_slice = &w.w3[l * cfg.hidden_dim * dim .. (l + 1) * cfg.hidden_dim * dim];

                rayon::join(
                    || matmul(hb, xb, w1_slice, dim, cfg.hidden_dim),
                    || matmul(hb2, xb, w3_slice, dim, cfg.hidden_dim),
                );
            }

            // SwiGLU activation: hb = hb * sigmoid(hb) * hb2
            for i in 0..cfg.hidden_dim {
                let val = state.hb[i];
                // silu
                let silu = val * (1.0 / (1.0 + (-val).exp()));
                state.hb[i] = silu * state.hb2[i];
            }

            // FFN output projection w2 -> xb
            matmul(&mut state.xb, &state.hb, &w.w2[l * dim * cfg.hidden_dim .. (l + 1) * dim * cfg.hidden_dim], cfg.hidden_dim, dim);

            // Residual connection: x += xb
            for i in 0..dim {
                state.x[i] += state.xb[i];
            }
        }

        // 3. Final RMSNorm
        rmsnorm(&mut state.xb, &state.x, &w.rms_final_weight);

        // 4. Output Classifier
        let w_classifier = match &w.wcls {
            Some(wcls_weights) => wcls_weights,
            None => &w.token_embedding_table,
        };
        matmul(&mut state.logits, &state.xb, w_classifier, dim, cfg.vocab_size);
    }
}

// ── Tensor Operations ──

fn rmsnorm(o: &mut [f32], x: &[f32], weight: &[f32]) {
    let size = x.len();
    let o = &mut o[..size];
    let weight = &weight[..size];
    let mut ss = 0.0f32;
    for i in 0..size {
        ss += x[i] * x[i];
    }
    ss /= size as f32;
    ss += 1e-5f32;
    let scale = 1.0f32 / ss.sqrt();
    for i in 0..size {
        o[i] = weight[i] * (scale * x[i]);
    }
}

fn softmax(x: &mut [f32]) {
    if x.is_empty() {
        return;
    }
    let mut max_val = x[0];
    for &val in x.iter().skip(1) {
        if val > max_val {
            max_val = val;
        }
    }
    let mut sum = 0.0f32;
    for val in x.iter_mut() {
        *val = (*val - max_val).exp();
        sum += *val;
    }
    for val in x.iter_mut() {
        *val /= sum;
    }
}

fn matmul(xout: &mut [f32], x: &[f32], w: &[f32], n: usize, _d: usize) {
    let threshold = 300_000;
    let size = xout.len();

    let dot_product = |w_row: &[f32]| -> f32 {
        let mut sum = 0.0f32;
        let len = x.len();
        assert_eq!(len, w_row.len());
        for i in 0..len {
            sum += x[i] * w_row[i];
        }
        sum
    };

    if size * n < threshold {
        // Sequential fast path: completely avoids Rayon thread pool synchronization overhead
        for i in 0..size {
            let w_row_offset = i * n;
            xout[i] = dot_product(&w[w_row_offset..w_row_offset + n]);
        }
    } else {
        // Parallel path: distributes matrix row dot-products across available CPU threads
        use rayon::prelude::*;
        xout.par_iter_mut().enumerate().for_each(|(i, out_val)| {
            let w_row_offset = i * n;
            *out_val = dot_product(&w[w_row_offset..w_row_offset + n]);
        });
    }
}

// ── Tokenizer ──

pub struct Tokenizer {
    pub vocab: Vec<String>,
    pub vocab_scores: Vec<f32>,
    pub vocab_map: std::collections::HashMap<String, usize>,
    pub max_token_length: usize,
}

impl Tokenizer {
    pub fn load<P: AsRef<Path>>(path: P, vocab_size: usize) -> Result<Self> {
        let mut file = File::open(path).context("Failed to open tokenizer file")?;
        
        let mut max_token_length_bytes = [0u8; 4];
        file.read_exact(&mut max_token_length_bytes).context("Failed to read max token length")?;
        let max_token_length = u32::from_le_bytes(max_token_length_bytes) as usize;

        let mut vocab = Vec::with_capacity(vocab_size);
        let mut vocab_scores = Vec::with_capacity(vocab_size);
        let mut vocab_map = std::collections::HashMap::with_capacity(vocab_size);

        for i in 0..vocab_size {
            let mut score_bytes = [0u8; 4];
            file.read_exact(&mut score_bytes).context("Failed to read token score")?;
            let score = f32::from_le_bytes(score_bytes);
            vocab_scores.push(score);

            let mut len_bytes = [0u8; 4];
            file.read_exact(&mut len_bytes).context("Failed to read token length")?;
            let len = u32::from_le_bytes(len_bytes) as usize;

            let mut token_bytes = vec![0u8; len];
            file.read_exact(&mut token_bytes).context("Failed to read token string")?;
            let token = String::from_utf8_lossy(&token_bytes).into_owned();
            vocab_map.insert(token.clone(), i);
            vocab.push(token);
        }

        Ok(Self {
            vocab,
            vocab_scores,
            vocab_map,
            max_token_length,
        })
    }

    pub fn decode(&self, prev_token: usize, token: usize) -> String {
        let mut piece = self.vocab[token].clone();
        
        // Handle Byte Fallback tokens: e.g. <0x0A> -> \n
        if piece.starts_with("<0x") && piece.ends_with('>') && piece.len() == 6 {
            if let Ok(byte_val) = u8::from_str_radix(&piece[3..5], 16) {
                piece = String::from(byte_val as char);
            }
        }
        
        // Handle sentencepiece spacing
        if prev_token == 1 { // BOS
            piece = piece.trim_start_matches(' ').to_string();
        }
        // Replace raw sentencepiece space representation (looks like a lower-one-eighth block: ' ')
        piece.replace(' ', " ")
    }
}
