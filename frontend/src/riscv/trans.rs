//! RISC-V instruction translation stubs.
//!
//! Each `trans_*` method will emit TCG IR for the
//! corresponding RISC-V instruction.  Currently stubs
//! that return `true` (decoded OK, no IR yet).

use super::insn_decode::*;
use super::RiscvDisasContext;

impl Decode for RiscvDisasContext {
    fn trans_lui(&mut self, _a: &ArgsU) -> bool {
        true
    }
    fn trans_auipc(&mut self, _a: &ArgsU) -> bool {
        true
    }
    fn trans_jal(&mut self, _a: &ArgsJ) -> bool {
        true
    }
    fn trans_jalr(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_beq(&mut self, _a: &ArgsB) -> bool {
        true
    }
    fn trans_bne(&mut self, _a: &ArgsB) -> bool {
        true
    }
    fn trans_blt(&mut self, _a: &ArgsB) -> bool {
        true
    }
    fn trans_bge(&mut self, _a: &ArgsB) -> bool {
        true
    }
    fn trans_bltu(&mut self, _a: &ArgsB) -> bool {
        true
    }
    fn trans_bgeu(&mut self, _a: &ArgsB) -> bool {
        true
    }
    fn trans_lb(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_lh(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_lw(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_lbu(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_lhu(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_sb(&mut self, _a: &ArgsS) -> bool {
        true
    }
    fn trans_sh(&mut self, _a: &ArgsS) -> bool {
        true
    }
    fn trans_sw(&mut self, _a: &ArgsS) -> bool {
        true
    }
    fn trans_addi(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_slti(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_sltiu(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_xori(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_ori(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_andi(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_slli(&mut self, _a: &ArgsShift) -> bool {
        true
    }
    fn trans_srli(&mut self, _a: &ArgsShift) -> bool {
        true
    }
    fn trans_srai(&mut self, _a: &ArgsShift) -> bool {
        true
    }
    fn trans_add(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_sub(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_sll(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_slt(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_sltu(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_xor(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_srl(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_sra(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_or(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_and(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_fence(
        &mut self,
        _a: &ArgsAutoFence,
    ) -> bool {
        true
    }

    // ── RV64I ──────────────────────────────────────────────

    fn trans_lwu(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_ld(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_sd(&mut self, _a: &ArgsS) -> bool {
        true
    }
    fn trans_addiw(&mut self, _a: &ArgsI) -> bool {
        true
    }
    fn trans_slliw(
        &mut self,
        _a: &ArgsShift,
    ) -> bool {
        true
    }
    fn trans_srliw(
        &mut self,
        _a: &ArgsShift,
    ) -> bool {
        true
    }
    fn trans_sraiw(
        &mut self,
        _a: &ArgsShift,
    ) -> bool {
        true
    }
    fn trans_addw(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_subw(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_sllw(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_srlw(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_sraw(&mut self, _a: &ArgsR) -> bool {
        true
    }

    // ── Privileged ─────────────────────────────────────────

    fn trans_ecall(&mut self, _a: &ArgsEmpty) -> bool {
        true
    }
    fn trans_ebreak(
        &mut self,
        _a: &ArgsEmpty,
    ) -> bool {
        true
    }

    // ── RV32M ──────────────────────────────────────────────

    fn trans_mul(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_mulh(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_mulhsu(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_mulhu(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_div(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_divu(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_rem(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_remu(&mut self, _a: &ArgsR) -> bool {
        true
    }

    // ── RV64M ──────────────────────────────────────────────

    fn trans_mulw(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_divw(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_divuw(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_remw(&mut self, _a: &ArgsR) -> bool {
        true
    }
    fn trans_remuw(&mut self, _a: &ArgsR) -> bool {
        true
    }
}