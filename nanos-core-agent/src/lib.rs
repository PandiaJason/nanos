#[allow(dead_code)]
extern "C" {
    fn fs_read(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn fs_write(path_ptr: *const u8, path_len: usize, content_ptr: *const u8, content_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn web_get(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn memory_store(ptr: *const u8, len: usize) -> i32;
    fn memory_recall(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn llm_infer(prompt_ptr: *const u8, prompt_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn get_manifest_goal(out_ptr: *mut u8, out_max: usize) -> i32;
    fn get_manifest_tools(out_ptr: *mut u8, out_max: usize) -> i32;
    fn mcp_call(
        server_ptr: *const u8, server_len: usize,
        tool_ptr: *const u8, tool_len: usize,
        args_ptr: *const u8, args_len: usize,
        out_ptr: *mut u8, out_max: usize
    ) -> i32;
    fn agent_send(target_ptr: *const u8, target_len: usize, msg_ptr: *const u8, msg_len: usize) -> i32;
    fn agent_recv(out_ptr: *mut u8, out_max: usize) -> i32;
    fn eval_js(code_ptr: *const u8, code_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
}

use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct ToolCall {
    action: String,
    args: serde_json::Value,
}

/// Build the system prompt using ChatML format for Qwen.
/// Tailored dynamically to only include allowed tools.
fn build_system_prompt(allowed_tools: &[&str]) -> String {
    let mut prompt = String::from(
        "<|im_start|>system\nYou are a tool-calling AI agent. You MUST respond with exactly one raw JSON object per turn. No markdown, no explanation, no code fences, no backticks.\n\nAvailable tools:\n"
    );

    for tool in allowed_tools {
        match *tool {
            "fs_read" => {
                prompt.push_str("- fs_read: Read a file. Args = string (the file path).\n");
            }
            "fs_write" => {
                prompt.push_str("- fs_write: Write content to a file. Args = {\"path\": \"...\", \"content\": \"...\"}.\n");
            }
            "web_get" => {
                prompt.push_str("- web_get: Fetch contents of a URL. Args = string (the URL).\n");
            }
            "mcp_call" => {
                prompt.push_str("- mcp_call: Call an external MCP tool. Args = {\"server\": \"...\", \"tool\": \"...\", \"arguments\": {...}}.\n");
            }
            "agent_send" => {
                prompt.push_str("- agent_send: Send a message to another agent. Args = {\"target\": \"...\", \"msg\": \"...\"}.\n");
            }
            "agent_recv" => {
                prompt.push_str("- agent_recv: Receive the next message from your message queue. Args = null or empty object (no arguments needed).\n");
            }
            "eval_js" => {
                prompt.push_str("- eval_js: Evaluate JavaScript code. Args = string (the code to run).\n");
            }
            "done" => {
                prompt.push_str("- done: Task complete. Args = string (result summary).\n");
            }
            _ => {
                prompt.push_str(&format!("- {}: Execute tool {}.\n", tool, tool));
            }
        }
    }

    // Done tool is always allowed
    if !allowed_tools.contains(&"done") {
        prompt.push_str("- done: Task complete. Args = string (result summary).\n");
    }

    prompt.push_str("\nIMPORTANT RULES:\n");
    prompt.push_str("1. Respond with ONLY one JSON object containing 'action' and 'args'. No extra text.\n");
    prompt.push_str("2. Once the goal is completed, call done.\n");

    prompt.push_str("\nExamples of correct format:\n");
    for tool in allowed_tools {
        match *tool {
            "fs_read" => {
                prompt.push_str("{\"action\": \"fs_read\", \"args\": \"instruction.txt\"}\n");
            }
            "fs_write" => {
                prompt.push_str("{\"action\": \"fs_write\", \"args\": {\"path\": \"output.txt\", \"content\": \"hello\"}}\n");
            }
            "agent_send" => {
                prompt.push_str("{\"action\": \"agent_send\", \"args\": {\"target\": \"writer\", \"msg\": \"hello\"}}\n");
            }
            "agent_recv" => {
                prompt.push_str("{\"action\": \"agent_recv\", \"args\": {}}\n");
            }
            _ => {}
        }
    }
    prompt.push_str("{\"action\": \"done\", \"args\": \"finished successfully\"}\n");
    prompt.push_str("<|im_end|>\n");

    prompt
}

#[no_mangle]
pub extern "C" fn run_agent() {
    // Fetch the goal dynamically from the host manifest
    let mut goal_buf = [0u8; 2048];
    let goal_len = unsafe { get_manifest_goal(goal_buf.as_mut_ptr(), goal_buf.len()) };
    let goal = core::str::from_utf8(&goal_buf[..goal_len as usize]).unwrap_or("Unknown goal");

    // Fetch the allowed tools dynamically from the host manifest
    let mut tools_buf = [0u8; 1024];
    let tools_len = unsafe { get_manifest_tools(tools_buf.as_mut_ptr(), tools_buf.len()) };
    let tools_str = core::str::from_utf8(&tools_buf[..tools_len as usize]).unwrap_or("");
    let allowed_tools: Vec<&str> = if tools_str.is_empty() {
        vec!["fs_read", "fs_write", "done"] // fallback defaults
    } else {
        tools_str.split(',').collect()
    };

    let mut context = build_system_prompt(&allowed_tools);
    context.push_str("<|im_start|>user\n");
    context.push_str(goal);
    context.push_str("<|im_end|>\n");

    let max_steps = 10;
    let mut last_action = String::new();
    let mut repeat_count = 0u32;

    for _step in 0..max_steps {
        // Ask the LLM to generate a tool call
        context.push_str("<|im_start|>assistant\n");

        let mut out_buf = [0u8; 4096];
        let response_len = unsafe {
            llm_infer(
                context.as_ptr(),
                context.len(),
                out_buf.as_mut_ptr(),
                out_buf.len(),
            )
        };

        let raw_output = core::str::from_utf8(&out_buf[..response_len as usize])
            .unwrap_or("")
            .trim();

        // Append the LLM's raw output to the context
        context.push_str(raw_output);
        context.push_str("<|im_end|>\n");

        // Extract the first JSON object from the output
        let json_str = extract_json(raw_output);

        if let Some(json) = json_str {
            if let Ok(tool_call) = serde_json::from_str::<ToolCall>(json) {
                // Detect repeating loops
                let current_action = format!("{}:{}", tool_call.action, tool_call.args);
                if current_action == last_action {
                    repeat_count += 1;
                } else {
                    repeat_count = 0;
                }
                last_action = current_action;

                // Route the tool call to the appropriate syscall
                let observation = dispatch_tool(&tool_call);

                if tool_call.action == "done" {
                    break;
                }

                // Truncate very long observations to prevent context overflow
                let truncated_obs = if observation.len() > 1500 {
                    let mut t = observation[..1500].to_string();
                    t.push_str("\n...[truncated]");
                    t
                } else {
                    observation
                };

                // Feed the observation back as a user message
                if repeat_count >= 2 {
                    // Agent is stuck — give it a strong nudge
                    context.push_str("<|im_start|>user\n[Observation]: ");
                    context.push_str(&truncated_obs);
                    context.push_str("\n\nYou already read this file. Now use fs_write to write the result to a file, then call done.<|im_end|>\n");
                } else {
                    context.push_str("<|im_start|>user\n[Observation]: ");
                    context.push_str(&truncated_obs);
                    context.push_str("\n\nNow decide your next action. Respond with a JSON object.<|im_end|>\n");
                }
            } else {
                context.push_str("<|im_start|>user\n[Error]: Your output was not valid JSON. Respond with ONLY a JSON object like {\"action\": \"fs_read\", \"args\": \"file.txt\"}<|im_end|>\n");
            }
        } else {
            context.push_str("<|im_start|>user\n[Error]: No JSON object found in your response. Respond with ONLY a JSON object.<|im_end|>\n");
        }
    }
}


/// Extract the first balanced JSON object `{...}` from a string.
fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, ch) in text[start..].char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Route a parsed ToolCall to the correct host syscall.
fn dispatch_tool(tc: &ToolCall) -> String {
    match tc.action.as_str() {
        "fs_read" => {
            if let Some(path) = tc.args.as_str() {
                let mut buf = [0u8; 8192];
                let len = unsafe { fs_read(path.as_ptr(), path.len(), buf.as_mut_ptr(), buf.len()) };
                core::str::from_utf8(&buf[..len as usize])
                    .unwrap_or("[Error reading file]")
                    .to_string()
            } else {
                "[Error]: fs_read args must be a string (file path).".to_string()
            }
        }
        "fs_write" => {
            let path = tc.args.get("path").and_then(|v| v.as_str());
            let content = tc.args.get("content").and_then(|v| v.as_str());
            if let (Some(p), Some(c)) = (path, content) {
                let mut buf = [0u8; 1024];
                let len = unsafe {
                    fs_write(p.as_ptr(), p.len(), c.as_ptr(), c.len(), buf.as_mut_ptr(), buf.len())
                };
                core::str::from_utf8(&buf[..len as usize])
                    .unwrap_or("[Error writing file]")
                    .to_string()
            } else {
                "[Error]: fs_write args must be {\"path\": \"...\", \"content\": \"...\"}".to_string()
            }
        }
        "web_get" => {
            if let Some(url) = tc.args.as_str() {
                let mut buf = [0u8; 8192];
                let len = unsafe { web_get(url.as_ptr(), url.len(), buf.as_mut_ptr(), buf.len()) };
                core::str::from_utf8(&buf[..len as usize])
                    .unwrap_or("[Error fetching URL]")
                    .to_string()
            } else {
                "[Error]: web_get args must be a string (URL).".to_string()
            }
        }
        "mcp_call" => {
            let server = tc.args.get("server").and_then(|v| v.as_str());
            let tool = tc.args.get("tool").and_then(|v| v.as_str());
            let arguments = tc.args.get("arguments");
            
            if let (Some(s), Some(t), Some(args)) = (server, tool, arguments) {
                let args_str = args.to_string();
                let mut buf = [0u8; 16384]; // Give a larger buffer for MCP tool output
                let len = unsafe {
                    mcp_call(
                        s.as_ptr(), s.len(),
                        t.as_ptr(), t.len(),
                        args_str.as_ptr(), args_str.len(),
                        buf.as_mut_ptr(), buf.len()
                    )
                };
                core::str::from_utf8(&buf[..len as usize])
                    .unwrap_or("[Error calling MCP tool]")
                    .to_string()
            } else {
                "[Error]: mcp_call args must be {\"server\": \"...\", \"tool\": \"...\", \"arguments\": {...}}".to_string()
            }
        }
        "done" => {
            let summary = tc.args.as_str().unwrap_or("No summary provided.");
            summary.to_string()
        }
        "agent_send" => {
            let target = tc.args.get("target").and_then(|v| v.as_str());
            let msg = tc.args.get("msg").and_then(|v| v.as_str());
            if let (Some(t), Some(m)) = (target, msg) {
                unsafe { agent_send(t.as_ptr(), t.len(), m.as_ptr(), m.len()) };
                "Message sent successfully.".to_string()
            } else {
                "[Error]: agent_send args must be {\"target\": \"...\", \"msg\": \"...\"}".to_string()
            }
        }
        "agent_recv" => {
            let mut buf = [0u8; 8192];
            let len = unsafe { agent_recv(buf.as_mut_ptr(), buf.len()) };
            if len > 0 {
                core::str::from_utf8(&buf[..len as usize])
                    .unwrap_or("[Error decoding message]")
                    .to_string()
            } else {
                "[No messages in queue]".to_string()
            }
        }
        "eval_js" => {
            if let Some(code) = tc.args.as_str() {
                let mut buf = [0u8; 8192];
                let len = unsafe { eval_js(code.as_ptr(), code.len(), buf.as_mut_ptr(), buf.len()) };
                core::str::from_utf8(&buf[..len as usize])
                    .unwrap_or("[Error executing JS]")
                    .to_string()
            } else {
                "[Error]: eval_js args must be a string (JS code).".to_string()
            }
        }
        _ => format!("[Error]: Unknown action '{}'.", tc.action),
    }
}

#[cfg(test)]
mod tests {
    use super::extract_json;

    #[test]
    fn extract_clean_json() {
        let input = r#"{"action": "done", "args": "ok"}"#;
        assert_eq!(extract_json(input), Some(input));
    }

    #[test]
    fn extract_json_from_markdown_fences() {
        let input = "```json\n{\"action\": \"fs_read\", \"args\": \"file.txt\"}\n```";
        let expected = r#"{"action": "fs_read", "args": "file.txt"}"#;
        assert_eq!(extract_json(input), Some(expected));
    }

    #[test]
    fn extract_nested_json() {
        let input = r#"{"action": "fs_write", "args": {"path": "out.txt", "content": "hello"}}"#;
        assert_eq!(extract_json(input), Some(input));
        // Verify it's actually valid JSON
        let parsed: serde_json::Value = serde_json::from_str(extract_json(input).unwrap()).unwrap();
        assert_eq!(parsed["args"]["path"], "out.txt");
    }

    #[test]
    fn extract_json_returns_none_for_no_json() {
        assert_eq!(extract_json("no json here"), None);
        assert_eq!(extract_json(""), None);
        assert_eq!(extract_json("just some text with no braces"), None);
    }

    #[test]
    fn extract_json_with_escaped_quotes() {
        let input = r#"{"action": "done", "args": "He said \"hello\""}"#;
        let result = extract_json(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(result).unwrap();
        assert_eq!(parsed["action"], "done");
        assert_eq!(parsed["args"], r#"He said "hello""#);
    }

    #[test]
    fn extract_json_with_leading_text() {
        let input = "Here is the result: {\"action\": \"done\", \"args\": \"finished\"} end";
        let expected = r#"{"action": "done", "args": "finished"}"#;
        assert_eq!(extract_json(input), Some(expected));
    }

    #[test]
    fn extract_json_unbalanced_returns_none() {
        // Opening brace but never closed
        assert_eq!(extract_json("{\"action\": \"test\""), None);
    }
}
