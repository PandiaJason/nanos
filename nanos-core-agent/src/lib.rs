extern "C" {
    fn fs_read(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn web_get(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn memory_store(ptr: *const u8, len: usize) -> i32;
    fn memory_recall(ptr: *const u8, len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
    fn llm_infer(prompt_ptr: *const u8, prompt_len: usize, out_ptr: *mut u8, out_max: usize) -> i32;
}

#[no_mangle]
pub extern "C" fn run_agent() {
    // 1. Store the secret securely in the host SQLite database via native FFI syscall
    let secret = "The secret launch code is 42.";
    unsafe {
        memory_store(secret.as_ptr(), secret.len());
    }
    
    // 2. Recall the secret using a partial query string to prove SQL LIKE is operating
    let query = "launch code";
    let mut recall_buf = [0u8; 1024];
    let recall_len = unsafe {
        memory_recall(query.as_ptr(), query.len(), recall_buf.as_mut_ptr(), recall_buf.len())
    };
    let recalled_text = core::str::from_utf8(&recall_buf[..recall_len as usize]).unwrap_or("");
    
    // 3. Inject the retrieved memory into the LLM context to prove the loop works!
    let mut context = String::from("<|system|>\nYou are an AI agent.\n<|user|>\nWhat is the secret launch code? You recalled this memory from SQLite: '");
    context.push_str(recalled_text);
    context.push_str("'\n<|assistant|>\n");
    
    let mut out_buf = [0u8; 1024];
    
    let _response_len = unsafe {
        llm_infer(
            context.as_ptr(),
            context.len(),
            out_buf.as_mut_ptr(),
            out_buf.len()
        )
    };
}
