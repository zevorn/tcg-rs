use crate::code_buffer::CodeBuffer;
use crate::x86_64::regs::{
    Reg, CALLEE_SAVED, CALL_ARG_REGS, STACK_ADDEND, STATIC_CALL_ARGS_SIZE, TCG_AREG0,
};
use crate::HostCodeGen;

/// x86-64 backend code generator.
///
/// Manages the prologue/epilogue layout and provides methods for
/// emitting TB control flow instructions (exit_tb, goto_tb, goto_ptr).
pub struct X86_64CodeGen {
    /// Offset where the prologue starts.
    pub prologue_offset: usize,
    /// Offset of the `goto_ptr` return path (sets rax=0, falls through to epilogue).
    pub epilogue_return_zero_offset: usize,
    /// Offset of the TB return path (rax already set by exit_tb).
    pub tb_ret_offset: usize,
    /// Offset right after the prologue jmp, where TB code generation begins.
    pub code_gen_start: usize,
}

impl X86_64CodeGen {
    pub fn new() -> Self {
        Self {
            prologue_offset: 0,
            epilogue_return_zero_offset: 0,
            tb_ret_offset: 0,
            code_gen_start: 0,
        }
    }

    /// Emit `exit_tb(val)`: load return value into rax and jump to epilogue.
    ///
    /// If val == 0, jumps to the zero-return path (avoids redundant mov).
    /// Otherwise, loads val into rax and jumps to tb_ret_offset.
    pub fn emit_exit_tb(&self, buf: &mut CodeBuffer, val: u64) {
        if val == 0 {
            emit_jmp_rel32(buf, self.epilogue_return_zero_offset);
        } else {
            emit_mov_imm64(buf, Reg::Rax, val);
            emit_jmp_rel32(buf, self.tb_ret_offset);
        }
    }

    /// Emit `goto_tb(n)`: a patchable direct jump (5 bytes: E9 + disp32).
    ///
    /// Returns the offset of the jump instruction (for recording in TB).
    /// The displacement is initially 0 and will be patched by TB chaining.
    /// NOP padding ensures the disp32 is 4-byte aligned for atomic patching.
    pub fn emit_goto_tb(&self, buf: &mut CodeBuffer) -> (usize, usize) {
        // Align the displacement field to 4 bytes for atomic patching.
        // The E9 opcode is 1 byte, so we need (code_ptr + 1) to be 4-aligned.
        let target_align = (buf.offset() + 1 + 3) & !3;
        let nop_count = target_align - (buf.offset() + 1);
        emit_nops(buf, nop_count);

        let jmp_offset = buf.offset();
        buf.emit_u8(0xE9); // JMP rel32
        buf.emit_u32(0); // placeholder displacement
        let reset_offset = buf.offset();

        (jmp_offset, reset_offset)
    }

    /// Emit `goto_ptr(reg)`: indirect jump through a register.
    ///
    /// Used for `lookup_and_goto_ptr` — after looking up a TB in the
    /// jump cache, jump directly to its host code.
    pub fn emit_goto_ptr(buf: &mut CodeBuffer, reg: Reg) {
        emit_jmp_reg(buf, reg);
    }
}

impl Default for X86_64CodeGen {
    fn default() -> Self {
        Self::new()
    }
}

// -- x86-64 instruction encoding helpers --

/// Emit `push reg` (64-bit).
fn emit_push(buf: &mut CodeBuffer, reg: Reg) {
    if reg.needs_rex() {
        buf.emit_u8(0x41); // REX.B
    }
    buf.emit_u8(0x50 + reg.low3());
}

/// Emit `pop reg` (64-bit).
fn emit_pop(buf: &mut CodeBuffer, reg: Reg) {
    if reg.needs_rex() {
        buf.emit_u8(0x41); // REX.B
    }
    buf.emit_u8(0x58 + reg.low3());
}

/// Emit `mov dst, src` (64-bit register to register).
fn emit_mov_rr(buf: &mut CodeBuffer, dst: Reg, src: Reg) {
    let mut rex: u8 = 0x48; // REX.W
    if src.needs_rex() {
        rex |= 0x04; // REX.R
    }
    if dst.needs_rex() {
        rex |= 0x01; // REX.B
    }
    buf.emit_u8(rex);
    buf.emit_u8(0x89); // MOV r/m64, r64
    buf.emit_u8(0xC0 | (src.low3() << 3) | dst.low3());
}

/// Emit `mov reg, imm64` (64-bit immediate).
fn emit_mov_imm64(buf: &mut CodeBuffer, reg: Reg, val: u64) {
    if val == 0 {
        // xor %eax, %eax (or equivalent for other regs)
        emit_xor_rr32(buf, reg, reg);
    } else if val <= u32::MAX as u64 {
        // mov %reg32, imm32 (zero-extends to 64-bit)
        if reg.needs_rex() {
            buf.emit_u8(0x41); // REX.B
        }
        buf.emit_u8(0xB8 + reg.low3());
        buf.emit_u32(val as u32);
    } else {
        // movabs %reg, imm64
        let mut rex: u8 = 0x48; // REX.W
        if reg.needs_rex() {
            rex |= 0x01; // REX.B
        }
        buf.emit_u8(rex);
        buf.emit_u8(0xB8 + reg.low3());
        buf.emit_u64(val);
    }
}

