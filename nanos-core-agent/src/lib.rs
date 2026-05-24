extern "C" {
    fn fs_read(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn web_get(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn memory_store(ptr: *const u8, len: usize) -> i32;
    fn memory_recall(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn llm_infer(prompt_ptr: *const u8, prompt_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
}

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
struct ToolCall {
    action: String,
    args: String,
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
- fs_read: reads a file. Args: absolute path.
- web_get: fetches a URL. Args: the URL.
- done: finishes the task. Args: result summary.

Example output:
{\"action\": \"fs_read\", \"args\": \"/workspace/report.txt\"}
";

    let mut context = String::from(system_prompt);
    context.push_str("\n<|user|>\nRead the file /etc/passwd and summarize it. If it fails, output done with 'Failed'.\n");
    
    let mut step_count = 0;
    loop {
        if step_count > 5 { break; } // Safety limit
        step_count += 1;
        
        context.push_str("<|assistant|>\n");
        let mut out_buf = [0u8; 1024];
        
        let response_len = unsafe {
            llm_infer(
                context.as_ptr(),
                context.len(),
                out_buf.as_mut_ptr(),
                out_buf.len()
            )
        };
        
        let llm_output = core::str::from_utf8(&out_buf[..response_len as usize]).unwrap_or("").trim();
        context.push_str(llm_output);
        context.push_str("\n");
        
        if let Ok(tool_call) = serde_json::from_str::<ToolCall>(llm_output) {
            if tool_call.action == "done" {
                break;
            } else if tool_call.action == "fs_read" {
                let mut read_buf = [0u8; 1024];
                let read_len = unsafe {
                    fs_read(tool_call.args.as_ptr(), tool_call.args.len(), read_buf.as_mut_ptr(), read_buf.len())
                };
                let file_content = core::str::from_utf8(&read_buf[..read_len as usize]).unwrap_or("");
                context.push_str("<|observation|>\n");
                context.push_str(file_content);
                context.push_str("\n");
            } else if tool_call.action == "web_get" {
                let mut web_buf = [0u8; 1024];
                let web_len = unsafe {
                    web_get(tool_call.args.as_ptr(), tool_call.args.len(), web_buf.as_mut_ptr(), web_buf.len())
                };
                let web_resp = core::str::from_utf8(&web_buf[..web_len as usize]).unwrap_or("");
                context.push_str("<|observation|>\n");
                context.push_str(web_resp);
                context.push_str("\n");
            } else {
                context.push_str("<|observation|>\nUnknown tool\n");
            }
        } else {
            // Failed to parse JSON, nudge it back
            context.push_str("<|observation|>\nError: Must output valid JSON matching ToolCall schema.\n");
        }
    }
}
