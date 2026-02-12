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
use tcg_core::types::{Cond, MemOp, Type};
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

    // -- Guest memory helpers --------------------------------

    /// Guest load: rd = *(addr), addr = rs1 + imm.
    fn gen_load(&self, ir: &mut Context, a: &ArgsI, memop: MemOp) -> bool {
        let base = self.gpr_or_zero(ir, a.rs1);
        let addr = if a.imm != 0 {
            let imm = ir.new_const(Type::I64, a.imm as u64);
            let t = ir.new_temp(Type::I64);
            ir.gen_add(Type::I64, t, base, imm)
        } else {
            base
        };
        let dst = ir.new_temp(Type::I64);
        ir.gen_qemu_ld(Type::I64, dst, addr, memop.bits() as u32);
        self.gen_set_gpr(ir, a.rd, dst);
        true
    }

    /// Guest store: *(addr) = rs2, addr = rs1 + imm.
    fn gen_store(&self, ir: &mut Context, a: &ArgsS, memop: MemOp) -> bool {
        let base = self.gpr_or_zero(ir, a.rs1);
        let addr = if a.imm != 0 {
            let imm = ir.new_const(Type::I64, a.imm as u64);
            let t = ir.new_temp(Type::I64);
            ir.gen_add(Type::I64, t, base, imm)
        } else {
            base
        };
        let val = self.gpr_or_zero(ir, a.rs2);
        ir.gen_qemu_st(Type::I64, val, addr, memop.bits() as u32);
        true
    }

    // -- R-type ALU helpers ----------------------------------

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

    // -- M-extension helpers (mul/div/rem) -----------------

    /// Signed division with RISC-V special-case handling.
    /// div-by-zero → -1 (quot) / dividend (rem).
    /// MIN / -1 → MIN (quot) / 0 (rem).
    fn gen_div_rem(&self, ir: &mut Context, a: &ArgsR, want_rem: bool) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let zero = ir.new_const(Type::I64, 0);
        let one = ir.new_const(Type::I64, 1);
        let neg1 = ir.new_const(Type::I64, u64::MAX);

        // Replace divisor=0 with 1 to avoid trap
        let safe = ir.new_temp(Type::I64);
        ir.gen_movcond(Type::I64, safe, s2, zero, one, s2, Cond::Eq);
        // Replace divisor=-1 with 1 to avoid overflow
        ir.gen_movcond(Type::I64, safe, safe, neg1, one, safe, Cond::Eq);

        let ah = ir.new_temp(Type::I64);
        let c63 = ir.new_const(Type::I64, 63);
        ir.gen_sar(Type::I64, ah, s1, c63);

        let quot = ir.new_temp(Type::I64);
        let rem = ir.new_temp(Type::I64);
        ir.gen_divs2(Type::I64, quot, rem, s1, ah, safe);

        if want_rem {
            // 0 → s1, -1 → 0, else → rem
            let r = ir.new_temp(Type::I64);
            ir.gen_movcond(Type::I64, r, s2, zero, s1, rem, Cond::Eq);
            ir.gen_movcond(Type::I64, r, s2, neg1, zero, r, Cond::Eq);
            self.gen_set_gpr(ir, a.rd, r);
        } else {
            // 0 → -1, -1 → neg(s1), else → quot
            let neg_s1 = ir.new_temp(Type::I64);
            ir.gen_neg(Type::I64, neg_s1, s1);
            let r = ir.new_temp(Type::I64);
            ir.gen_movcond(Type::I64, r, s2, zero, neg1, quot, Cond::Eq);
            ir.gen_movcond(Type::I64, r, s2, neg1, neg_s1, r, Cond::Eq);
            self.gen_set_gpr(ir, a.rd, r);
        }
        true
    }

    /// Unsigned division with RISC-V special-case handling.
    /// div-by-zero → MAX (quot) / dividend (rem).
    fn gen_divu_remu(
        &self,
        ir: &mut Context,
        a: &ArgsR,
        want_rem: bool,
    ) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let zero = ir.new_const(Type::I64, 0);
        let one = ir.new_const(Type::I64, 1);

        let safe = ir.new_temp(Type::I64);
        ir.gen_movcond(Type::I64, safe, s2, zero, one, s2, Cond::Eq);

        let quot = ir.new_temp(Type::I64);
        let rem = ir.new_temp(Type::I64);
        ir.gen_divu2(Type::I64, quot, rem, s1, zero, safe);

        if want_rem {
            let r = ir.new_temp(Type::I64);
            ir.gen_movcond(Type::I64, r, s2, zero, s1, rem, Cond::Eq);
            self.gen_set_gpr(ir, a.rd, r);
        } else {
            let neg1 = ir.new_const(Type::I64, u64::MAX);
            let r = ir.new_temp(Type::I64);
            ir.gen_movcond(Type::I64, r, s2, zero, neg1, quot, Cond::Eq);
            self.gen_set_gpr(ir, a.rd, r);
        }
        true
    }

    /// 32-bit signed division (W-suffix).
    fn gen_div_rem_w(
        &self,
        ir: &mut Context,
        a: &ArgsR,
        want_rem: bool,
    ) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let a32 = ir.new_temp(Type::I32);
        ir.gen_extrl_i64_i32(a32, s1);
        let b32 = ir.new_temp(Type::I32);
        ir.gen_extrl_i64_i32(b32, s2);

        let zero = ir.new_const(Type::I32, 0);
        let one = ir.new_const(Type::I32, 1);
        let neg1 = ir.new_const(Type::I32, u32::MAX as u64);

        let safe = ir.new_temp(Type::I32);
        ir.gen_movcond(Type::I32, safe, b32, zero, one, b32, Cond::Eq);
        ir.gen_movcond(Type::I32, safe, safe, neg1, one, safe, Cond::Eq);

        let ah = ir.new_temp(Type::I32);
        let c31 = ir.new_const(Type::I32, 31);
        ir.gen_sar(Type::I32, ah, a32, c31);

        let quot = ir.new_temp(Type::I32);
        let rem = ir.new_temp(Type::I32);
        ir.gen_divs2(Type::I32, quot, rem, a32, ah, safe);

        if want_rem {
            let r = ir.new_temp(Type::I32);
            ir.gen_movcond(Type::I32, r, b32, zero, a32, rem, Cond::Eq);
            ir.gen_movcond(Type::I32, r, b32, neg1, zero, r, Cond::Eq);
            self.gen_set_gpr_sx32(ir, a.rd, r);
        } else {
            let neg_a = ir.new_temp(Type::I32);
            ir.gen_neg(Type::I32, neg_a, a32);
            let r = ir.new_temp(Type::I32);
            ir.gen_movcond(Type::I32, r, b32, zero, neg1, quot, Cond::Eq);
            ir.gen_movcond(Type::I32, r, b32, neg1, neg_a, r, Cond::Eq);
            self.gen_set_gpr_sx32(ir, a.rd, r);
        }
        true
    }

    /// 32-bit unsigned division (W-suffix).
    fn gen_divu_remu_w(
        &self,
        ir: &mut Context,
        a: &ArgsR,
        want_rem: bool,
    ) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let a32 = ir.new_temp(Type::I32);
        ir.gen_extrl_i64_i32(a32, s1);
        let b32 = ir.new_temp(Type::I32);
        ir.gen_extrl_i64_i32(b32, s2);

        let zero = ir.new_const(Type::I32, 0);
        let one = ir.new_const(Type::I32, 1);

        let safe = ir.new_temp(Type::I32);
        ir.gen_movcond(Type::I32, safe, b32, zero, one, b32, Cond::Eq);

        let quot = ir.new_temp(Type::I32);
        let rem = ir.new_temp(Type::I32);
        ir.gen_divu2(Type::I32, quot, rem, a32, zero, safe);

        if want_rem {
            let r = ir.new_temp(Type::I32);
            ir.gen_movcond(Type::I32, r, b32, zero, a32, rem, Cond::Eq);
            self.gen_set_gpr_sx32(ir, a.rd, r);
        } else {
            let max = ir.new_const(Type::I32, u32::MAX as u64);
            let r = ir.new_temp(Type::I32);
            ir.gen_movcond(Type::I32, r, b32, zero, max, quot, Cond::Eq);
            self.gen_set_gpr_sx32(ir, a.rd, r);
        }
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
        let next_pc = self.base.pc_next + self.cur_insn_len as u64;
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
        let link = self.base.pc_next + self.cur_insn_len as u64;
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
        let link = self.base.pc_next + self.cur_insn_len as u64;
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

    // ── RV32I: Loads ──────────────────────────────────

    fn trans_lb(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_load(ir, a, MemOp::sb())
    }
    fn trans_lh(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_load(ir, a, MemOp::sw())
    }
    fn trans_lw(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_load(ir, a, MemOp::sl())
    }
    fn trans_lbu(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_load(ir, a, MemOp::ub())
    }
    fn trans_lhu(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_load(ir, a, MemOp::uw())
    }

    // ── RV32I: Stores ─────────────────────────────────

    fn trans_sb(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        self.gen_store(ir, a, MemOp::ub())
    }
    fn trans_sh(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        self.gen_store(ir, a, MemOp::uw())
    }
    fn trans_sw(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        self.gen_store(ir, a, MemOp::ul())
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

    fn trans_lwu(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_load(ir, a, MemOp::ul())
    }
    fn trans_ld(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_load(ir, a, MemOp::uq())
    }
    fn trans_sd(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        self.gen_store(ir, a, MemOp::uq())
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

    // ── RV32M: Multiply / Divide ────────────────────────

    fn trans_mul(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith(ir, a, Context::gen_mul)
    }

    fn trans_mulh(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let lo = ir.new_temp(Type::I64);
        let hi = ir.new_temp(Type::I64);
        ir.gen_muls2(Type::I64, lo, hi, s1, s2);
        self.gen_set_gpr(ir, a.rd, hi);
        true
    }

    fn trans_mulhsu(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let lo = ir.new_temp(Type::I64);
        let hi = ir.new_temp(Type::I64);
        ir.gen_mulu2(Type::I64, lo, hi, s1, s2);
        // Correction: high -= (s1 >> 63) & s2
        let c63 = ir.new_const(Type::I64, 63);
        let sign = ir.new_temp(Type::I64);
        ir.gen_sar(Type::I64, sign, s1, c63);
        let adj = ir.new_temp(Type::I64);
        ir.gen_and(Type::I64, adj, sign, s2);
        ir.gen_sub(Type::I64, hi, hi, adj);
        self.gen_set_gpr(ir, a.rd, hi);
        true
    }

    fn trans_mulhu(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        let s1 = self.gpr_or_zero(ir, a.rs1);
        let s2 = self.gpr_or_zero(ir, a.rs2);
        let lo = ir.new_temp(Type::I64);
        let hi = ir.new_temp(Type::I64);
        ir.gen_mulu2(Type::I64, lo, hi, s1, s2);
        self.gen_set_gpr(ir, a.rd, hi);
        true
    }

    fn trans_div(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_div_rem(ir, a, false)
    }

    fn trans_divu(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_divu_remu(ir, a, false)
    }

    fn trans_rem(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_div_rem(ir, a, true)
    }

    fn trans_remu(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_divu_remu(ir, a, true)
    }

    // ── RV64M: W-suffix Mul / Div ─────────────────────

    fn trans_mulw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_arith_w(ir, a, Context::gen_mul)
    }

    fn trans_divw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_div_rem_w(ir, a, false)
    }

    fn trans_divuw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_divu_remu_w(ir, a, false)
    }

    fn trans_remw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_div_rem_w(ir, a, true)
    }

    fn trans_remuw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_divu_remu_w(ir, a, true)
    }
}

