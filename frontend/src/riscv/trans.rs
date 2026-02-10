//! RISC-V instruction translation — TCG IR generation.
//!
//! Follows QEMU's gen_xxx helper pattern: repetitive instruction
//! translation logic is factored into gen_arith, gen_arith_imm,
//! gen_shift_imm, gen_shiftw, etc., each parameterised by a
//! `BinOp` function pointer.

use super::insn_decode::*;
use super::RiscvDisasContext;
use crate::DisasJumpType;
use tcg_core::context::Context;
use tcg_core::types::{Cond, Type};
use tcg_core::TempIdx;

/// Binary IR operation: `fn(ir, ty, dst, lhs, rhs) -> dst`.
type BinOp = fn(&mut Context, Type, TempIdx, TempIdx, TempIdx) -> TempIdx;

// ── Helpers ────────────────────────────────────────────────────

impl RiscvDisasContext {
    // -- GPR access ----------------------------------------

    /// Read GPR `idx`; x0 yields a constant zero.
    fn gpr_or_zero(&self, ir: &mut Context, idx: i64) -> TempIdx {
        if idx == 0 {
            ir.new_const(Type::I64, 0)
        } else {
            self.gpr[idx as usize]
        }
    }

    /// Write `val` into GPR `rd`; writes to x0 discarded.
    fn gen_set_gpr(&self, ir: &mut Context, rd: i64, val: TempIdx) {
        if rd != 0 {
            ir.gen_mov(Type::I64, self.gpr[rd as usize], val);
        }
    }

    /// Sign-extend low 32 bits into a 64-bit GPR.
    fn gen_set_gpr_sx32(&self, ir: &mut Context, rd: i64, val: TempIdx) {
        if rd != 0 {
            ir.gen_ext_i32_i64(self.gpr[rd as usize], val);
        }
    }

    // -- R-type helpers ------------------------------------

