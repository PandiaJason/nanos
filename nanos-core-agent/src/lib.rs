extern "C" {
    fn fs_read(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn llm_infer(prompt_ptr: *const u8, prompt_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
}

#[no_mangle]
pub extern "C" fn run_agent() {
    let mut context = String::from("<|system|>\nYou are an AI agent. You have one tool available: fs_read. You MUST output exactly 'ACTION: fs_read' to read the file.\n<|user|>\nRead the file /docs.\n<|assistant|>\n");
    
    let mut out_buf = [0u8; 1024];
    
    for _ in 0..5 {
        let response_len = unsafe {
            llm_infer(
                context.as_ptr(),
                context.len(),
                out_buf.as_mut_ptr(),
                out_buf.len()
            )
        };
        
        if response_len <= 0 {
            break;
        }
        
        let response = core::str::from_utf8(&out_buf[..response_len as usize]).unwrap_or("");
        context.push_str(response);
        
        if response.contains("ACTION: fs_read") {
            let msg = "/docs";
            let mut fs_buf = [0u8; 1024];
            let fs_len = unsafe { fs_read(msg.as_ptr(), msg.len(), fs_buf.as_mut_ptr(), fs_buf.len()) };
            let fs_response = core::str::from_utf8(&fs_buf[..fs_len as usize]).unwrap_or("");
            
            context.push_str("\n<|observation|>\n");
            context.push_str(fs_response);
            context.push_str("\n<|assistant|>\n");
        } else {
            // Assume it's a final answer
            break;
        }
    }
}
