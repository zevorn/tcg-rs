//! RISC-V frontend — RV64 user-mode instruction translation.

pub mod cpu;
#[allow(dead_code)]
mod insn_decode;
mod trans;

use crate::{DisasContextBase, DisasJumpType, TranslatorOps};
use cpu::{gpr_offset, NUM_GPRS, PC_OFFSET};
use tcg_core::{Context, TempIdx, Type};

// ---------------------------------------------------------------
// Disassembly context
// ---------------------------------------------------------------

/// RISC-V disassembly context (extends `DisasContextBase`).
pub struct RiscvDisasContext {
    /// Generic base fields (pc, is_jmp, counters).
    pub base: DisasContextBase,
    /// IR temp for the env pointer (fixed to host RBP).
    pub env: TempIdx,
    /// IR temps for guest GPRs x0-x31 (globals).
    pub gpr: [TempIdx; NUM_GPRS],
    /// IR temp for the guest PC (global).
    pub pc: TempIdx,
    /// Raw instruction word being decoded.
    pub opcode: u32,
    /// Length of the current instruction (2 or 4).
    pub cur_insn_len: u32,
    /// Pointer to guest code bytes for fetching.
    pub guest_base: *const u8,
}

impl RiscvDisasContext {
    /// Create a new context for translating a TB starting
    /// at `pc`.  `guest_base` points to the host mapping of
    /// guest memory (user-mode: identity).
    pub fn new(pc: u64, guest_base: *const u8) -> Self {
        Self {
            base: DisasContextBase {
                pc_first: pc,
                pc_next: pc,
                is_jmp: DisasJumpType::Next,
                num_insns: 0,
                max_insns: 512,
            },
            env: TempIdx(0),
            gpr: [TempIdx(0); NUM_GPRS],
            pc: TempIdx(0),
            opcode: 0,
            cur_insn_len: 4,
            guest_base,
        }
    }

    /// Fetch a 32-bit instruction at the current PC.
    ///
    /// # Safety
    /// `guest_base + pc_next` must be a valid, readable
    /// 4-byte aligned host address.
    unsafe fn fetch_insn32(&self) -> u32 {
        let ptr = self.guest_base.add(self.base.pc_next as usize) as *const u32;
        ptr.read_unaligned()
    }
}

// ---------------------------------------------------------------
// TranslatorOps implementation
// ---------------------------------------------------------------

/// Marker type for the RISC-V translator.
pub struct RiscvTranslator;

impl TranslatorOps for RiscvTranslator {
    type DisasContext = RiscvDisasContext;

    fn init_disas_context(ctx: &mut RiscvDisasContext, ir: &mut Context) {
        // Register the env pointer (fixed to host RBP = reg 5).
        ctx.env = ir.new_fixed(Type::I64, 5, "env");

        // Register guest GPRs as globals at known offsets.
        for i in 0..NUM_GPRS {
            ctx.gpr[i] =
                ir.new_global(Type::I64, ctx.env, gpr_offset(i), "gpr");
        }

        // Register guest PC as a global.
        ctx.pc = ir.new_global(Type::I64, ctx.env, PC_OFFSET, "pc");
    }

    fn tb_start(_ctx: &mut RiscvDisasContext, _ir: &mut Context) {
        // Nothing special for user-mode.
    }

    fn insn_start(ctx: &mut RiscvDisasContext, ir: &mut Context) {
        ir.gen_insn_start(ctx.base.pc_next);
        ctx.base.num_insns += 1;
    }

    fn translate_insn(ctx: &mut RiscvDisasContext, ir: &mut Context) {
        // Fetch instruction (user-mode: direct host access).
        let insn = unsafe { ctx.fetch_insn32() };
        ctx.opcode = insn;
        ctx.cur_insn_len = 4;

        // Dispatch through decodetree-generated decoder.
        if !insn_decode::decode(ctx, ir, insn) {
            // Unrecognized instruction — sync PC and exit.
            let pc_val = ctx.base.pc_next;
            let pc_const = ir.new_const(Type::I64, pc_val);
            ir.gen_mov(Type::I64, ctx.pc, pc_const);
            ir.gen_exit_tb(0);
            ctx.base.is_jmp = DisasJumpType::NoReturn;
        }

        ctx.base.pc_next += ctx.cur_insn_len as u64;
    }

    fn tb_stop(ctx: &mut RiscvDisasContext, ir: &mut Context) {
        match ctx.base.is_jmp {
            DisasJumpType::NoReturn => {
                // TB already terminated by the instruction.
            }
            DisasJumpType::Next | DisasJumpType::TooMany => {
                // Fall through: update PC and exit.
                let pc_val = ctx.base.pc_next;
                let pc_const = ir.new_const(Type::I64, pc_val);
                ir.gen_mov(Type::I64, ctx.pc, pc_const);
                ir.gen_exit_tb(0);
            }
        }
    }

    fn base(ctx: &RiscvDisasContext) -> &DisasContextBase {
        &ctx.base
    }

    fn base_mut(ctx: &mut RiscvDisasContext) -> &mut DisasContextBase {
        &mut ctx.base
    }
}