/// Emit `xor %r32, %r32` (32-bit, zero-extends to 64-bit).
fn emit_xor_rr32(buf: &mut CodeBuffer, dst: Reg, src: Reg) {
    if dst.needs_rex() || src.needs_rex() {
        let mut rex: u8 = 0x40;
        if src.needs_rex() {
            rex |= 0x04;
        }
        if dst.needs_rex() {
            rex |= 0x01;
        }
        buf.emit_u8(rex);
    }
    buf.emit_u8(0x31); // XOR r/m32, r32
    buf.emit_u8(0xC0 | (src.low3() << 3) | dst.low3());
}

/// Emit `jmp *reg` (indirect jump through register).
fn emit_jmp_reg(buf: &mut CodeBuffer, reg: Reg) {
    if reg.needs_rex() {
        buf.emit_u8(0x41); // REX.B
    }
    buf.emit_u8(0xFF);
    buf.emit_u8(0xE0 | reg.low3()); // JMP r/m64, /4
}

/// Emit `ret`.
fn emit_ret(buf: &mut CodeBuffer) {
    buf.emit_u8(0xC3);
}

/// Emit `sub rsp, imm32`.
fn emit_sub_rsp_imm32(buf: &mut CodeBuffer, imm: u32) {
    if imm == 0 {
        return;
    }
    buf.emit_u8(0x48); // REX.W
    if imm <= 127 {
        buf.emit_u8(0x83); // SUB r/m64, imm8
        buf.emit_u8(0xEC); // ModR/M: mod=11, reg=5(/5=SUB), rm=RSP
        buf.emit_u8(imm as u8);
    } else {
        buf.emit_u8(0x81); // SUB r/m64, imm32
        buf.emit_u8(0xEC);
        buf.emit_u32(imm);
    }
}

/// Emit `add rsp, imm32`.
fn emit_add_rsp_imm32(buf: &mut CodeBuffer, imm: u32) {
    if imm == 0 {
        return;
    }
    buf.emit_u8(0x48); // REX.W
    if imm <= 127 {
        buf.emit_u8(0x83); // ADD r/m64, imm8
        buf.emit_u8(0xC4); // ModR/M: mod=11, reg=0(/0=ADD), rm=RSP
        buf.emit_u8(imm as u8);
    } else {
        buf.emit_u8(0x81); // ADD r/m64, imm32
        buf.emit_u8(0xC4);
        buf.emit_u32(imm);
    }
}

/// Emit `jmp rel32` to an absolute offset in the code buffer.
fn emit_jmp_rel32(buf: &mut CodeBuffer, target_offset: usize) {
    let jmp_start = buf.offset();
    buf.emit_u8(0xE9); // JMP rel32
                       // Displacement is relative to the instruction after the jmp (jmp_start + 5).
    let disp = target_offset as i64 - (jmp_start as i64 + 5);
    buf.emit_u32(disp as u32);
}

/// Emit `n` bytes of NOP padding.
fn emit_nops(buf: &mut CodeBuffer, n: usize) {
    for _ in 0..n {
        buf.emit_u8(0x90);
    }
}

impl HostCodeGen for X86_64CodeGen {
    fn emit_prologue(&mut self, buf: &mut CodeBuffer) {
        self.prologue_offset = buf.offset();

        // 1. Save callee-saved registers (RBP, RBX, R12, R13, R14, R15).
        for &reg in CALLEE_SAVED {
            emit_push(buf, reg);
        }

        // 2. Load env pointer: mov TCG_AREG0 (rbp), rdi (first argument).
        emit_mov_rr(buf, TCG_AREG0, CALL_ARG_REGS[0]);

        // 3. Allocate stack frame: sub rsp, STACK_ADDEND.
        emit_sub_rsp_imm32(buf, STACK_ADDEND as u32);

        // 4. Jump to TB code: jmp *rsi (second argument = TB host code pointer).
        emit_jmp_reg(buf, CALL_ARG_REGS[1]);

        self.code_gen_start = buf.offset();
    }

    fn emit_epilogue(&mut self, buf: &mut CodeBuffer) {
        // -- goto_ptr return path: set rax = 0, then fall through to epilogue --
        self.epilogue_return_zero_offset = buf.offset();
        emit_xor_rr32(buf, Reg::Rax, Reg::Rax);

        // -- TB return path: rax already contains the return value --
        self.tb_ret_offset = buf.offset();

        // 1. Deallocate stack frame.
        emit_add_rsp_imm32(buf, STACK_ADDEND as u32);

        // 2. Restore callee-saved registers (reverse order).
        for &reg in CALLEE_SAVED.iter().rev() {
            emit_pop(buf, reg);
        }

        // 3. Return to execution loop.
        emit_ret(buf);
    }

    fn patch_jump(&mut self, buf: &mut CodeBuffer, jump_offset: usize, target_offset: usize) {
        // x86-64 JMP rel32: E9 + 4-byte displacement.
        // Displacement is relative to the instruction after the jump (jump_offset + 5).
        let disp = (target_offset as i64) - (jump_offset as i64 + 5);
        assert!(
            disp >= i32::MIN as i64 && disp <= i32::MAX as i64,
            "jump displacement out of i32 range"
        );
        buf.patch_u32(jump_offset + 1, disp as u32);
    }

    fn epilogue_offset(&self) -> usize {
        self.tb_ret_offset
    }

    fn init_context(&self, ctx: &mut tcg_core::Context) {
        ctx.reserved_regs = crate::x86_64::regs::RESERVED_REGS;
        ctx.set_frame(
            Reg::Rsp as u8,
            STATIC_CALL_ARGS_SIZE as i64,
            (crate::x86_64::regs::CPU_TEMP_BUF_NLONGS * 8) as i64,
        );
    }
}