    /// R-type ALU: `rd = op(rs1, rs2)`.
    fn gen_arith(&self, ir: &mut Context, a: &ArgsR, op: BinOp) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let d = ir.new_temp(Type::I64);
        op(ir, Type::I64, d, s1, s2);
        self.gen_set_gpr(ir, a.rd, d);
        true
    }

    /// R-type setcond: `rd = (rs1 cond rs2) ? 1 : 0`.
    fn gen_setcond_rr(&self, ir: &mut Context, a: &ArgsR, cond: Cond) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let d = ir.new_temp(Type::I64);
        ir.gen_setcond(Type::I64, d, s1, s2, cond);
        self.gen_set_gpr(ir, a.rd, d);
        true
    }

    // -- I-type helpers ------------------------------------

    /// I-type ALU: `rd = op(rs1, sext(imm))`.
    fn gen_arith_imm(&self, ir: &mut Context, a: &ArgsI, op: BinOp) -> bool {
        let src = self.gpr_or_zero(ir, a.rs1);
        let imm = ir.new_const(Type::I64, a.imm as u64);
        let d = ir.new_temp(Type::I64);
        op(ir, Type::I64, d, src, imm);
        self.gen_set_gpr(ir, a.rd, d);
        true
    }

    /// I-type setcond: `rd = (rs1 cond imm) ? 1 : 0`.
    fn gen_setcond_imm(&self, ir: &mut Context, a: &ArgsI, cond: Cond) -> bool {
        let src = self.gpr_or_zero(ir, a.rs1);
        let imm = ir.new_const(Type::I64, a.imm as u64);
        let d = ir.new_temp(Type::I64);
        ir.gen_setcond(Type::I64, d, src, imm, cond);
        self.gen_set_gpr(ir, a.rd, d);
        true
    }

    // -- Shift helpers -------------------------------------

    /// Shift immediate: `rd = op(rs1, shamt)`.
    fn gen_shift_imm(
        &self,
        ir: &mut Context,
        a: &ArgsShift,
        op: BinOp,
    ) -> bool {
        let src = self.gpr_or_zero(ir, a.rs1);
        let sh = ir.new_const(Type::I64, a.shamt as u64);
        let d = ir.new_temp(Type::I64);
        op(ir, Type::I64, d, src, sh);
        self.gen_set_gpr(ir, a.rd, d);
        true
    }

    // -- W-suffix helpers (RV64) ---------------------------

    /// R-type W: `rd = sext32(op(rs1, rs2))`.
    fn gen_arith_w(&self, ir: &mut Context, a: &ArgsR, op: BinOp) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let d = ir.new_temp(Type::I64);
        op(ir, Type::I64, d, s1, s2);
        self.gen_set_gpr_sx32(ir, a.rd, d);
        true
    }

    /// I-type W: `rd = sext32(op(rs1, imm))`.
    fn gen_arith_imm_w(&self, ir: &mut Context, a: &ArgsI, op: BinOp) -> bool {
        let src = self.gpr_or_zero(ir, a.rs1);
        let imm = ir.new_const(Type::I64, a.imm as u64);
        let d = ir.new_temp(Type::I64);
        op(ir, Type::I64, d, src, imm);
        self.gen_set_gpr_sx32(ir, a.rd, d);
        true
    }

    /// R-type shift W: truncate to I32, shift, sext.
    fn gen_shiftw(&self, ir: &mut Context, a: &ArgsR, op: BinOp) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let a32 = ir.new_temp(Type::I32);
        ir.gen_extrl_i64_i32(a32, s1);
        let b32 = ir.new_temp(Type::I32);
        ir.gen_extrl_i64_i32(b32, s2);
        let d32 = ir.new_temp(Type::I32);
        op(ir, Type::I32, d32, a32, b32);
        self.gen_set_gpr_sx32(ir, a.rd, d32);
        true
    }

    /// Shift immediate W: truncate to I32, shift, sext.
    fn gen_shift_imm_w(
        &self,
        ir: &mut Context,
        a: &ArgsShift,
        op: BinOp,
    ) -> bool {
        let src = self.gpr_or_zero(ir, a.rs1);
        let s32 = ir.new_temp(Type::I32);
        ir.gen_extrl_i64_i32(s32, src);
        let sh = ir.new_const(Type::I32, a.shamt as u64);
        let d32 = ir.new_temp(Type::I32);
        op(ir, Type::I32, d32, s32, sh);
        self.gen_set_gpr_sx32(ir, a.rd, d32);
        true
    }

    // -- Branch helper -------------------------------------

    /// Conditional branch that terminates the TB.
    fn gen_branch(&mut self, ir: &mut Context, a: &ArgsB, cond: Cond) {
        let src1 = self.gpr_or_zero(ir, a.rs1);
        let src2 = self.gpr_or_zero(ir, a.rs2);

        let taken = ir.new_label();
        ir.gen_brcond(Type::I64, src1, src2, cond, taken);

        // Not taken: PC = next insn
        let next_pc = self.base.pc_next + 4;
        let c = ir.new_const(Type::I64, next_pc);
        ir.gen_mov(Type::I64, self.pc, c);
        ir.gen_exit_tb(0);

        // Taken: PC = branch target
        ir.gen_set_label(taken);
        let target = (self.base.pc_next as i64 + a.imm) as u64;
        let c = ir.new_const(Type::I64, target);
        ir.gen_mov(Type::I64, self.pc, c);
        ir.gen_exit_tb(0);

        self.base.is_jmp = DisasJumpType::NoReturn;
    }
}

// ── Decode trait implementation ────────────────────────────────

impl Decode<Context> for RiscvDisasContext {
    // ── RV32I: Upper immediate ─────────────────────────

    fn trans_lui(&mut self, ir: &mut Context, a: &ArgsU) -> bool {
        let c = ir.new_const(Type::I64, a.imm as u64);
        self.gen_set_gpr(ir, a.rd, c);
        true
    }

    fn trans_auipc(&mut self, ir: &mut Context, a: &ArgsU) -> bool {
        let v = (self.base.pc_next as i64 + a.imm) as u64;
        let c = ir.new_const(Type::I64, v);
        self.gen_set_gpr(ir, a.rd, c);
        true
    }

    // ── RV32I: Jumps ───────────────────────────────────

