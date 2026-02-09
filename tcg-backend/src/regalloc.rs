use crate::code_buffer::CodeBuffer;
use crate::HostCodeGen;
use tcg_core::label::RelocKind;
use tcg_core::temp::TempKind;
use tcg_core::types::{RegSet, TempVal};
use tcg_core::{Context, OpFlags, Opcode, TempIdx, OPCODE_DEFS};

/// Register allocator state.
struct RegAllocState {
    reg_to_temp: [Option<TempIdx>; 16],
    free_regs: RegSet,
    allocatable: RegSet,
}

impl RegAllocState {
    fn new(allocatable: RegSet) -> Self {
        Self {
            reg_to_temp: [None; 16],
            free_regs: allocatable,
            allocatable,
        }
    }

    fn alloc_reg(&mut self) -> u8 {
        self.free_regs.first().expect("out of registers")
    }

    fn free_reg(&mut self, reg: u8) {
        self.reg_to_temp[reg as usize] = None;
        if self.allocatable.contains(reg) {
            self.free_regs = self.free_regs.set(reg);
        }
    }

    fn assign(&mut self, reg: u8, tidx: TempIdx) {
        self.reg_to_temp[reg as usize] = Some(tidx);
        self.free_regs = self.free_regs.clear(reg);
    }
}

/// Ensure a temp is in a host register. Returns the register.
fn temp_load(
    ctx: &mut Context,
    state: &mut RegAllocState,
    backend: &impl HostCodeGen,
    buf: &mut CodeBuffer,
    tidx: TempIdx,
) -> u8 {
    let temp = ctx.temp(tidx);
    match temp.val_type {
        TempVal::Reg => temp.reg.unwrap(),
        TempVal::Const => {
            let val = temp.val;
            let ty = temp.ty;
            let reg = state.alloc_reg();
            state.assign(reg, tidx);
            backend.tcg_out_movi(buf, ty, reg, val);
            let t = ctx.temp_mut(tidx);
            t.val_type = TempVal::Reg;
            t.reg = Some(reg);
            reg
        }
        TempVal::Mem => {
            let ty = temp.ty;
            let mem_base = temp.mem_base;
            let mem_offset = temp.mem_offset;
            let reg = state.alloc_reg();
            state.assign(reg, tidx);
            if let Some(base_idx) = mem_base {
                let base_reg = ctx.temp(base_idx).reg.unwrap();
                backend.tcg_out_ld(buf, ty, reg, base_reg, mem_offset);
            }
            let t = ctx.temp_mut(tidx);
            t.val_type = TempVal::Reg;
            t.reg = Some(reg);
            t.mem_coherent = true;
            reg
        }
        TempVal::Dead => {
            panic!("temp_load on dead temp {tidx:?}");
        }
    }
}

/// Sync a temp back to memory (for globals).
fn temp_sync(
    ctx: &Context,
    backend: &impl HostCodeGen,
    buf: &mut CodeBuffer,
    tidx: TempIdx,
) {
    let temp = ctx.temp(tidx);
    if temp.mem_coherent {
        return;
    }
    if let (Some(reg), Some(base_idx)) = (temp.reg, temp.mem_base) {
        let base_reg = ctx.temp(base_idx).reg.unwrap();
        backend.tcg_out_st(buf, temp.ty, reg, base_reg, temp.mem_offset);
    }
}

/// Sync all live globals back to memory.
fn sync_globals(
    ctx: &mut Context,
    backend: &impl HostCodeGen,
    buf: &mut CodeBuffer,
) {
    let nb_globals = ctx.nb_globals() as usize;
    for i in 0..nb_globals {
        let tidx = TempIdx(i as u32);
        let temp = ctx.temp(tidx);
        if temp.val_type == TempVal::Reg && !temp.mem_coherent {
            temp_sync(ctx, backend, buf, tidx);
            ctx.temp_mut(tidx).mem_coherent = true;
        }
    }
}

