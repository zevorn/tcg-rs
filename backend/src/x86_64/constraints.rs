use crate::constraint::*;
use crate::x86_64::regs::{Reg, ALLOCATABLE_REGS};
use tcg_core::Opcode;

const R: tcg_core::RegSet = ALLOCATABLE_REGS;

/// Return the static register constraint for an opcode on
/// x86-64.
///
/// Mirrors QEMU's `tcg_target_op_def()` in
/// `tcg/i386/tcg-target.c.inc`.
pub fn op_constraint(opc: Opcode) -> &'static OpConstraint {
    match opc {
        // -- Three-address via LEA --
        Opcode::Add => {
            static C: OpConstraint = o1_i2(R, R, R);
            &C
        }
        // -- Destructive binary (output aliases input 0) --
        Opcode::Sub | Opcode::Mul | Opcode::And | Opcode::Or | Opcode::Xor => {
            static C: OpConstraint = o1_i2_alias(R, R, R);
            &C
        }
        // -- Destructive unary (output aliases input 0) --
        Opcode::Neg | Opcode::Not => {
            static C: OpConstraint = o1_i1_alias(R, R);
            &C
        }
        // -- Shifts: output aliases input 0, count in RCX --
        Opcode::Shl | Opcode::Shr | Opcode::Sar => {
            static C: OpConstraint = o1_i2_alias_fixed(R, R, Reg::Rcx as u8);
            &C
        }
        // -- SetCond: newreg output (setcc writes low byte) --
        Opcode::SetCond => {
            static C: OpConstraint = n1_i2(R, R, R);
            &C
        }
        // -- BrCond: no outputs --
        Opcode::BrCond => {
            static C: OpConstraint = o0_i2(R, R);
            &C
        }
        // -- Load: output, base input --
        Opcode::Ld
        | Opcode::Ld8U
        | Opcode::Ld8S
        | Opcode::Ld16U
        | Opcode::Ld16S
        | Opcode::Ld32U
        | Opcode::Ld32S => {
            static C: OpConstraint = o1_i1(R, R);
            &C
        }
        // -- Store: value input, base input --
        Opcode::St | Opcode::St8 | Opcode::St16 | Opcode::St32 => {
            static C: OpConstraint = o0_i2(R, R);
            &C
        }
        // -- Type conversions: output, input --
        Opcode::ExtI32I64 | Opcode::ExtUI32I64 | Opcode::ExtrlI64I32 => {
            static C: OpConstraint = o1_i1(R, R);
            &C
        }
        _ => &OpConstraint::EMPTY,
    }
}
