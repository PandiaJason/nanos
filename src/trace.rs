use serde::{Serialize, Deserialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTrace {
    pub step: u32,
    pub action: String,
    pub args: String,
    pub tokens: String,
    pub latency: Duration,
    pub result: String,
}

pub fn print_trace_table(traces: &[AgentTrace]) {
    if traces.is_empty() {
        println!("No traces recorded.");
        return;
    }

    println!("┌──────┬───────────┬────────────────────────────────┬──────────┬──────────┬──────────┐");
    println!("│ Step │ Action    │ Args                           │ Tokens   │ Latency  │ Result   │");
    println!("├──────┼───────────┼────────────────────────────────┼──────────┼──────────┼──────────┤");

    let mut total_prompt_tokens = 0;
    let mut total_gen_tokens = 0;
    let mut total_latency = Duration::from_secs(0);

    for trace in traces {
        let action = if trace.action.len() > 9 { &trace.action[..9] } else { &trace.action };
        let args = if trace.args.len() > 30 { format!("{}...", &trace.args[..27]) } else { trace.args.clone() };
        let result = if trace.result.len() > 8 { format!("{}...", &trace.result[..5]) } else { trace.result.clone() };
        
        let latency_str = if trace.latency.as_millis() > 0 {
            format!("{}ms", trace.latency.as_millis())
        } else {
            format!("{:.1}ms", trace.latency.as_micros() as f64 / 1000.0)
        };

        println!(
            "│ {:<4} │ {:<9} │ {:<30} │ {:<8} │ {:<8} │ {:<8} │",
            trace.step,
            action,
            args,
            trace.tokens,
            latency_str,
            result
        );

        total_latency += trace.latency;
        
        // Parse tokens like "312->45" or similar
        if trace.action == "llm_infer" {
            let parts: Vec<&str> = trace.tokens.split("→").collect();
            if parts.len() == 2 {
                if let (Ok(p), Ok(g)) = (parts[0].trim().parse::<u32>(), parts[1].trim().parse::<u32>()) {
                    total_prompt_tokens += p;
                    total_gen_tokens += g;
                }
            }
        }
    }

    println!("└──────┴───────────┴────────────────────────────────┴──────────┴──────────┴──────────┘");
    println!(
        "Total: {} steps, {} prompt tokens, {} generated tokens ({:.2?})",
        traces.len(),
        total_prompt_tokens,
        total_gen_tokens,
        total_latency
    );
}