    fn trans_jal(&mut self, ir: &mut Context, a: &ArgsJ) -> bool {
        let link = self.base.pc_next + 4;
        let c = ir.new_const(Type::I64, link);
        self.gen_set_gpr(ir, a.rd, c);
        let target = (self.base.pc_next as i64 + a.imm) as u64;
        let c = ir.new_const(Type::I64, target);
        ir.gen_mov(Type::I64, self.pc, c);
        ir.gen_exit_tb(0);
        self.base.is_jmp = DisasJumpType::NoReturn;
        true
    }

    fn trans_jalr(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        let link = self.base.pc_next + 4;
        let src = self.gpr_or_zero(ir, a.rs1);
        let imm = ir.new_const(Type::I64, a.imm as u64);
        let tmp = ir.new_temp(Type::I64);
        ir.gen_add(Type::I64, tmp, src, imm);
        // Clear bit 0
        let mask = ir.new_const(Type::I64, !1u64);
        ir.gen_and(Type::I64, tmp, tmp, mask);
        let c = ir.new_const(Type::I64, link);
        self.gen_set_gpr(ir, a.rd, c);
        ir.gen_mov(Type::I64, self.pc, tmp);
        ir.gen_exit_tb(0);
        self.base.is_jmp = DisasJumpType::NoReturn;
        true
    }

    // ── RV32I: Branches ────────────────────────────────

    fn trans_beq(&mut self, ir: &mut Context, a: &ArgsB) -> bool {
        self.gen_branch(ir, a, Cond::Eq);
        true
    }
    fn trans_bne(&mut self, ir: &mut Context, a: &ArgsB) -> bool {
        self.gen_branch(ir, a, Cond::Ne);
        true
    }
    fn trans_blt(&mut self, ir: &mut Context, a: &ArgsB) -> bool {
        self.gen_branch(ir, a, Cond::Lt);
        true
    }
    fn trans_bge(&mut self, ir: &mut Context, a: &ArgsB) -> bool {
        self.gen_branch(ir, a, Cond::Ge);
        true
    }
    fn trans_bltu(&mut self, ir: &mut Context, a: &ArgsB) -> bool {
        self.gen_branch(ir, a, Cond::Ltu);
        true
    }
    fn trans_bgeu(&mut self, ir: &mut Context, a: &ArgsB) -> bool {
        self.gen_branch(ir, a, Cond::Geu);
        true
    }

    // ── RV32I: Loads (need guest memory ops) ───────────

    fn trans_lb(&mut self, _ir: &mut Context, _a: &ArgsI) -> bool {
        false
    }
    fn trans_lh(&mut self, _ir: &mut Context, _a: &ArgsI) -> bool {
        false
    }
    fn trans_lw(&mut self, _ir: &mut Context, _a: &ArgsI) -> bool {
        false
    }
    fn trans_lbu(&mut self, _ir: &mut Context, _a: &ArgsI) -> bool {
        false
    }
    fn trans_lhu(&mut self, _ir: &mut Context, _a: &ArgsI) -> bool {
        false
    }

    // ── RV32I: Stores (need guest memory ops) ──────────

    fn trans_sb(&mut self, _ir: &mut Context, _a: &ArgsS) -> bool {
        false
    }
    fn trans_sh(&mut self, _ir: &mut Context, _a: &ArgsS) -> bool {
        false
    }
    fn trans_sw(&mut self, _ir: &mut Context, _a: &ArgsS) -> bool {
        false
    }

    // ── RV32I: ALU immediate ───────────────────────────

    fn trans_addi(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_arith_imm(ir, a, Context::gen_add)
    }
    fn trans_slti(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_setcond_imm(ir, a, Cond::Lt)
    }
    fn trans_sltiu(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_setcond_imm(ir, a, Cond::Ltu)
    }
    fn trans_xori(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_arith_imm(ir, a, Context::gen_xor)
    }
    fn trans_ori(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_arith_imm(ir, a, Context::gen_or)
    }
    fn trans_andi(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_arith_imm(ir, a, Context::gen_and)
    }

    // ── RV32I: Shift immediate ─────────────────────────

