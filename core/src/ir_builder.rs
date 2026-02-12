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

    pub fn gen_rotl(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::RotL, ty, d, a, b)
    }

    pub fn gen_rotr(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::RotR, ty, d, a, b)
    }

    pub fn gen_andc(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::AndC, ty, d, a, b)
    }

    pub fn gen_orc(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::OrC, ty, d, a, b)
    }

    pub fn gen_eqv(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Eqv, ty, d, a, b)
    }

    pub fn gen_nand(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Nand, ty, d, a, b)
    }

    pub fn gen_nor(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Nor, ty, d, a, b)
    }

    pub fn gen_divs(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::DivS, ty, d, a, b)
    }

    pub fn gen_divu(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::DivU, ty, d, a, b)
    }

    pub fn gen_rems(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::RemS, ty, d, a, b)
    }

    pub fn gen_remu(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::RemU, ty, d, a, b)
    }

    // -- Double-width division (2 oargs, 3 iargs) --

    pub fn gen_divs2(
        &mut self,
        ty: Type,
        dl: TempIdx,
        dh: TempIdx,
        al: TempIdx,
        ah: TempIdx,
        b: TempIdx,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::DivS2, ty, &[dl, dh, al, ah, b]);
        self.emit_op(op);
    }

    pub fn gen_divu2(
        &mut self,
        ty: Type,
        dl: TempIdx,
        dh: TempIdx,
        al: TempIdx,
        ah: TempIdx,
        b: TempIdx,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::DivU2, ty, &[dl, dh, al, ah, b]);
        self.emit_op(op);
    }

    // -- Widening multiply (2 oargs, 2 iargs) --

    pub fn gen_muls2(
        &mut self,
        ty: Type,
        dl: TempIdx,
        dh: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::MulS2, ty, &[dl, dh, a, b]);
        self.emit_op(op);
    }

    pub fn gen_mulu2(
        &mut self,
        ty: Type,
        dl: TempIdx,
        dh: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::MulU2, ty, &[dl, dh, a, b]);
        self.emit_op(op);
    }

    pub fn gen_mulsh(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::MulSH, ty, d, a, b)
    }

    pub fn gen_muluh(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::MulUH, ty, d, a, b)
    }

    // -- Carry arithmetic --

    pub fn gen_addco(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::AddCO, ty, d, a, b)
    }

    pub fn gen_addci(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::AddCI, ty, d, a, b)
    }

    pub fn gen_addcio(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::AddCIO, ty, d, a, b)
    }

    pub fn gen_addc1o(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::AddC1O, ty, d, a, b)
    }

    pub fn gen_subbo(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::SubBO, ty, d, a, b)
    }

    pub fn gen_subbi(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::SubBI, ty, d, a, b)
    }

    pub fn gen_subbio(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::SubBIO, ty, d, a, b)
    }

    pub fn gen_subb1o(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::SubB1O, ty, d, a, b)
    }

    // -- Bit field --

    pub fn gen_extract(
        &mut self,
        ty: Type,
        d: TempIdx,
        src: TempIdx,
        ofs: u32,
        len: u32,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::Extract,
            ty,
            &[d, src, carg(ofs), carg(len)],
        );
        self.emit_op(op);
        d
    }

    pub fn gen_sextract(
        &mut self,
        ty: Type,
        d: TempIdx,
        src: TempIdx,
        ofs: u32,
        len: u32,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::SExtract,
            ty,
            &[d, src, carg(ofs), carg(len)],
        );
        self.emit_op(op);
        d
    }

    pub fn gen_deposit(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
        ofs: u32,
        len: u32,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::Deposit,
            ty,
            &[d, a, b, carg(ofs), carg(len)],
        );
        self.emit_op(op);
        d
    }

    pub fn gen_extract2(
        &mut self,
        ty: Type,
        d: TempIdx,
        al: TempIdx,
        ah: TempIdx,
        ofs: u32,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op =
            Op::with_args(idx, Opcode::Extract2, ty, &[d, al, ah, carg(ofs)]);
        self.emit_op(op);
        d
    }

    // -- Byte swap --

    pub fn gen_bswap16(
        &mut self,
        ty: Type,
        d: TempIdx,
        src: TempIdx,
        flags: u32,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op =
            Op::with_args(idx, Opcode::Bswap16, ty, &[d, src, carg(flags)]);
        self.emit_op(op);
        d
    }

    pub fn gen_bswap32(
        &mut self,
        ty: Type,
        d: TempIdx,
        src: TempIdx,
        flags: u32,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op =
            Op::with_args(idx, Opcode::Bswap32, ty, &[d, src, carg(flags)]);
        self.emit_op(op);
        d
    }

    pub fn gen_bswap64(
        &mut self,
        ty: Type,
        d: TempIdx,
        src: TempIdx,
        flags: u32,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op =
            Op::with_args(idx, Opcode::Bswap64, ty, &[d, src, carg(flags)]);
        self.emit_op(op);
        d
    }

    // -- Bit counting --

    pub fn gen_clz(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Clz, ty, d, a, b)
    }

    pub fn gen_ctz(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_binary(Opcode::Ctz, ty, d, a, b)
    }

    pub fn gen_ctpop(&mut self, ty: Type, d: TempIdx, src: TempIdx) -> TempIdx {
        self.emit_unary(Opcode::CtPop, ty, d, src)
    }

    // -- 32-bit host: 64-bit ops on paired regs --

    /// BrCond2I32: 64-bit conditional branch on 32-bit host.
    /// 0 oargs, 4 iargs (al, ah, bl, bh), 2 cargs (cond, label)
    pub fn gen_brcond2_i32(
        &mut self,
        al: TempIdx,
        ah: TempIdx,
        bl: TempIdx,
        bh: TempIdx,
        cond: Cond,
        label_id: u32,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::BrCond2I32,
            Type::I32,
            &[al, ah, bl, bh, carg(cond as u32), carg(label_id)],
        );
        self.emit_op(op);
    }

    /// SetCond2I32: 64-bit setcond on 32-bit host.
    /// 1 oarg, 4 iargs (al, ah, bl, bh), 1 carg (cond)
    pub fn gen_setcond2_i32(
        &mut self,
        d: TempIdx,
        al: TempIdx,
        ah: TempIdx,
        bl: TempIdx,
        bh: TempIdx,
        cond: Cond,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::SetCond2I32,
            Type::I32,
            &[d, al, ah, bl, bh, carg(cond as u32)],
        );
        self.emit_op(op);
        d
    }

    // -- NegSetCond / MovCond --

    pub fn gen_negsetcond(
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
            Opcode::NegSetCond,
            ty,
            &[d, a, b, carg(cond as u32)],
        );
        self.emit_op(op);
        d
    }

    #[allow(clippy::too_many_arguments)]
    pub fn gen_movcond(
        &mut self,
        ty: Type,
        d: TempIdx,
        c1: TempIdx,
        c2: TempIdx,
        v1: TempIdx,
        v2: TempIdx,
        cond: Cond,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::MovCond,
            ty,
            &[d, c1, c2, v1, v2, carg(cond as u32)],
        );
        self.emit_op(op);
        d
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

    // -- Type conversion (1 oarg, 1 iarg) --

    /// Sign-extend i32 → i64.
    pub fn gen_ext_i32_i64(&mut self, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_unary(Opcode::ExtI32I64, Type::I64, d, s)
    }

    /// Zero-extend i32 → i64.
    pub fn gen_ext_u32_i64(&mut self, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_unary(Opcode::ExtUI32I64, Type::I64, d, s)
    }

    /// Truncate i64 → i32 (low 32 bits).
    pub fn gen_extrl_i64_i32(&mut self, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_unary(Opcode::ExtrlI64I32, Type::I32, d, s)
    }

    /// Extract i64 → i32 (high 32 bits).
    pub fn gen_extrh_i64_i32(&mut self, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_unary(Opcode::ExtrhI64I32, Type::I32, d, s)
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

    // -- Sized loads (1 oarg, 1 iarg, 1 carg) --

    fn emit_sized_load(
        &mut self,
        opc: Opcode,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, opc, ty, &[dst, base, carg(offset as u32)]);
        self.emit_op(op);
        dst
    }

    /// Load unsigned byte: dst = *(u8*)(base + offset)
    pub fn gen_ld8u(
        &mut self,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        self.emit_sized_load(Opcode::Ld8U, ty, dst, base, offset)
    }

    /// Load signed byte: dst = *(i8*)(base + offset)
    pub fn gen_ld8s(
        &mut self,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        self.emit_sized_load(Opcode::Ld8S, ty, dst, base, offset)
    }

    /// Load unsigned halfword: dst = *(u16*)(base + offset)
    pub fn gen_ld16u(
        &mut self,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        self.emit_sized_load(Opcode::Ld16U, ty, dst, base, offset)
    }

    /// Load signed halfword: dst = *(i16*)(base + offset)
    pub fn gen_ld16s(
        &mut self,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        self.emit_sized_load(Opcode::Ld16S, ty, dst, base, offset)
    }

    /// Load unsigned word: dst = *(u32*)(base + offset)
    pub fn gen_ld32u(
        &mut self,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        self.emit_sized_load(Opcode::Ld32U, ty, dst, base, offset)
    }

    /// Load signed word: dst = *(i32*)(base + offset)
    pub fn gen_ld32s(
        &mut self,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        self.emit_sized_load(Opcode::Ld32S, ty, dst, base, offset)
    }

    // -- Sized stores (0 oargs, 2 iargs, 1 carg) --

    fn emit_sized_store(
        &mut self,
        opc: Opcode,
        ty: Type,
        src: TempIdx,
        base: TempIdx,
        offset: i64,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, opc, ty, &[src, base, carg(offset as u32)]);
        self.emit_op(op);
    }

    /// Store byte: *(u8*)(base + offset) = src
    pub fn gen_st8(
        &mut self,
        ty: Type,
        src: TempIdx,
        base: TempIdx,
        offset: i64,
    ) {
        self.emit_sized_store(Opcode::St8, ty, src, base, offset);
    }

    /// Store halfword: *(u16*)(base + offset) = src
    pub fn gen_st16(
        &mut self,
        ty: Type,
        src: TempIdx,
        base: TempIdx,
        offset: i64,
    ) {
        self.emit_sized_store(Opcode::St16, ty, src, base, offset);
    }

    /// Store word: *(u32*)(base + offset) = src
    pub fn gen_st32(
        &mut self,
        ty: Type,
        src: TempIdx,
        base: TempIdx,
        offset: i64,
    ) {
        self.emit_sized_store(Opcode::St32, ty, src, base, offset);
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

    /// GotoPtr: indirect jump through register.
    pub fn gen_goto_ptr(&mut self, ptr: TempIdx) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::GotoPtr, Type::I64, &[ptr]);
        self.emit_op(op);
    }

    /// Mb: memory barrier.
    pub fn gen_mb(&mut self, bar_type: u32) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::Mb, Type::I64, &[carg(bar_type)]);
        self.emit_op(op);
    }

    /// Call helper: dst = helper(args[0..6])
    /// Call: 1 oarg, 6 iargs, 2 cargs (func_lo, func_hi)
    pub fn gen_call(
        &mut self,
        dst: TempIdx,
        helper: u64,
        args: &[TempIdx],
    ) -> TempIdx {
        let mut full_args = Vec::with_capacity(1 + 6 + 2);
        full_args.push(dst);
        let zero = self.new_const(Type::I64, 0);
        for i in 0..6 {
            let arg = args.get(i).copied().unwrap_or(zero);
            full_args.push(arg);
        }
        full_args.push(carg(helper as u32));
        full_args.push(carg((helper >> 32) as u32));
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::Call, Type::I64, &full_args);
        self.emit_op(op);
        dst
    }

    pub fn gen_discard(&mut self, ty: Type, t: TempIdx) {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::Discard, ty, &[t]);
        self.emit_op(op);
    }

    // -- Guest memory access --

    pub fn gen_qemu_ld(
        &mut self,
        ty: Type,
        dst: TempIdx,
        addr: TempIdx,
        memop: u32,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op =
            Op::with_args(idx, Opcode::QemuLd, ty, &[dst, addr, carg(memop)]);
        self.emit_op(op);
        dst
    }

    pub fn gen_qemu_st(
        &mut self,
        ty: Type,
        val: TempIdx,
        addr: TempIdx,
        memop: u32,
    ) {
        let idx = self.next_op_idx();
        let op =
            Op::with_args(idx, Opcode::QemuSt, ty, &[val, addr, carg(memop)]);
        self.emit_op(op);
    }

    pub fn gen_qemu_ld2(
        &mut self,
        ty: Type,
        dl: TempIdx,
        dh: TempIdx,
        addr: TempIdx,
        memop: u32,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::QemuLd2,
            ty,
            &[dl, dh, addr, carg(memop)],
        );
        self.emit_op(op);
    }

    pub fn gen_qemu_st2(
        &mut self,
        ty: Type,
        vl: TempIdx,
        vh: TempIdx,
        addr: TempIdx,
        memop: u32,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::QemuSt2,
            ty,
            &[vl, vh, addr, carg(memop)],
        );
        self.emit_op(op);
    }

    // -- Vector ops --

    fn emit_vec_binary(
        &mut self,
        opc: Opcode,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, opc, ty, &[d, a, b]);
        self.emit_op(op);
        d
    }

    fn emit_vec_unary(
        &mut self,
        opc: Opcode,
        ty: Type,
        d: TempIdx,
        s: TempIdx,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, opc, ty, &[d, s]);
        self.emit_op(op);
        d
    }

    pub fn gen_dup_vec(&mut self, ty: Type, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_vec_unary(Opcode::DupVec, ty, d, s)
    }

    pub fn gen_dup2_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::Dup2Vec, ty, d, a, b)
    }

    pub fn gen_ld_vec(
        &mut self,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::LdVec,
            ty,
            &[dst, base, carg(offset as u32)],
        );
        self.emit_op(op);
        dst
    }

    pub fn gen_st_vec(
        &mut self,
        ty: Type,
        src: TempIdx,
        base: TempIdx,
        offset: i64,
    ) {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::StVec,
            ty,
            &[src, base, carg(offset as u32)],
        );
        self.emit_op(op);
    }

    pub fn gen_dupm_vec(
        &mut self,
        ty: Type,
        dst: TempIdx,
        base: TempIdx,
        offset: i64,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::DupmVec,
            ty,
            &[dst, base, carg(offset as u32)],
        );
        self.emit_op(op);
        dst
    }

    pub fn gen_add_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::AddVec, ty, d, a, b)
    }

    pub fn gen_sub_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::SubVec, ty, d, a, b)
    }

    pub fn gen_mul_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::MulVec, ty, d, a, b)
    }

    pub fn gen_neg_vec(&mut self, ty: Type, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_vec_unary(Opcode::NegVec, ty, d, s)
    }

    pub fn gen_abs_vec(&mut self, ty: Type, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_vec_unary(Opcode::AbsVec, ty, d, s)
    }

    pub fn gen_ssadd_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::SsaddVec, ty, d, a, b)
    }

    pub fn gen_usadd_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::UsaddVec, ty, d, a, b)
    }

    pub fn gen_sssub_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::SssubVec, ty, d, a, b)
    }

    pub fn gen_ussub_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::UssubVec, ty, d, a, b)
    }

    pub fn gen_smin_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::SminVec, ty, d, a, b)
    }

    pub fn gen_umin_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::UminVec, ty, d, a, b)
    }

    pub fn gen_smax_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::SmaxVec, ty, d, a, b)
    }

    pub fn gen_umax_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::UmaxVec, ty, d, a, b)
    }

    pub fn gen_and_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::AndVec, ty, d, a, b)
    }

    pub fn gen_or_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::OrVec, ty, d, a, b)
    }

    pub fn gen_xor_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::XorVec, ty, d, a, b)
    }

    pub fn gen_andc_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::AndcVec, ty, d, a, b)
    }

    pub fn gen_orc_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::OrcVec, ty, d, a, b)
    }

    pub fn gen_nand_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::NandVec, ty, d, a, b)
    }

    pub fn gen_nor_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::NorVec, ty, d, a, b)
    }

    pub fn gen_eqv_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::EqvVec, ty, d, a, b)
    }

    pub fn gen_not_vec(&mut self, ty: Type, d: TempIdx, s: TempIdx) -> TempIdx {
        self.emit_vec_unary(Opcode::NotVec, ty, d, s)
    }

    // Vector shift by immediate (1 oarg, 1 iarg, 1 carg)

    fn emit_vec_shift_imm(
        &mut self,
        opc: Opcode,
        ty: Type,
        d: TempIdx,
        s: TempIdx,
        imm: u32,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, opc, ty, &[d, s, carg(imm)]);
        self.emit_op(op);
        d
    }

    pub fn gen_shli_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        s: TempIdx,
        imm: u32,
    ) -> TempIdx {
        self.emit_vec_shift_imm(Opcode::ShliVec, ty, d, s, imm)
    }

    pub fn gen_shri_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        s: TempIdx,
        imm: u32,
    ) -> TempIdx {
        self.emit_vec_shift_imm(Opcode::ShriVec, ty, d, s, imm)
    }

    pub fn gen_sari_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        s: TempIdx,
        imm: u32,
    ) -> TempIdx {
        self.emit_vec_shift_imm(Opcode::SariVec, ty, d, s, imm)
    }

    pub fn gen_rotli_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        s: TempIdx,
        imm: u32,
    ) -> TempIdx {
        self.emit_vec_shift_imm(Opcode::RotliVec, ty, d, s, imm)
    }

    // Vector shift by scalar (1 oarg, 2 iargs)

    pub fn gen_shls_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::ShlsVec, ty, d, a, b)
    }

    pub fn gen_shrs_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::ShrsVec, ty, d, a, b)
    }

    pub fn gen_sars_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::SarsVec, ty, d, a, b)
    }

    pub fn gen_rotls_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::RotlsVec, ty, d, a, b)
    }

    // Vector shift by vector (1 oarg, 2 iargs)

    pub fn gen_shlv_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::ShlvVec, ty, d, a, b)
    }

    pub fn gen_shrv_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::ShrvVec, ty, d, a, b)
    }

    pub fn gen_sarv_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::SarvVec, ty, d, a, b)
    }

    pub fn gen_rotlv_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::RotlvVec, ty, d, a, b)
    }

    pub fn gen_rotrv_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
    ) -> TempIdx {
        self.emit_vec_binary(Opcode::RotrvVec, ty, d, a, b)
    }

    // Vector compare/select

    pub fn gen_cmp_vec(
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
            Opcode::CmpVec,
            ty,
            &[d, a, b, carg(cond as u32)],
        );
        self.emit_op(op);
        d
    }

    pub fn gen_bitsel_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        a: TempIdx,
        b: TempIdx,
        c: TempIdx,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(idx, Opcode::BitselVec, ty, &[d, a, b, c]);
        self.emit_op(op);
        d
    }

    #[allow(clippy::too_many_arguments)]
    pub fn gen_cmpsel_vec(
        &mut self,
        ty: Type,
        d: TempIdx,
        c1: TempIdx,
        c2: TempIdx,
        v1: TempIdx,
        v2: TempIdx,
        cond: Cond,
    ) -> TempIdx {
        let idx = self.next_op_idx();
        let op = Op::with_args(
            idx,
            Opcode::CmpselVec,
            ty,
            &[d, c1, c2, v1, v2, carg(cond as u32)],
        );
        self.emit_op(op);
        d
    }
}
