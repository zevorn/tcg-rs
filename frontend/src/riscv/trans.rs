//! RISC-V instruction translation — TCG IR generation.
//!
//! Follows QEMU's gen_xxx helper pattern: repetitive instruction
//! translation logic is factored into gen_arith, gen_arith_imm,
//! gen_shift_imm, gen_shiftw, etc., each parameterised by a
//! `BinOp` function pointer.

use super::cpu::{
    fpr_offset, FFLAGS_OFFSET, FRM_OFFSET, UCAUSE_OFFSET, UEPC_OFFSET,
    UIE_OFFSET, UIP_OFFSET, USCRATCH_OFFSET, USTATUS_FS_DIRTY,
    USTATUS_FS_MASK, USTATUS_OFFSET, UTVEC_OFFSET, UTVAL_OFFSET,
};
use super::fpu;
use super::insn_decode::*;
use super::RiscvDisasContext;
use crate::DisasJumpType;
use tcg_core::context::Context;
use tcg_core::types::{Cond, MemOp, Type};
use tcg_core::TempIdx;

/// Binary IR operation: `fn(ir, ty, dst, lhs, rhs) -> dst`.
type BinOp = fn(&mut Context, Type, TempIdx, TempIdx, TempIdx) -> TempIdx;

// Memory barrier constants (QEMU TCG_MO_* / TCG_BAR_*).
const TCG_MO_ALL: u32 = 0x0F;
const TCG_BAR_LDAQ: u32 = 0x10;
const TCG_BAR_STRL: u32 = 0x20;

