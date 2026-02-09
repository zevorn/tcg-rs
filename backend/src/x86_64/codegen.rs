use crate::code_buffer::CodeBuffer;
use crate::constraint::OpConstraint;
use crate::x86_64::emitter::*;
use crate::x86_64::regs::{
    Reg, CALLEE_SAVED, CALL_ARG_REGS, STACK_ADDEND, STATIC_CALL_ARGS_SIZE,
};
use crate::HostCodeGen;
use tcg_core::{Cond, Context, Op, Opcode, Type};

impl HostCodeGen for X86_64CodeGen {
    fn op_constraint(&self, opc: Opcode) -> &'static OpConstraint {
        crate::x86_64::constraints::op_constraint(opc)
    }

    fn emit_prologue(&mut self, buf: &mut CodeBuffer) {
        self.prologue_offset = buf.offset();
        for &reg in CALLEE_SAVED {
            emit_push(buf, reg);
        }
        // mov TCG_AREG0 (rbp), rdi
        emit_mov_rr(buf, true, Reg::Rbp, CALL_ARG_REGS[0]);
        // sub rsp, STACK_ADDEND
        emit_arith_ri(buf, ArithOp::Sub, true, Reg::Rsp, STACK_ADDEND as i32);
        // jmp *rsi (TB code pointer)
        emit_jmp_reg(buf, CALL_ARG_REGS[1]);
        self.code_gen_start = buf.offset();
    }

    fn emit_epilogue(&mut self, buf: &mut CodeBuffer) {
        self.epilogue_return_zero_offset = buf.offset();
        emit_mov_ri(buf, false, Reg::Rax, 0);
        self.tb_ret_offset = buf.offset();
        emit_arith_ri(buf, ArithOp::Add, true, Reg::Rsp, STACK_ADDEND as i32);
        for &reg in CALLEE_SAVED.iter().rev() {
            emit_pop(buf, reg);
        }
        emit_ret(buf);
    }

    fn patch_jump(
        &mut self,
        buf: &mut CodeBuffer,
        jump_offset: usize,
        target_offset: usize,
    ) {
        let disp = (target_offset as i64) - (jump_offset as i64 + 5);
        assert!(
            (i32::MIN as i64..=i32::MAX as i64).contains(&disp),
            "jump displacement out of i32 range"
        );
        buf.patch_u32(jump_offset + 1, disp as u32);
    }

    fn epilogue_offset(&self) -> usize {
        self.tb_ret_offset
    }

    fn init_context(&self, ctx: &mut tcg_core::Context) {
        use crate::x86_64::regs;
        ctx.reserved_regs = regs::RESERVED_REGS;
        ctx.set_frame(
            Reg::Rsp as u8,
            STATIC_CALL_ARGS_SIZE as i64,
            (regs::CPU_TEMP_BUF_NLONGS * 8) as i64,
        );
    }

    fn tcg_out_mov(&self, buf: &mut CodeBuffer, ty: Type, dst: u8, src: u8) {
        if dst == src {
            return;
        }
        let rexw = ty == Type::I64;
        emit_mov_rr(buf, rexw, Reg::from_u8(dst), Reg::from_u8(src));
    }

    fn tcg_out_movi(&self, buf: &mut CodeBuffer, ty: Type, dst: u8, val: u64) {
        let rexw = ty == Type::I64;
        emit_mov_ri(buf, rexw, Reg::from_u8(dst), val);
    }

    fn tcg_out_ld(
        &self,
        buf: &mut CodeBuffer,
        ty: Type,
        dst: u8,
        base: u8,
        offset: i64,
    ) {
        let rexw = ty == Type::I64;
        emit_load(
            buf,
            rexw,
            Reg::from_u8(dst),
            Reg::from_u8(base),
            offset as i32,
        );
    }

    fn tcg_out_st(
        &self,
        buf: &mut CodeBuffer,
        ty: Type,
        src: u8,
        base: u8,
        offset: i64,
    ) {
        let rexw = ty == Type::I64;
        emit_store(
            buf,
            rexw,
            Reg::from_u8(src),
            Reg::from_u8(base),
            offset as i32,
        );
    }

    fn tcg_out_op(
        &self,
        buf: &mut CodeBuffer,
        ctx: &Context,
        op: &Op,
        oregs: &[u8],
        iregs: &[u8],
        cargs: &[u32],
    ) {
        let rexw = op.op_type == Type::I64;
        match op.opc {
            Opcode::Add => {
                let d = Reg::from_u8(oregs[0]);
                let a = Reg::from_u8(iregs[0]);
                let b = Reg::from_u8(iregs[1]);
                if oregs[0] == iregs[0] {
                    emit_arith_rr(buf, ArithOp::Add, rexw, d, b);
                } else if oregs[0] == iregs[1] {
                    emit_arith_rr(buf, ArithOp::Add, rexw, d, a);
                } else {
                    emit_lea_sib(buf, rexw, d, a, b, 0, 0);
                }
            }
            // Constraints guarantee oregs[0] == iregs[0]
            Opcode::Sub => {
                let d = Reg::from_u8(oregs[0]);
                let b = Reg::from_u8(iregs[1]);
                emit_arith_rr(buf, ArithOp::Sub, rexw, d, b);
            }
            Opcode::Mul => {
                let d = Reg::from_u8(oregs[0]);
                let b = Reg::from_u8(iregs[1]);
                emit_imul_rr(buf, rexw, d, b);
            }
            Opcode::And => {
                let d = Reg::from_u8(oregs[0]);
                let b = Reg::from_u8(iregs[1]);
                emit_arith_rr(buf, ArithOp::And, rexw, d, b);
            }
            Opcode::Or => {
                let d = Reg::from_u8(oregs[0]);
                let b = Reg::from_u8(iregs[1]);
                emit_arith_rr(buf, ArithOp::Or, rexw, d, b);
            }
            Opcode::Xor => {
                let d = Reg::from_u8(oregs[0]);
                let b = Reg::from_u8(iregs[1]);
                emit_arith_rr(buf, ArithOp::Xor, rexw, d, b);
            }
            Opcode::Neg => {
                emit_neg(buf, rexw, Reg::from_u8(oregs[0]));
            }
            Opcode::Not => {
                emit_not(buf, rexw, Reg::from_u8(oregs[0]));
            }
            // Constraints guarantee oregs[0] == iregs[0]
            // and iregs[1] == RCX.
            Opcode::Shl | Opcode::Shr | Opcode::Sar => {
                let d = Reg::from_u8(oregs[0]);
                let sop = match op.opc {
                    Opcode::Shl => ShiftOp::Shl,
                    Opcode::Shr => ShiftOp::Shr,
                    Opcode::Sar => ShiftOp::Sar,
                    _ => unreachable!(),
                };
                emit_shift_cl(buf, sop, rexw, d);
            }
            Opcode::SetCond => {
                let d = Reg::from_u8(oregs[0]);
                let a = Reg::from_u8(iregs[0]);
                let b = Reg::from_u8(iregs[1]);
                let cond = cond_from_u32(cargs[0]);
                let x86c = X86Cond::from_tcg(cond);
                if cond.is_tst() {
                    emit_test_rr(buf, rexw, a, b);
                } else {
                    emit_arith_rr(buf, ArithOp::Cmp, rexw, a, b);
                }
                emit_setcc(buf, x86c, d);
                emit_movzx(buf, OPC_MOVZBL | P_REXB_RM, d, d);
            }
            Opcode::BrCond => {
                let a = Reg::from_u8(iregs[0]);
                let b = Reg::from_u8(iregs[1]);
                let cond = cond_from_u32(cargs[0]);
                let label_id = cargs[1];
                let x86c = X86Cond::from_tcg(cond);
                if cond.is_tst() {
                    emit_test_rr(buf, rexw, a, b);
                } else {
                    emit_arith_rr(buf, ArithOp::Cmp, rexw, a, b);
                }
                let label = ctx.label(label_id);
                if label.has_value {
                    emit_jcc(buf, x86c, label.value);
                } else {
                    emit_opc(buf, OPC_JCC_long + (x86c as u32), 0, 0);
                    buf.emit_u32(0);
                }
            }
            Opcode::Ld => {
                let d = Reg::from_u8(oregs[0]);
                let base = Reg::from_u8(iregs[0]);
                let offset = cargs[0] as i32;
                emit_load(buf, rexw, d, base, offset);
            }
            Opcode::St => {
                let src = Reg::from_u8(iregs[0]);
                let base = Reg::from_u8(iregs[1]);
                let offset = cargs[0] as i32;
                emit_store(buf, rexw, src, base, offset);
            }
            Opcode::ExitTb => {
                let val = cargs[0] as u64;
                self.emit_exit_tb(buf, val);
            }
            Opcode::GotoTb => {
                self.emit_goto_tb(buf);
            }
            _ => {
                panic!("tcg_out_op: unhandled {:?}", op.opc,);
            }
        }
    }
}

fn cond_from_u32(val: u32) -> Cond {
    match val {
        0 => Cond::Never,
        1 => Cond::Always,
        8 => Cond::Eq,
        9 => Cond::Ne,
        10 => Cond::Lt,
        11 => Cond::Ge,
        12 => Cond::Le,
        13 => Cond::Gt,
        14 => Cond::Ltu,
        15 => Cond::Geu,
        16 => Cond::Leu,
        17 => Cond::Gtu,
        18 => Cond::TstEq,
        19 => Cond::TstNe,
        _ => panic!("invalid Cond value: {val}"),
    }
}
