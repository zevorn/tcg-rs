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

use std::fmt;

use tcg_backend::code_buffer::CodeBuffer;
use tcg_backend::HostCodeGen;
use tcg_core::tb::JumpCache;
use tcg_core::Context;

/// Execution statistics for profiling the TB lookup/chain pipeline.
#[derive(Default)]
pub struct ExecStats {
    pub loop_iters: u64,
    // TB lookup
    pub jc_hit: u64,
    pub ht_hit: u64,
    pub translate: u64,
    // Exit types
    pub chain_exit: [u64; 2],
    pub nochain_exit: u64,
    pub real_exit: u64,
    // Chaining
    pub chain_patched: u64,
    pub chain_cycle: u64,
    pub chain_already: u64,
    // Hint
    pub hint_used: u64,
}

impl fmt::Display for ExecStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total_lookup = self.jc_hit + self.ht_hit + self.translate;
        writeln!(f, "=== TCG Execution Stats ===")?;
        writeln!(f, "loop iters:    {}", self.loop_iters)?;
        writeln!(f, "--- TB lookup ---")?;
        writeln!(
            f,
            "  jc hit:      {} ({:.1}%)",
            self.jc_hit,
            pct(self.jc_hit, total_lookup)
        )?;
        writeln!(
            f,
            "  ht hit:      {} ({:.1}%)",
            self.ht_hit,
            pct(self.ht_hit, total_lookup)
        )?;
        writeln!(
            f,
            "  translate:   {} ({:.1}%)",
            self.translate,
            pct(self.translate, total_lookup)
        )?;
        writeln!(f, "--- Exit types ---")?;
        writeln!(f, "  chain[0]:    {}", self.chain_exit[0])?;
        writeln!(f, "  chain[1]:    {}", self.chain_exit[1])?;
        writeln!(f, "  nochain:     {}", self.nochain_exit)?;
        writeln!(f, "  real exit:   {}", self.real_exit)?;
        writeln!(f, "--- Chaining ---")?;
        writeln!(f, "  patched:     {}", self.chain_patched)?;
        writeln!(f, "  cycle:       {}", self.chain_cycle)?;
        writeln!(f, "  already:     {}", self.chain_already)?;
        writeln!(f, "--- Hint ---")?;
        writeln!(f, "  hint used:   {}", self.hint_used)?;
        Ok(())
    }
}

fn pct(n: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        n as f64 / total as f64 * 100.0
    }
}

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
    pub stats: ExecStats,
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
            stats: ExecStats::default(),
        }
    }
}