// CSR numbers (user-level).
const CSR_USTATUS: i64 = 0x000;
const CSR_FFLAGS: i64 = 0x001;
const CSR_FRM: i64 = 0x002;
const CSR_FCSR: i64 = 0x003;
const CSR_UIE: i64 = 0x004;
const CSR_UTVEC: i64 = 0x005;
const CSR_USCRATCH: i64 = 0x040;
const CSR_UEPC: i64 = 0x041;
const CSR_UCAUSE: i64 = 0x042;
const CSR_UTVAL: i64 = 0x043;
const CSR_UIP: i64 = 0x044;
const CSR_CYCLE: i64 = 0xC00;
const CSR_TIME: i64 = 0xC01;
const CSR_INSTRET: i64 = 0xC02;

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

    // -- FPR access ----------------------------------------

    fn fpr_load(&self, ir: &mut Context, idx: i64) -> TempIdx {
        let t = ir.new_temp(Type::I64);
        ir.gen_ld(Type::I64, t, self.env, fpr_offset(idx as usize));
        t
    }

    fn fpr_store(&self, ir: &mut Context, idx: i64, val: TempIdx) {
        ir.gen_st(Type::I64, val, self.env, fpr_offset(idx as usize));
    }

    // -- FP state helpers -----------------------------------

    fn gen_fp_check(&self, ir: &mut Context) {
        let status = ir.new_temp(Type::I64);
        ir.gen_ld(Type::I64, status, self.env, USTATUS_OFFSET);
        let mask = ir.new_const(Type::I64, USTATUS_FS_MASK);
        let fs = ir.new_temp(Type::I64);
        ir.gen_and(Type::I64, fs, status, mask);
        let zero = ir.new_const(Type::I64, 0);
        let ok = ir.new_label();
        ir.gen_brcond(Type::I64, fs, zero, Cond::Ne, ok);
        let pc = ir.new_const(Type::I64, self.base.pc_next);
        ir.gen_mov(Type::I64, self.pc, pc);
        ir.gen_exit_tb(3);
        ir.gen_set_label(ok);
    }

    fn gen_set_fs_dirty(&self, ir: &mut Context) {
        let status = ir.new_temp(Type::I64);
        ir.gen_ld(Type::I64, status, self.env, USTATUS_OFFSET);
        let clear = ir.new_const(Type::I64, !USTATUS_FS_MASK);
        let cleared = ir.new_temp(Type::I64);
        ir.gen_and(Type::I64, cleared, status, clear);
        let dirty = ir.new_const(Type::I64, USTATUS_FS_DIRTY);
        let new_status = ir.new_temp(Type::I64);
        ir.gen_or(Type::I64, new_status, cleared, dirty);
        ir.gen_st(Type::I64, new_status, self.env, USTATUS_OFFSET);
    }

    fn gen_helper_call(
        &self,
        ir: &mut Context,
        helper: usize,
        args: &[TempIdx],
    ) -> TempIdx {
        let dst = ir.new_temp(Type::I64);
        ir.gen_call(dst, helper as u64, args);
        dst
    }

    fn gen_fp_load(
        &self,
        ir: &mut Context,
        a: &ArgsI,
        memop: MemOp,
        is_single: bool,
    ) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let base = self.gpr_or_zero(ir, a.rs1);
        let addr = if a.imm != 0 {
            let imm = ir.new_const(Type::I64, a.imm as u64);
            let t = ir.new_temp(Type::I64);
            ir.gen_add(Type::I64, t, base, imm)
        } else {
            base
        };
        let val = ir.new_temp(Type::I64);
        ir.gen_qemu_ld(Type::I64, val, addr, memop.bits() as u32);
        if is_single {
            let mask = ir.new_const(Type::I64, 0xffff_ffff_0000_0000u64);
            let boxed = ir.new_temp(Type::I64);
            ir.gen_or(Type::I64, boxed, val, mask);
            self.fpr_store(ir, a.rd, boxed);
        } else {
            self.fpr_store(ir, a.rd, val);
        }
        true
    }

    fn gen_fp_store(
        &self,
        ir: &mut Context,
        a: &ArgsS,
        memop: MemOp,
        is_single: bool,
    ) -> bool {
        self.gen_fp_check(ir);
        let base = self.gpr_or_zero(ir, a.rs1);
        let addr = if a.imm != 0 {
            let imm = ir.new_const(Type::I64, a.imm as u64);
            let t = ir.new_temp(Type::I64);
            ir.gen_add(Type::I64, t, base, imm)
        } else {
            base
        };
        let val = self.fpr_load(ir, a.rs2);
        let store_val = if is_single {
            let lo32 = ir.new_temp(Type::I32);
            ir.gen_extrl_i64_i32(lo32, val);
            let lo64 = ir.new_temp(Type::I64);
            ir.gen_ext_i32_i64(lo64, lo32);
            lo64
        } else {
            val
        };
        ir.gen_qemu_st(Type::I64, store_val, addr, memop.bits() as u32);
        true
    }

    // -- CSR helpers ----------------------------------------

    fn gen_csr_read(&self, ir: &mut Context, csr: i64) -> Option<TempIdx> {
        match csr {
            CSR_FFLAGS => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, FFLAGS_OFFSET);
                let mask = ir.new_const(Type::I64, fpu::FFLAGS_MASK);
                let out = ir.new_temp(Type::I64);
                ir.gen_and(Type::I64, out, v, mask);
                Some(out)
            }
            CSR_FRM => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, FRM_OFFSET);
                let mask = ir.new_const(Type::I64, fpu::FRM_MASK);
                let out = ir.new_temp(Type::I64);
                ir.gen_and(Type::I64, out, v, mask);
                Some(out)
            }
            CSR_FCSR => {
                let fflags = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, fflags, self.env, FFLAGS_OFFSET);
                let fmask = ir.new_const(Type::I64, fpu::FFLAGS_MASK);
                ir.gen_and(Type::I64, fflags, fflags, fmask);
                let frm = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, frm, self.env, FRM_OFFSET);
                let rmask = ir.new_const(Type::I64, fpu::FRM_MASK);
                ir.gen_and(Type::I64, frm, frm, rmask);
                let shift = ir.new_const(Type::I64, 5);
                let frm_shift = ir.new_temp(Type::I64);
                ir.gen_shl(Type::I64, frm_shift, frm, shift);
                let out = ir.new_temp(Type::I64);
                ir.gen_or(Type::I64, out, fflags, frm_shift);
                Some(out)
            }
            CSR_USTATUS => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, USTATUS_OFFSET);
                Some(v)
            }
            CSR_UIE => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, UIE_OFFSET);
                Some(v)
            }
            CSR_UTVEC => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, UTVEC_OFFSET);
                Some(v)
            }
            CSR_USCRATCH => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, USCRATCH_OFFSET);
                Some(v)
            }
            CSR_UEPC => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, UEPC_OFFSET);
                Some(v)
            }
            CSR_UCAUSE => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, UCAUSE_OFFSET);
                Some(v)
            }
            CSR_UTVAL => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, UTVAL_OFFSET);
                Some(v)
            }
            CSR_UIP => {
                let v = ir.new_temp(Type::I64);
                ir.gen_ld(Type::I64, v, self.env, UIP_OFFSET);
                Some(v)
            }
            CSR_CYCLE | CSR_TIME | CSR_INSTRET => {
                let v = ir.new_const(Type::I64, 0);
                Some(v)
            }
            _ => None,
        }
    }

    fn gen_csr_write(
        &self,
        ir: &mut Context,
        csr: i64,
        val: TempIdx,
    ) -> bool {
        match csr {
            CSR_FFLAGS => {
                let mask = ir.new_const(Type::I64, fpu::FFLAGS_MASK);
                let v = ir.new_temp(Type::I64);
                ir.gen_and(Type::I64, v, val, mask);
                ir.gen_st(Type::I64, v, self.env, FFLAGS_OFFSET);
                self.gen_set_fs_dirty(ir);
                true
            }
            CSR_FRM => {
                let mask = ir.new_const(Type::I64, fpu::FRM_MASK);
                let v = ir.new_temp(Type::I64);
                ir.gen_and(Type::I64, v, val, mask);
                ir.gen_st(Type::I64, v, self.env, FRM_OFFSET);
                self.gen_set_fs_dirty(ir);
                true
            }
            CSR_FCSR => {
                let fmask = ir.new_const(Type::I64, fpu::FFLAGS_MASK);
                let fflags = ir.new_temp(Type::I64);
                ir.gen_and(Type::I64, fflags, val, fmask);
                ir.gen_st(Type::I64, fflags, self.env, FFLAGS_OFFSET);
                let shift = ir.new_const(Type::I64, 5);
                let frm = ir.new_temp(Type::I64);
                ir.gen_shr(Type::I64, frm, val, shift);
                let rmask = ir.new_const(Type::I64, fpu::FRM_MASK);
                ir.gen_and(Type::I64, frm, frm, rmask);
                ir.gen_st(Type::I64, frm, self.env, FRM_OFFSET);
                self.gen_set_fs_dirty(ir);
                true
            }
            CSR_USTATUS => {
                ir.gen_st(Type::I64, val, self.env, USTATUS_OFFSET);
                true
            }
            CSR_UIE => {
                ir.gen_st(Type::I64, val, self.env, UIE_OFFSET);
                true
            }
            CSR_UTVEC => {
                ir.gen_st(Type::I64, val, self.env, UTVEC_OFFSET);
                true
            }
            CSR_USCRATCH => {
                ir.gen_st(Type::I64, val, self.env, USCRATCH_OFFSET);
                true
            }
            CSR_UEPC => {
                ir.gen_st(Type::I64, val, self.env, UEPC_OFFSET);
                true
            }
            CSR_UCAUSE => {
                ir.gen_st(Type::I64, val, self.env, UCAUSE_OFFSET);
                true
            }
            CSR_UTVAL => {
                ir.gen_st(Type::I64, val, self.env, UTVAL_OFFSET);
                true
            }
            CSR_UIP => {
                ir.gen_st(Type::I64, val, self.env, UIP_OFFSET);
                true
            }
            CSR_CYCLE | CSR_TIME | CSR_INSTRET => false,
            _ => false,
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

    // -- Atomic helpers (A extension) ----------------------

    /// LR: load-reserved.
    fn gen_lr(&self, ir: &mut Context, a: &ArgsAtomic, memop: MemOp) -> bool {
        let addr = self.gpr_or_zero(ir, a.rs1);
        if a.rl != 0 {
            ir.gen_mb(TCG_MO_ALL | TCG_BAR_STRL);
        }
        let val = ir.new_temp(Type::I64);
        ir.gen_qemu_ld(Type::I64, val, addr, memop.bits() as u32);
        if a.aq != 0 {
            ir.gen_mb(TCG_MO_ALL | TCG_BAR_LDAQ);
        }
        ir.gen_mov(Type::I64, self.load_res, addr);
        ir.gen_mov(Type::I64, self.load_val, val);
        self.gen_set_gpr(ir, a.rd, val);
        true
    }

    /// SC: store-conditional (single-thread simplified).
    ///
    /// In single-threaded mode, SC always succeeds if there
    /// is a valid reservation (set by a preceding LR).
    /// We skip the address comparison since no other thread
    /// can invalidate the reservation.
    fn gen_sc(&self, ir: &mut Context, a: &ArgsAtomic, memop: MemOp) -> bool {
        let addr = self.gpr_or_zero(ir, a.rs1);

        // Always succeed: store and set rd = 0.
        let src2 = self.gpr_or_zero(ir, a.rs2);
        ir.gen_qemu_st(Type::I64, src2, addr, memop.bits() as u32);
        let zero = ir.new_const(Type::I64, 0);
        self.gen_set_gpr(ir, a.rd, zero);

        // Clear reservation.
        let neg1 = ir.new_const(Type::I64, u64::MAX);
        ir.gen_mov(Type::I64, self.load_res, neg1);
        true
    }

    /// AMO: atomic read-modify-write (single-thread: ld+op+st).
    fn gen_amo(
        &self,
        ir: &mut Context,
        a: &ArgsAtomic,
        op: BinOp,
        memop: MemOp,
    ) -> bool {
        let addr = self.gpr_or_zero(ir, a.rs1);
        if a.rl != 0 {
            ir.gen_mb(TCG_MO_ALL | TCG_BAR_STRL);
        }
        let old = ir.new_temp(Type::I64);
        ir.gen_qemu_ld(Type::I64, old, addr, memop.bits() as u32);
        let src2 = self.gpr_or_zero(ir, a.rs2);
        let new = ir.new_temp(Type::I64);
        op(ir, Type::I64, new, old, src2);
        ir.gen_qemu_st(Type::I64, new, addr, memop.bits() as u32);
        if a.aq != 0 {
            ir.gen_mb(TCG_MO_ALL | TCG_BAR_LDAQ);
        }
        self.gen_set_gpr(ir, a.rd, old);
        true
    }

    /// AMO swap: store rs2, return old value.
    fn gen_amo_swap(
        &self,
        ir: &mut Context,
        a: &ArgsAtomic,
        memop: MemOp,
    ) -> bool {
        let addr = self.gpr_or_zero(ir, a.rs1);
        if a.rl != 0 {
            ir.gen_mb(TCG_MO_ALL | TCG_BAR_STRL);
        }
        let old = ir.new_temp(Type::I64);
        ir.gen_qemu_ld(Type::I64, old, addr, memop.bits() as u32);
        let src2 = self.gpr_or_zero(ir, a.rs2);
        ir.gen_qemu_st(Type::I64, src2, addr, memop.bits() as u32);
        if a.aq != 0 {
            ir.gen_mb(TCG_MO_ALL | TCG_BAR_LDAQ);
        }
        self.gen_set_gpr(ir, a.rd, old);
        true
    }

    /// AMO min/max: conditional select via movcond.
    fn gen_amo_minmax(
        &self,
        ir: &mut Context,
        a: &ArgsAtomic,
        cond: Cond,
        memop: MemOp,
    ) -> bool {
        let addr = self.gpr_or_zero(ir, a.rs1);
        if a.rl != 0 {
            ir.gen_mb(TCG_MO_ALL | TCG_BAR_STRL);
        }
        let old = ir.new_temp(Type::I64);
        ir.gen_qemu_ld(Type::I64, old, addr, memop.bits() as u32);
        let src2 = self.gpr_or_zero(ir, a.rs2);
        let new = ir.new_temp(Type::I64);
        // new = (old cond src2) ? old : src2
        ir.gen_movcond(Type::I64, new, old, src2, old, src2, cond);
        ir.gen_qemu_st(Type::I64, new, addr, memop.bits() as u32);
        if a.aq != 0 {
            ir.gen_mb(TCG_MO_ALL | TCG_BAR_LDAQ);
        }
        self.gen_set_gpr(ir, a.rd, old);
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

    // ── RV32A: Atomic ─────────────────────────────────────

    fn trans_lr_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_lr(ir, a, MemOp::sl())
    }
    fn trans_sc_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_sc(ir, a, MemOp::ul())
    }
    fn trans_amoswap_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_swap(ir, a, MemOp::sl())
    }
    fn trans_amoadd_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo(ir, a, Context::gen_add, MemOp::sl())
    }
    fn trans_amoxor_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo(ir, a, Context::gen_xor, MemOp::sl())
    }
    fn trans_amoand_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo(ir, a, Context::gen_and, MemOp::sl())
    }
    fn trans_amoor_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo(ir, a, Context::gen_or, MemOp::sl())
    }
    fn trans_amomin_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_minmax(ir, a, Cond::Lt, MemOp::sl())
    }
    fn trans_amomax_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_minmax(ir, a, Cond::Gt, MemOp::sl())
    }
    fn trans_amominu_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_minmax(ir, a, Cond::Ltu, MemOp::sl())
    }
    fn trans_amomaxu_w(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_minmax(ir, a, Cond::Gtu, MemOp::sl())
    }

    // ── RV64A: Atomic ─────────────────────────────────────

    fn trans_lr_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_lr(ir, a, MemOp::uq())
    }
    fn trans_sc_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_sc(ir, a, MemOp::uq())
    }
    fn trans_amoswap_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_swap(ir, a, MemOp::uq())
    }
    fn trans_amoadd_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo(ir, a, Context::gen_add, MemOp::uq())
    }
    fn trans_amoxor_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo(ir, a, Context::gen_xor, MemOp::uq())
    }
    fn trans_amoand_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo(ir, a, Context::gen_and, MemOp::uq())
    }
    fn trans_amoor_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo(ir, a, Context::gen_or, MemOp::uq())
    }
    fn trans_amomin_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_minmax(ir, a, Cond::Lt, MemOp::uq())
    }
    fn trans_amomax_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_minmax(ir, a, Cond::Gt, MemOp::uq())
    }
    fn trans_amominu_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_minmax(ir, a, Cond::Ltu, MemOp::uq())
    }
    fn trans_amomaxu_d(&mut self, ir: &mut Context, a: &ArgsAtomic) -> bool {
        self.gen_amo_minmax(ir, a, Cond::Gtu, MemOp::uq())
    }

    // ── Zicsr: CSR access ─────────────────────────────

    fn trans_csrrw(&mut self, ir: &mut Context, a: &ArgsCsr) -> bool {
        let old = match self.gen_csr_read(ir, a.csr) {
            Some(v) => v,
            None => return false,
        };
        let rs1 = self.gpr_or_zero(ir, a.rs1);
        if !self.gen_csr_write(ir, a.csr, rs1) {
            return false;
        }
        self.gen_set_gpr(ir, a.rd, old);
        true
    }

    fn trans_csrrs(&mut self, ir: &mut Context, a: &ArgsCsr) -> bool {
        let old = match self.gen_csr_read(ir, a.csr) {
            Some(v) => v,
            None => return false,
        };
        if a.rs1 != 0 {
            let rs1 = self.gpr_or_zero(ir, a.rs1);
            let new = ir.new_temp(Type::I64);
            ir.gen_or(Type::I64, new, old, rs1);
            if !self.gen_csr_write(ir, a.csr, new) {
                return false;
            }
        }
        self.gen_set_gpr(ir, a.rd, old);
        true
    }

    fn trans_csrrc(&mut self, ir: &mut Context, a: &ArgsCsr) -> bool {
        let old = match self.gen_csr_read(ir, a.csr) {
            Some(v) => v,
            None => return false,
        };
        if a.rs1 != 0 {
            let rs1 = self.gpr_or_zero(ir, a.rs1);
            let inv = ir.new_temp(Type::I64);
            ir.gen_not(Type::I64, inv, rs1);
            let new = ir.new_temp(Type::I64);
            ir.gen_and(Type::I64, new, old, inv);
            if !self.gen_csr_write(ir, a.csr, new) {
                return false;
            }
        }
        self.gen_set_gpr(ir, a.rd, old);
        true
    }

    fn trans_csrrwi(&mut self, ir: &mut Context, a: &ArgsCsr) -> bool {
        let old = match self.gen_csr_read(ir, a.csr) {
            Some(v) => v,
            None => return false,
        };
        let zimm = ir.new_const(Type::I64, a.rs1 as u64);
        if !self.gen_csr_write(ir, a.csr, zimm) {
            return false;
        }
        self.gen_set_gpr(ir, a.rd, old);
        true
    }

    fn trans_csrrsi(&mut self, ir: &mut Context, a: &ArgsCsr) -> bool {
        let old = match self.gen_csr_read(ir, a.csr) {
            Some(v) => v,
            None => return false,
        };
        if a.rs1 != 0 {
            let zimm = ir.new_const(Type::I64, a.rs1 as u64);
            let new = ir.new_temp(Type::I64);
            ir.gen_or(Type::I64, new, old, zimm);
            if !self.gen_csr_write(ir, a.csr, new) {
                return false;
            }
        }
        self.gen_set_gpr(ir, a.rd, old);
        true
    }

    fn trans_csrrci(&mut self, ir: &mut Context, a: &ArgsCsr) -> bool {
        let old = match self.gen_csr_read(ir, a.csr) {
            Some(v) => v,
            None => return false,
        };
        if a.rs1 != 0 {
            let zimm = ir.new_const(Type::I64, a.rs1 as u64);
            let inv = ir.new_temp(Type::I64);
            ir.gen_not(Type::I64, inv, zimm);
            let new = ir.new_temp(Type::I64);
            ir.gen_and(Type::I64, new, old, inv);
            if !self.gen_csr_write(ir, a.csr, new) {
                return false;
            }
        }
        self.gen_set_gpr(ir, a.rd, old);
        true
    }

    // ── RV32F/RV64F: FP Loads/Stores ──────────────────

    fn trans_flw(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_fp_load(ir, a, MemOp::ul(), true)
    }
    fn trans_fsw(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        self.gen_fp_store(ir, a, MemOp::ul(), true)
    }
    fn trans_fld(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        self.gen_fp_load(ir, a, MemOp::uq(), false)
    }
    fn trans_fsd(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        self.gen_fp_store(ir, a, MemOp::uq(), false)
    }

    // ── RV32F: FMA ────────────────────────────────────

    fn trans_fmadd_s(&mut self, ir: &mut Context, a: &ArgsR4Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rs3 = self.fpr_load(ir, a.rs3);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fmadd_s as usize,
            &[self.env, rs1, rs2, rs3, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fmsub_s(&mut self, ir: &mut Context, a: &ArgsR4Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rs3 = self.fpr_load(ir, a.rs3);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fmsub_s as usize,
            &[self.env, rs1, rs2, rs3, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fnmsub_s(&mut self, ir: &mut Context, a: &ArgsR4Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rs3 = self.fpr_load(ir, a.rs3);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fnmsub_s as usize,
            &[self.env, rs1, rs2, rs3, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fnmadd_s(&mut self, ir: &mut Context, a: &ArgsR4Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rs3 = self.fpr_load(ir, a.rs3);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fnmadd_s as usize,
            &[self.env, rs1, rs2, rs3, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }

    // ── RV32F: Arithmetic ─────────────────────────────

    fn trans_fadd_s(&mut self, ir: &mut Context, a: &ArgsRRm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fadd_s as usize,
            &[self.env, rs1, rs2, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fsub_s(&mut self, ir: &mut Context, a: &ArgsRRm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fsub_s as usize,
            &[self.env, rs1, rs2, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fmul_s(&mut self, ir: &mut Context, a: &ArgsRRm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fmul_s as usize,
            &[self.env, rs1, rs2, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fdiv_s(&mut self, ir: &mut Context, a: &ArgsRRm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fdiv_s as usize,
            &[self.env, rs1, rs2, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fsqrt_s(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fsqrt_s as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }

    fn trans_fsgnj_s(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fsgnj_s as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fsgnjn_s(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fsgnjn_s as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fsgnjx_s(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fsgnjx_s as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fmin_s(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fmin_s as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fmax_s(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fmax_s as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }

    fn trans_feq_s(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_feq_s as usize, &[self.env, rs1, rs2]);
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_flt_s(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_flt_s as usize, &[self.env, rs1, rs2]);
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fle_s(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fle_s as usize, &[self.env, rs1, rs2]);
        self.gen_set_gpr(ir, a.rd, res);
        true
    }

    fn trans_fclass_s(&mut self, ir: &mut Context, a: &ArgsR2) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let res =
            self.gen_helper_call(ir, fpu::helper_fclass_s as usize, &[self.env, rs1]);
        self.gen_set_gpr(ir, a.rd, res);
        true
    }

    // ── RV32F: Conversions ─────────────────────────────

    fn trans_fcvt_w_s(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_w_s as usize,
            &[self.env, rs1, rm],
        );
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fcvt_wu_s(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_wu_s as usize,
            &[self.env, rs1, rm],
        );
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fcvt_s_w(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.gpr_or_zero(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_s_w as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fcvt_s_wu(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.gpr_or_zero(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_s_wu as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }

    fn trans_fmv_x_w(&mut self, ir: &mut Context, a: &ArgsR2) -> bool {
        self.gen_fp_check(ir);
        let val = self.fpr_load(ir, a.rs1);
        let lo32 = ir.new_temp(Type::I32);
        ir.gen_extrl_i64_i32(lo32, val);
        self.gen_set_gpr_sx32(ir, a.rd, lo32);
        true
    }
    fn trans_fmv_w_x(&mut self, ir: &mut Context, a: &ArgsR2) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let src = self.gpr_or_zero(ir, a.rs1);
        let lo32 = ir.new_temp(Type::I32);
        ir.gen_extrl_i64_i32(lo32, src);
        let lo64 = ir.new_temp(Type::I64);
        ir.gen_ext_u32_i64(lo64, lo32);
        let mask = ir.new_const(Type::I64, 0xffff_ffff_0000_0000u64);
        let boxed = ir.new_temp(Type::I64);
        ir.gen_or(Type::I64, boxed, lo64, mask);
        self.fpr_store(ir, a.rd, boxed);
        true
    }

    // ── RV64F additions ───────────────────────────────

    fn trans_fcvt_l_s(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_l_s as usize,
            &[self.env, rs1, rm],
        );
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fcvt_lu_s(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_lu_s as usize,
            &[self.env, rs1, rm],
        );
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fcvt_s_l(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.gpr_or_zero(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_s_l as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fcvt_s_lu(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.gpr_or_zero(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_s_lu as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }

    // ── RV32D/RV64D: FMA ──────────────────────────────

    fn trans_fmadd_d(&mut self, ir: &mut Context, a: &ArgsR4Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rs3 = self.fpr_load(ir, a.rs3);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fmadd_d as usize,
            &[self.env, rs1, rs2, rs3, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fmsub_d(&mut self, ir: &mut Context, a: &ArgsR4Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rs3 = self.fpr_load(ir, a.rs3);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fmsub_d as usize,
            &[self.env, rs1, rs2, rs3, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fnmsub_d(&mut self, ir: &mut Context, a: &ArgsR4Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rs3 = self.fpr_load(ir, a.rs3);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fnmsub_d as usize,
            &[self.env, rs1, rs2, rs3, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fnmadd_d(&mut self, ir: &mut Context, a: &ArgsR4Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rs3 = self.fpr_load(ir, a.rs3);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fnmadd_d as usize,
            &[self.env, rs1, rs2, rs3, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }

    // ── RV32D: Arithmetic ─────────────────────────────

    fn trans_fadd_d(&mut self, ir: &mut Context, a: &ArgsRRm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fadd_d as usize,
            &[self.env, rs1, rs2, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fsub_d(&mut self, ir: &mut Context, a: &ArgsRRm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fsub_d as usize,
            &[self.env, rs1, rs2, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fmul_d(&mut self, ir: &mut Context, a: &ArgsRRm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fmul_d as usize,
            &[self.env, rs1, rs2, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fdiv_d(&mut self, ir: &mut Context, a: &ArgsRRm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fdiv_d as usize,
            &[self.env, rs1, rs2, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fsqrt_d(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fsqrt_d as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }

    fn trans_fsgnj_d(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fsgnj_d as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fsgnjn_d(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fsgnjn_d as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fsgnjx_d(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fsgnjx_d as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fmin_d(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fmin_d as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fmax_d(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fmax_d as usize, &[self.env, rs1, rs2]);
        self.fpr_store(ir, a.rd, res);
        true
    }

    fn trans_feq_d(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_feq_d as usize, &[self.env, rs1, rs2]);
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_flt_d(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_flt_d as usize, &[self.env, rs1, rs2]);
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fle_d(&mut self, ir: &mut Context, a: &ArgsR) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rs2 = self.fpr_load(ir, a.rs2);
        let res =
            self.gen_helper_call(ir, fpu::helper_fle_d as usize, &[self.env, rs1, rs2]);
        self.gen_set_gpr(ir, a.rd, res);
        true
    }

    fn trans_fclass_d(&mut self, ir: &mut Context, a: &ArgsR2) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let res =
            self.gen_helper_call(ir, fpu::helper_fclass_d as usize, &[self.env, rs1]);
        self.gen_set_gpr(ir, a.rd, res);
        true
    }

    // ── RV32D: Conversions ─────────────────────────────

    fn trans_fcvt_s_d(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_s_d as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fcvt_d_s(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_d_s as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fcvt_w_d(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_w_d as usize,
            &[self.env, rs1, rm],
        );
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fcvt_wu_d(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_wu_d as usize,
            &[self.env, rs1, rm],
        );
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fcvt_d_w(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.gpr_or_zero(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_d_w as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fcvt_d_wu(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.gpr_or_zero(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_d_wu as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }

    // ── RV64D additions ───────────────────────────────

    fn trans_fcvt_l_d(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_l_d as usize,
            &[self.env, rs1, rm],
        );
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fcvt_lu_d(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        let rs1 = self.fpr_load(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_lu_d as usize,
            &[self.env, rs1, rm],
        );
        self.gen_set_gpr(ir, a.rd, res);
        true
    }
    fn trans_fcvt_d_l(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.gpr_or_zero(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_d_l as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }
    fn trans_fcvt_d_lu(&mut self, ir: &mut Context, a: &ArgsR2Rm) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let rs1 = self.gpr_or_zero(ir, a.rs1);
        let rm = ir.new_const(Type::I64, a.rm as u64);
        let res = self.gen_helper_call(
            ir,
            fpu::helper_fcvt_d_lu as usize,
            &[self.env, rs1, rm],
        );
        self.fpr_store(ir, a.rd, res);
        true
    }

    fn trans_fmv_x_d(&mut self, ir: &mut Context, a: &ArgsR2) -> bool {
        self.gen_fp_check(ir);
        let val = self.fpr_load(ir, a.rs1);
        self.gen_set_gpr(ir, a.rd, val);
        true
    }
    fn trans_fmv_d_x(&mut self, ir: &mut Context, a: &ArgsR2) -> bool {
        self.gen_fp_check(ir);
        self.gen_set_fs_dirty(ir);
        let src = self.gpr_or_zero(ir, a.rs1);
        self.fpr_store(ir, a.rd, src);
        true
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

    fn trans_c_fld(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        <Self as Decode<Context>>::trans_fld(self, ir, a)
    }

    fn trans_c_flw(&mut self, ir: &mut Context, a: &ArgsI) -> bool {
        <Self as Decode<Context>>::trans_flw(self, ir, a)
    }

    fn trans_sw(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        <Self as Decode<Context>>::trans_sw(self, ir, a)
    }

    fn trans_sd(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        <Self as Decode<Context>>::trans_sd(self, ir, a)
    }

    fn trans_c_fsd(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        <Self as Decode<Context>>::trans_fsd(self, ir, a)
    }

    fn trans_c_fsw(&mut self, ir: &mut Context, a: &ArgsS) -> bool {
        <Self as Decode<Context>>::trans_fsw(self, ir, a)
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