// ── Decode16 trait implementation (RVC) ───────────────────────
//
// Most compressed instructions map directly to their 32-bit
// equivalents, so we delegate to the Decode impl.

impl Decode16<Context> for RiscvDisasContext {
    fn trans_illegal(&mut self, _ir: &mut Context, _a: &ArgsEmpty) -> bool {
        false
    }

    fn trans_c64_illegal(&mut self, _ir: &mut Context, _a: &ArgsEmpty) -> bool {
        false
    }

    fn trans_addi(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        <Self as Decode<Context>>::trans_addi(self, ir, a)
    }

    fn trans_lw(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        <Self as Decode<Context>>::trans_lw(self, ir, a)
    }

    fn trans_ld(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        <Self as Decode<Context>>::trans_ld(self, ir, a)
    }

    fn trans_sw(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        <Self as Decode<Context>>::trans_sw(self, ir, a)
    }

    fn trans_sd(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        <Self as Decode<Context>>::trans_sd(self, ir, a)
    }

    fn trans_lui(&mut self, ir: &mut Context, a: &ArgsU) -> bool {
        <Self as Decode<Context>>::trans_lui(self, ir, a)
    }

    fn trans_srli(&mut self, ir: &mut Context, a: &ArgsShift) -> bool {
        <Self as Decode<Context>>::trans_srli(self, ir, a)
    }

