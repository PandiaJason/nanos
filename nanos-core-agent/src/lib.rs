extern "C" {
    fn fs_read(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn fs_write(path_ptr: *const u8, path_len: usize, content_ptr: *const u8, content_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn web_get(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn memory_store(ptr: *const u8, len: usize) -> i32;
    fn memory_recall(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn llm_infer(prompt_ptr: *const u8, prompt_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn get_manifest_goal(out_ptr: *mut u8, out_max: usize) -> i32;
    fn mcp_call(
        server_ptr: *const u8, server_len: usize,
        tool_ptr: *const u8, tool_len: usize,
        args_ptr: *const u8, args_len: usize,
        out_ptr: *mut u8, out_max: usize
    ) -> i32;
    fn agent_send(target_ptr: *const u8, target_len: usize, msg_ptr: *const u8, msg_len: usize) -> i32;
    fn agent_recv(out_ptr: *mut u8, out_max: usize) -> i32;
}

use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct ToolCall {
    action: String,
    args: serde_json::Value,
}

/// Build the system prompt using ChatML format for Qwen3.
/// /no_think disables the CoT thinking mode for clean, direct JSON output.
fn build_system_prompt() -> String {
    String::from(
        "<|im_start|>system\nYou are a tool-calling AI agent. You MUST respond with exactly one raw JSON object per turn, nothing else. No markdown, no explanation, no code fences.\n\nAvailable tools:\n- fs_read: Read a file. Args = string (the file path).\n- fs_write: Write a file. Args = {\"path\": \"...\", \"content\": \"...\"}.\n- agent_send: Send a message to another agent. Args = {\"target\": \"agent_name\", \"msg\": \"...\"}.\n- agent_recv: Receive message from queue. Args = empty.\n- done: Task complete. Args = string (result summary).\n\nYou MUST respond with ONLY a JSON object like:\n{\"action\": \"fs_read\", \"args\": \"instruction.txt\"}\n{\"action\": \"agent_send\", \"args\": {\"target\": \"writer\", \"msg\": \"The code is 42\"}}\n{\"action\": \"agent_recv\", \"args\": {}}\n{\"action\": \"done\", \"args\": \"Task finished successfully.\"}\n<|im_end|>\n"
    )
}

#[no_mangle]
pub extern "C" fn run_agent() {
    // Fetch the goal dynamically from the host manifest
    let mut goal_buf = [0u8; 2048];
    let goal_len = unsafe { get_manifest_goal(goal_buf.as_mut_ptr(), goal_buf.len()) };
    let goal = core::str::from_utf8(&goal_buf[..goal_len as usize]).unwrap_or("Unknown goal");

    let mut context = build_system_prompt();
    context.push_str("<|im_start|>user\n");
    context.push_str(goal);
    context.push_str("<|im_end|>\n");

    let max_steps = 10;

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
                // Route the tool call to the appropriate syscall
                let observation = dispatch_tool(&tool_call);

                if tool_call.action == "done" {
                    // Agent declared completion
                    break;
                }

                // Feed the observation back as a user message
                context.push_str("<|im_start|>user\n[Observation]: ");
                context.push_str(&observation);
                context.push_str("<|im_end|>\n");
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
        _ => format!("[Error]: Unknown action '{}'.", tc.action),
    }
}
