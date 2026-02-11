//! TCG Execution Engine — TB cache and CPU execution loop.
//!
//! Provides the execution loop that drives the
//! lookup → translate → execute cycle, with TB caching via
//! a global hash table and per-CPU jump cache.
//!
//! Reference: `~/qemu/accel/tcg/cpu-exec.c`,
//! `~/qemu/accel/tcg/translate-all.c`.

pub mod exec_loop;
pub mod tb_store;

pub use exec_loop::{cpu_exec_loop, ExitReason};
pub use tb_store::TbStore;

use tcg_backend::code_buffer::CodeBuffer;
use tcg_backend::HostCodeGen;
use tcg_core::tb::JumpCache;
use tcg_core::Context;

/// Trait for guest CPU state used by the execution loop.
///
/// Each guest architecture implements this to provide PC/flags
/// access and frontend translation.
pub trait GuestCpu {
    /// Return the current guest program counter.
    fn get_pc(&self) -> u64;

    /// Return CPU flags that affect translation.
    fn get_flags(&self) -> u32;

    /// Translate guest code starting at `pc` into IR.
    ///
    /// Returns the number of guest bytes translated.
    /// Called only on TB cache miss; implementations should
    /// register globals on the first call and reuse them on
    /// subsequent calls.
    fn gen_code(&mut self, ir: &mut Context, pc: u64, max_insns: u32) -> u32;

    /// Return a raw pointer to the CPU env struct.
    fn env_ptr(&mut self) -> *mut u8;
}

/// Execution environment holding all shared translation state.
pub struct ExecEnv<B: HostCodeGen> {
    pub tb_store: TbStore,
    pub jump_cache: JumpCache,
    pub code_buf: CodeBuffer,
    pub backend: B,
    pub ir_ctx: Context,
    /// Offset where TB code generation starts (after
    /// prologue/epilogue).
    pub code_gen_start: usize,
}

/// Minimum remaining bytes in code buffer before refusing
/// to translate a new TB.
const MIN_CODE_BUF_REMAINING: usize = 4096;

impl<B: HostCodeGen> ExecEnv<B> {
    /// Create a new execution environment.
    ///
    /// Emits prologue and epilogue into the code buffer and
    /// initializes the IR context with backend-specific settings.
    pub fn new(mut backend: B) -> Self {
        let mut code_buf =
            CodeBuffer::new(16 * 1024 * 1024).expect("mmap failed");
        backend.emit_prologue(&mut code_buf);
        backend.emit_epilogue(&mut code_buf);
        let code_gen_start = code_buf.offset();

        let mut ir_ctx = Context::new();
        backend.init_context(&mut ir_ctx);

        Self {
            tb_store: TbStore::new(),
            jump_cache: JumpCache::new(),
            code_buf,
            backend,
            ir_ctx,
            code_gen_start,
        }
    }
}