    fn trans_srai(&mut self, ir: &mut Context, a: &ArgsShift) -> bool {
        <Self as Decode<Context>>::trans_srai(self, ir, a)
    }

    fn trans_andi(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        <Self as Decode<Context>>::trans_andi(self, ir, a)
    }

    fn trans_sub(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        <Self as Decode<Context>>::trans_sub(self, ir, a)
    }

    fn trans_xor(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        <Self as Decode<Context>>::trans_xor(self, ir, a)
    }

    fn trans_or(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        <Self as Decode<Context>>::trans_or(self, ir, a)
    }

    fn trans_and(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        <Self as Decode<Context>>::trans_and(self, ir, a)
    }

    fn trans_jal(&mut self, ir: &mut Context, a: &ArgsJ) -> bool {
        <Self as Decode<Context>>::trans_jal(self, ir, a)
    }

    fn trans_beq(&mut self, ir: &mut Context, a: &ArgsB) -> bool {
        <Self as Decode<Context>>::trans_beq(self, ir, a)
    }

    fn trans_bne(&mut self, ir: &mut Context, a: &ArgsB) -> bool {
        <Self as Decode<Context>>::trans_bne(self, ir, a)
    }

    fn trans_addiw(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        <Self as Decode<Context>>::trans_addiw(self, ir, a)
    }

    fn trans_subw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        <Self as Decode<Context>>::trans_subw(self, ir, a)
    }

    fn trans_addw(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        <Self as Decode<Context>>::trans_addw(self, ir, a)
    }

    fn trans_slli(&mut self, ir: &mut Context, a: &ArgsShift) -> bool {
        <Self as Decode<Context>>::trans_slli(self, ir, a)
    }

    fn trans_jalr(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        <Self as Decode<Context>>::trans_jalr(self, ir, a)
    }

    fn trans_ebreak(&mut self, ir: &mut Context, a: &ArgsEmpty) -> bool {
        <Self as Decode<Context>>::trans_ebreak(self, ir, a)
    }

    fn trans_add(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        <Self as Decode<Context>>::trans_add(self, ir, a)
    }
}
