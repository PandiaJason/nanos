// Provide stub implementations of host FFI symbols when building for
// native targets (i.e. `cargo test`). On wasm32 the real host supplies them.
fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target != "wasm32" {
        // Write a small C file with no-op stubs and compile it in.
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let stubs_path = std::path::Path::new(&out_dir).join("host_stubs.c");
        std::fs::write(&stubs_path, r#"
#include <stddef.h>
#include <stdint.h>
int fs_read(const uint8_t *p, size_t l, uint8_t *o, size_t m) { return 0; }
int fs_write(const uint8_t *a, size_t b, const uint8_t *c, size_t d, uint8_t *e, size_t f) { return 0; }
int web_get(const uint8_t *p, size_t l, uint8_t *o, size_t m) { return 0; }
int memory_store(const uint8_t *p, size_t l) { return 0; }
int memory_recall(const uint8_t *p, size_t l, uint8_t *o, size_t m) { return 0; }
int llm_infer(const uint8_t *p, size_t l, uint8_t *o, size_t m) { return 0; }
int get_manifest_goal(uint8_t *o, size_t m) { return 0; }
int get_manifest_tools(uint8_t *o, size_t m) { return 0; }
int mcp_call(const uint8_t *a, size_t b, const uint8_t *c, size_t d, const uint8_t *e, size_t f, uint8_t *g, size_t h) { return 0; }
int agent_send(const uint8_t *a, size_t b, const uint8_t *c, size_t d) { return 0; }
int agent_recv(uint8_t *o, size_t m) { return 0; }
int eval_js(const uint8_t *p, size_t l, uint8_t *o, size_t m) { return 0; }
"#).unwrap();
        cc::Build::new()
            .file(&stubs_path)
            .flag("-Wno-unused-parameter")
            .compile("host_stubs");
    }
}
