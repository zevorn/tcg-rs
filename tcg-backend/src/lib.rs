pub mod code_buffer;
pub mod liveness;
pub mod regalloc;
pub mod translate;
pub mod x86_64;

pub use code_buffer::CodeBuffer;
pub use x86_64::X86_64CodeGen;

/// Trait for host architecture code generators.
///
/// Each target architecture (x86-64, AArch64, RISC-V, etc.)
/// implements this trait to generate host machine code from TCG IR.
///
/// Reference: `~/qemu/tcg/<arch>/tcg-target.c.inc`.
pub trait HostCodeGen {
    /// Emit the prologue: save callee-saved registers, set up
    /// env pointer, allocate stack frame, jump to TB code.
    fn emit_prologue(&mut self, buf: &mut CodeBuffer);

    /// Emit the epilogue: restore callee-saved registers,
    /// deallocate stack frame, return to caller.
    fn emit_epilogue(&mut self, buf: &mut CodeBuffer);

    /// Patch a direct jump at `jump_offset` to point to
    /// `target_offset`. Used for TB chaining.
    fn patch_jump(
        &mut self,
        buf: &mut CodeBuffer,
        jump_offset: usize,
        target_offset: usize,
    );

    /// Return the offset of the TB return path.
    fn epilogue_offset(&self) -> usize;

    /// Initialize a translation context with backend-specific
    /// settings (reserved registers, stack frame layout, etc.).
    fn init_context(&self, ctx: &mut tcg_core::Context);

    // -- Register allocator primitives --

    /// Emit host mov between two registers.
    fn tcg_out_mov(
        &self,
        buf: &mut CodeBuffer,
        ty: tcg_core::Type,
        dst: u8,
        src: u8,
    );

    /// Emit host load-immediate into a register.
    fn tcg_out_movi(
        &self,
        buf: &mut CodeBuffer,
        ty: tcg_core::Type,
        dst: u8,
        val: u64,
    );

    /// Emit host load from memory [base + offset] into register.
    fn tcg_out_ld(
        &self,
        buf: &mut CodeBuffer,
        ty: tcg_core::Type,
        dst: u8,
        base: u8,
        offset: i64,
    );

    /// Emit host store from register to memory [base + offset].
    fn tcg_out_st(
        &self,
        buf: &mut CodeBuffer,
        ty: tcg_core::Type,
        src: u8,
        base: u8,
        offset: i64,
    );

    /// Emit host code for a single IR op. Called by the register
    /// allocator after inputs are loaded and outputs allocated.
    /// `oregs`/`iregs` are allocated host register numbers.
    /// `cargs` are raw constant values from the op's carg slots.
    fn tcg_out_op(
        &self,
        buf: &mut CodeBuffer,
        ctx: &tcg_core::Context,
        op: &tcg_core::Op,
        oregs: &[u8],
        iregs: &[u8],
        cargs: &[u32],
    );
}
