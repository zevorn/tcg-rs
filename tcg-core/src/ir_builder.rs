use crate::context::Context;
use crate::op::Op;
use crate::opcode::Opcode;
use crate::temp::TempIdx;
use crate::types::{Cond, Type};

// Constant args are encoded as TempIdx(raw_value as u32).
fn carg(val: u32) -> TempIdx {
    TempIdx(val)
}

impl Context {
    // -- Internal helpers --

    fn emit_binary(
        &mut self,
        opc: Opcode,
        ty: Type,
        dst: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, opc, ty, &[dst, a, b]);
        self.emit_op(op);
        dst
    }

    fn emit_unary(
        &mut self,
        opc: Opcode,
        ty: Type,
        dst: TempIdx,
        src: TempIdx,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, opc, ty, &[dst, src]);
        self.emit_op(op);
        dst
    }

    // -- Binary ALU (1 oarg, 2 iargs) --

    pub fn gen_add(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Add, ty, d, a, b)
    }

    pub fn gen_sub(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Sub, ty, d, a, b)
    }

    pub fn gen_mul(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Mul, ty, d, a, b)
    }

    pub fn gen_and(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::And, ty, d, a, b)
    }

    pub fn gen_or(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Or, ty, d, a, b)
    }

    pub fn gen_xor(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Xor, ty, d, a, b)
    }

    pub fn gen_shl(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Shl, ty, d, a, b)
    }

    pub fn gen_shr(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Shr, ty, d, a, b)
    }

    pub fn gen_sar(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Sar, ty, d, a, b)
    }

    // -- Unary (1 oarg, 1 iarg) --

    pub fn gen_neg(&mut self, ty: Type, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_unary(Opcode::Neg, ty, d, s)
    }

    pub fn gen_not(&mut self, ty: Type, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_unary(Opcode::Not, ty, d, s)
    }

    pub fn gen_mov(&mut self, ty: Type, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_unary(Opcode::Mov, ty, d, s)
    }

    // -- SetCond (1 oarg, 2 iargs, 1 carg) --

    pub fn gen_setcond(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
        cond: Cond,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::SetCond,
            ty,
            &[d, a, b, carg(cond as u32)],
        );
        self.emit_op(op);
        d
    }

    // -- Host Ld/St (for CPUState access) --

    /// Load: dst = *(base + offset)
    /// Ld: 1 oarg, 1 iarg, 1 carg (offset)
    pub fn gen_ld(
        &mut self,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::Ld,
            ty,
            &[dst, base, carg(offset as u32)],
        );
        self.emit_op(op);
        dst
    }

    /// Store: *(base + offset) = src
    /// St: 0 oargs, 2 iargs, 1 carg (offset)
    pub fn gen_st(
        &mut self,
        ty: Type,
        src: TempIdx,
        base: TempIdx,
        offset: i64,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::St,
            ty,
            &[src, base, carg(offset as u32)],
        );
        self.emit_op(op);
    }

    // -- Control flow --

    /// Unconditional branch to label.
    /// Br: 0 oargs, 0 iargs, 1 carg (label_id)
    pub fn gen_br(&mut self, label_id: u32) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::Br, Type::I64, &[carg(label_id)]);
        self.emit_op(op);
    }

    /// Conditional branch.
    /// BrCond: 0 oargs, 2 iargs, 2 cargs (cond, label_id)
    pub fn gen_brcond(
        &mut self,
        ty: Type,
        a: TempIdx,
        b: TempIdx,
        cond: Cond,
        label_id: u32,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::BrCond,
            ty,
            &[a, b, carg(cond as u32), carg(label_id)],
        );
        self.emit_op(op);
    }

    /// Define label position.
    /// SetLabel: 0 oargs, 0 iargs, 1 carg (label_id)
    pub fn gen_set_label(&mut self, label_id: u32) {
        let idx = self.next_op_idx();
        let op =
            Op::with_args(idx, Opcode::SetLabel, Type::I64, &[carg(label_id)]);
        self.emit_op(op);
    }

    // -- TB exit --

    /// GotoTb: 0 oargs, 0 iargs, 1 carg (tb_idx)
    pub fn gen_goto_tb(&mut self, tb_idx: u32) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::GotoTb, Type::I64, &[carg(tb_idx)]);
        self.emit_op(op);
    }

    /// ExitTb: 0 oargs, 0 iargs, 1 carg (val)
    pub fn gen_exit_tb(&mut self, val: u64) {
        let idx = self.next_op_idx();
        let op =
            Op::with_args(idx, Opcode::ExitTb, Type::I64, &[carg(val as u32)]);
        self.emit_op(op);
    }

    // -- Boundary --

    /// InsnStart: 0 oargs, 0 iargs, 2 cargs (pc_lo, pc_hi)
    pub fn gen_insn_start(&mut self, pc: u64) {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::InsnStart,
            Type::I64,
            &[carg(pc as u32), carg((pc >> 32) as u32)],
        );
        self.emit_op(op);
    }
}
