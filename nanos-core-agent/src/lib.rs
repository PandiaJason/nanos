extern "C" {
    fn fs_read(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn fs_write(path_ptr: *const u8, path_len: usize, content_ptr: *const u8, content_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn web_get(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn memory_store(ptr: *const u8, len: usize) -> i32;
    fn memory_recall(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn llm_infer(prompt_ptr: *const u8, prompt_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn get_manifest_goal(out_ptr: *mut u8, out_max: usize) -> i32;
}

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
struct ToolCall {
    action: String,
    args: serde_json::Value,
}

#[no_mangle]
pub extern "C" fn run_agent() {
    // SECURITY TEST: Attempt unauthorized web request
    let url = "https://example.com";
    let mut web_buf = [0u8; 1024];
    let web_len = unsafe { web_get(url.as_ptr(), url.len(), web_buf.as_mut_ptr(), web_buf.len()) };
    let web_resp = core::str::from_utf8(&web_buf[..web_len as usize]).unwrap_or("");
    // The host should have intercepted this!

    // SECURITY TEST: Attempt unauthorized file read
    let path = "/etc/passwd";
    let mut read_buf = [0u8; 1024];
    let read_len = unsafe { fs_read(path.as_ptr(), path.len(), read_buf.as_mut_ptr(), read_buf.len()) };
    let read_resp = core::str::from_utf8(&read_buf[..read_len as usize]).unwrap_or("");
    // The host should have intercepted this too!
    
    let system_prompt = "<|system|>\nYou are an AI agent. When you want to execute a tool, you MUST output a raw JSON object and nothing else.
Allowed tools:
- fs_read: reads a file. Args: absolute path (string).
- fs_write: writes a file. Args: {\"path\": \"...\", \"content\": \"...\"}
- web_get: fetches a URL. Args: the URL (string).
- done: finishes the task. Args: result summary (string).

Example output:
{\"action\": \"fs_read\", \"args\": \"/workspace/report.txt\"}
";

    // Fetch the goal dynamically from the host manifest
    let mut goal_buf = [0u8; 1024];
    let goal_len = unsafe { get_manifest_goal(goal_buf.as_mut_ptr(), goal_buf.len()) };
    let goal = core::str::from_utf8(&goal_buf[..goal_len as usize]).unwrap_or("Unknown goal");

    let mut context = String::from(system_prompt);
    context.push_str("\n<|user|>\nGoal: ");
    context.push_str(goal);
    context.push_str("\n");
    
    let mut step_count = 0;
    loop {
        if step_count > 15 { break; } // Safety limit for infinite loops
        step_count += 1;
        
        context.push_str("<|assistant|>\n");
        let mut out_buf = [0u8; 2048];
        
        let response_len = unsafe {
            llm_infer(
                context.as_ptr(),
                context.len(),
                out_buf.as_mut_ptr(),
                out_buf.len()
            )
        };
        
        let raw_output = core::str::from_utf8(&out_buf[..response_len as usize]).unwrap_or("").trim();
        context.push_str(raw_output);
        context.push_str("\n");
        
        // Extract JSON substring (first { to last })
        let json_start = raw_output.find('{');
        let json_end = raw_output.rfind('}');
        
        if let (Some(start), Some(end)) = (json_start, json_end) {
            let json_str = &raw_output[start..=end];
            if let Ok(tool_call) = serde_json::from_str::<ToolCall>(json_str) {
                if tool_call.action == "done" {
                    break;
                } else if tool_call.action == "fs_read" {
                    if let Some(path) = tool_call.args.as_str() {
                        let mut read_buf = [0u8; 4096];
                        let read_len = unsafe {
                            fs_read(path.as_ptr(), path.len(), read_buf.as_mut_ptr(), read_buf.len())
                        };
                        let file_content = core::str::from_utf8(&read_buf[..read_len as usize]).unwrap_or("");
                        context.push_str("<|observation|>\n");
                        context.push_str(file_content);
                        context.push_str("\n");
                    }
                } else if tool_call.action == "fs_write" {
                    if let (Some(path), Some(content)) = (tool_call.args.get("path").and_then(|v| v.as_str()), tool_call.args.get("content").and_then(|v| v.as_str())) {
                        let mut write_buf = [0u8; 1024];
                        let write_len = unsafe {
                            fs_write(path.as_ptr(), path.len(), content.as_ptr(), content.len(), write_buf.as_mut_ptr(), write_buf.len())
                        };
                        let result_msg = core::str::from_utf8(&write_buf[..write_len as usize]).unwrap_or("");
                        context.push_str("<|observation|>\n");
                        context.push_str(result_msg);
                        context.push_str("\n");
                    } else {
                        context.push_str("<|observation|>\nError: fs_write requires 'path' and 'content' string fields.\n");
                    }
                } else if tool_call.action == "web_get" {
                    if let Some(url) = tool_call.args.as_str() {
                        let mut web_buf = [0u8; 4096];
                        let web_len = unsafe {
                            web_get(url.as_ptr(), url.len(), web_buf.as_mut_ptr(), web_buf.len())
                        };
                        let web_resp = core::str::from_utf8(&web_buf[..web_len as usize]).unwrap_or("");
                        context.push_str("<|observation|>\n");
                        context.push_str(web_resp);
                        context.push_str("\n");
                    }
                } else {
                    context.push_str("<|observation|>\nUnknown tool\n");
                }
            } else {
                context.push_str("<|observation|>\nError: Failed to parse tool arguments.\n");
            }
        } else {
            // No JSON found
            context.push_str("<|observation|>\nError: Must output valid JSON matching ToolCall schema.\n");
        }
    }
}