/// Free a temp's register if it's dead after this op.
fn temp_dead(ctx: &mut Context, state: &mut RegAllocState, tidx: TempIdx) {
    let temp = ctx.temp(tidx);
    if temp.is_global_or_fixed() {
        // Don't free globals/fixed — they persist.
        // But mark as mem-resident if synced.
        return;
    }
    if let Some(reg) = temp.reg {
        state.free_reg(reg);
    }
    let t = ctx.temp_mut(tidx);
    t.val_type = TempVal::Dead;
    t.reg = None;
}

/// Allocate an output register for a temp.
fn temp_alloc_output(
    ctx: &mut Context,
    state: &mut RegAllocState,
    tidx: TempIdx,
) -> u8 {
    let reg = state.alloc_reg();
    state.assign(reg, tidx);
    let t = ctx.temp_mut(tidx);
    t.val_type = TempVal::Reg;
    t.reg = Some(reg);
    t.mem_coherent = false;
    reg
}

/// Main register allocation + code generation pass.
pub fn regalloc_and_codegen(
    ctx: &mut Context,
    backend: &impl HostCodeGen,
    buf: &mut CodeBuffer,
) {
    let allocatable = crate::x86_64::regs::ALLOCATABLE_REGS;
    let mut state = RegAllocState::new(allocatable);

    // Initialize fixed temps (they're always in their reg)
    let nb_globals = ctx.nb_globals();
    for i in 0..nb_globals {
        let tidx = TempIdx(i);
        let temp = ctx.temp(tidx);
        if temp.kind == TempKind::Fixed {
            if let Some(reg) = temp.reg {
                state.assign(reg, tidx);
            }
        }
    }

    let num_ops = ctx.num_ops();
    for oi in 0..num_ops {
        let op = ctx.ops()[oi].clone();
        let def = &OPCODE_DEFS[op.opc as usize];
        let flags = def.flags;
        let life = op.life;

        match op.opc {
            Opcode::Nop | Opcode::InsnStart => continue,

            Opcode::Mov => {
                // Register rename or emit host mov
                let dst_idx = op.args[0];
                let src_idx = op.args[1];
                let src_reg = temp_load(ctx, &mut state, backend, buf, src_idx);
                // Free dead input
                if life.is_dead(1) {
                    temp_dead(ctx, &mut state, src_idx);
                }
                // Allocate output
                let dst_reg = temp_alloc_output(ctx, &mut state, dst_idx);
                if dst_reg != src_reg {
                    backend.tcg_out_mov(buf, op.op_type, dst_reg, src_reg);
                }
                if life.is_dead(0) {
                    temp_dead(ctx, &mut state, dst_idx);
                }
            }

            Opcode::SetLabel => {
                let label_id = op.args[0].0;
                // Sync all globals at label
                sync_globals(ctx, backend, buf);
                // Resolve label
                let offset = buf.offset();
                let label = ctx.label_mut(label_id);
                label.set_value(offset);
                // Back-patch forward references
                let uses: Vec<_> = label.uses.drain(..).collect();
                for u in uses {
                    match u.kind {
                        RelocKind::Rel32 => {
                            let disp = (offset as i64) - (u.offset as i64 + 4);
                            buf.patch_u32(u.offset, disp as u32);
                        }
                    }
                }
            }

            Opcode::Br => {
                let label_id = op.args[0].0;
                sync_globals(ctx, backend, buf);
                let label = ctx.label(label_id);
                if label.has_value {
                    crate::x86_64::emitter::emit_jmp(buf, label.value);
                } else {
                    // Forward ref
                    buf.emit_u8(0xE9);
                    let patch_off = buf.offset();
                    buf.emit_u32(0);
                    ctx.label_mut(label_id)
                        .add_use(patch_off, RelocKind::Rel32);
                }
            }

            Opcode::ExitTb | Opcode::GotoTb => {
                sync_globals(ctx, backend, buf);
                let nb_cargs = def.nb_cargs as usize;
                let cstart = (def.nb_oargs + def.nb_iargs) as usize;
                let cargs: Vec<u32> =
                    (0..nb_cargs).map(|i| op.args[cstart + i].0).collect();
                backend.tcg_out_op(buf, ctx, &op, &[], &[], &cargs);
            }

            Opcode::BrCond => {
                let nb_iargs = def.nb_iargs as usize;
                let nb_oargs = def.nb_oargs as usize;
                let nb_cargs = def.nb_cargs as usize;

                // Load inputs
                let mut iregs = Vec::new();
                for i in 0..nb_iargs {
                    let tidx = op.args[nb_oargs + i];
                    let reg = temp_load(ctx, &mut state, backend, buf, tidx);
                    iregs.push(reg);
                }

                // Collect cargs
                let cstart = nb_oargs + nb_iargs;
                let cargs: Vec<u32> =
                    (0..nb_cargs).map(|i| op.args[cstart + i].0).collect();

                // Free dead inputs
                for i in 0..nb_iargs {
                    let arg_pos = (nb_oargs + i) as u32;
                    if life.is_dead(arg_pos) {
                        let tidx = op.args[nb_oargs + i];
                        temp_dead(ctx, &mut state, tidx);
                    }
                }

                // Sync globals at BB boundary
                sync_globals(ctx, backend, buf);

                // Emit the comparison
                let label_id = cargs[1];
                let label = ctx.label(label_id);
                let label_resolved = label.has_value;

                backend.tcg_out_op(buf, ctx, &op, &[], &iregs, &cargs);

                // If forward ref, record for patching
                if !label_resolved {
                    // The jcc was emitted with a placeholder.
                    // The disp32 is at buf.offset() - 4.
                    let patch_off = buf.offset() - 4;
                    ctx.label_mut(label_id)
                        .add_use(patch_off, RelocKind::Rel32);
                }
            }

            _ => {
                // Generic op handling
                let nb_oargs = def.nb_oargs as usize;
                let nb_iargs = def.nb_iargs as usize;
                let nb_cargs = def.nb_cargs as usize;

                // Load inputs into registers
                let mut iregs = Vec::new();
                for i in 0..nb_iargs {
                    let tidx = op.args[nb_oargs + i];
                    let reg = temp_load(ctx, &mut state, backend, buf, tidx);
                    iregs.push(reg);
                }

                // Free dead input registers
                for i in 0..nb_iargs {
                    let arg_pos = (nb_oargs + i) as u32;
                    if life.is_dead(arg_pos) {
                        let tidx = op.args[nb_oargs + i];
                        temp_dead(ctx, &mut state, tidx);
                    }
                }

                // Allocate output registers
                let mut oregs = Vec::new();
                for i in 0..nb_oargs {
                    let tidx = op.args[i];
                    let reg = temp_alloc_output(ctx, &mut state, tidx);
                    oregs.push(reg);
                }

                // Collect constant args
                let cstart = nb_oargs + nb_iargs;
                let cargs: Vec<u32> =
                    (0..nb_cargs).map(|i| op.args[cstart + i].0).collect();

                // Emit host code
                backend.tcg_out_op(buf, ctx, &op, &oregs, &iregs, &cargs);

                // Free dead outputs
                for i in 0..nb_oargs {
                    if life.is_dead(i as u32) {
                        let tidx = op.args[i];
                        temp_dead(ctx, &mut state, tidx);
                    }
                }

                // Sync globals if needed
                for i in 0..nb_iargs {
                    let arg_pos = (nb_oargs + i) as u32;
                    if life.is_sync(arg_pos) {
                        let tidx = op.args[nb_oargs + i];
                        temp_sync(ctx, backend, buf, tidx);
                        ctx.temp_mut(tidx).mem_coherent = true;
                    }
                }

                // Sync globals at BB boundaries
                if flags.contains(OpFlags::BB_END) {
                    sync_globals(ctx, backend, buf);
                }
            }
        }
    }
}