    fn trans_slli(&mut self, ir: &mut Context, a: &ArgsShift) -> bool {
        self.gen_shift_imm(ir, a, Context::gen_shl)
    }
    fn trans_srli(&mut self, ir: &mut Context, a: &ArgsShift) -> bool {
        self.gen_shift_imm(ir, a, Context::gen_shr)
    }
    fn trans_srai(&mut self, ir: &mut Context, a: &ArgsShift) -> bool {
        self.gen_shift_imm(ir, a, Context::gen_sar)
    }

    // ── RV32I: R-type ALU ──────────────────────────────

    fn trans_add(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith(ir, a, Context::gen_add)
    }
    fn trans_sub(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith(ir, a, Context::gen_sub)
    }
    fn trans_sll(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith(ir, a, Context::gen_shl)
    }
    fn trans_slt(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_setcond_rr(ir, a, Cond::Lt)
    }
    fn trans_sltu(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_setcond_rr(ir, a, Cond::Ltu)
    }
    fn trans_xor(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith(ir, a, Context::gen_xor)
    }
    fn trans_srl(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith(ir, a, Context::gen_shr)
    }
    fn trans_sra(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith(ir, a, Context::gen_sar)
    }
    fn trans_or(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith(ir, a, Context::gen_or)
    }
    fn trans_and(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith(ir, a, Context::gen_and)
    }

    // ── RV32I: Fence / System ──────────────────────────

    fn trans_fence(&mut self, _ir: &mut Context, _a: &ArgsAutoFence) -> bool {
        true // NOP for user-mode
    }

    fn trans_ecall(&mut self, ir: &mut Context, _a: &ArgsEmpty) -> bool {
        let pc = ir.new_const(Type::I64, self.base.pc_next);
        ir.gen_mov(Type::I64, self.pc, pc);
        ir.gen_exit_tb(1);
        self.base.is_jmp = DisasJumpType::NoReturn;
        true
    }

    fn trans_ebreak(&mut self, ir: &mut Context, _a: &ArgsEmpty) -> bool {
        let pc = ir.new_const(Type::I64, self.base.pc_next);
        ir.gen_mov(Type::I64, self.pc, pc);
        ir.gen_exit_tb(2);
        self.base.is_jmp = DisasJumpType::NoReturn;
        true
    }

    // ── RV64I: Loads / Stores (need guest memory) ──────

    fn trans_lwu(&mut self, _ir: &mut Context, _a: &ArgsI) -> bool {
        false
    }
    fn trans_ld(&mut self, _ir: &mut Context, _a: &ArgsI) -> bool {
        false
    }
    fn trans_sd(&mut self, _ir: &mut Context, _a: &ArgsS) -> bool {
        false
    }

    // ── RV64I: W-suffix ALU ────────────────────────────

    fn trans_addiw(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_arith_imm_w(ir, a, Context::gen_add)
    }
    fn trans_slliw(&mut self, ir: &mut Context, a: &ArgsShift) -> bool {
        self.gen_shift_imm_w(ir, a, Context::gen_shl)
    }
    fn trans_srliw(&mut self, ir: &mut Context, a: &ArgsShift) -> bool {
        self.gen_shift_imm_w(ir, a, Context::gen_shr)
    }
    fn trans_sraiw(&mut self, ir: &mut Context, a: &ArgsShift) -> bool {
        self.gen_shift_imm_w(ir, a, Context::gen_sar)
    }
    fn trans_addw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith_w(ir, a, Context::gen_add)
    }
    fn trans_subw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith_w(ir, a, Context::gen_sub)
    }
    fn trans_sllw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_shiftw(ir, a, Context::gen_shl)
    }
    fn trans_srlw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_shiftw(ir, a, Context::gen_shr)
    }
    fn trans_sraw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_shiftw(ir, a, Context::gen_sar)
    }

    // ── RV32M: Multiply / Divide (deferred) ────────────

    fn trans_mul(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_mulh(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_mulhsu(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_mulhu(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_div(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_divu(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_rem(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_remu(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }

    // ── RV64M: W-suffix Mul / Div (deferred) ───────────

    fn trans_mulw(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_divw(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_divuw(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_remw(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
    fn trans_remuw(&mut self, _ir: &mut Context, _a: &ArgsR) -> bool {
        false
    }
}
