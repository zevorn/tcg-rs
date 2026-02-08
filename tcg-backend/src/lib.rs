pub mod code_buffer;
pub mod x86_64;

pub use code_buffer::CodeBuffer;
pub use x86_64::X86_64CodeGen;

/// Trait for host architecture code generators.
///
/// Each target architecture (x86-64, AArch64, RISC-V, etc.) implements this
/// trait to generate host machine code from TCG IR.
///
/// Reference: `~/qemu/tcg/<arch>/tcg-target.c.inc`.
pub trait HostCodeGen {
    /// Emit the prologue: save callee-saved registers, set up env pointer,
    /// allocate stack frame, jump to TB code.
    /// Called once during backend initialization.
    fn emit_prologue(&mut self, buf: &mut CodeBuffer);

    /// Emit the epilogue: restore callee-saved registers, deallocate stack
    /// frame, return to caller. TB code jumps here via `exit_tb`.
    fn emit_epilogue(&mut self, buf: &mut CodeBuffer);

    /// Patch a direct jump at `jump_offset` to point to `target_offset`.
    /// Used for TB chaining (`goto_tb` patching).
    fn patch_jump(&mut self, buf: &mut CodeBuffer, jump_offset: usize, target_offset: usize);

    /// Return the offset of the TB return path in the code buffer.
    fn epilogue_offset(&self) -> usize;

    /// Initialize a translation context with backend-specific settings
    /// (reserved registers, stack frame layout, etc.).
    fn init_context(&self, ctx: &mut tcg_core::Context);
}
