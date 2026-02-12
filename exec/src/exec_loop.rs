use crate::{ExecEnv, GuestCpu, MIN_CODE_BUF_REMAINING};
use tcg_backend::translate::translate;
use tcg_backend::HostCodeGen;
use tcg_core::tb::{decode_tb_exit, TB_EXIT_NOCHAIN};

/// Reason the execution loop exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    /// TB returned a non-zero exit value.
    Exit(usize),
    /// Code buffer is full; caller should flush and retry.
    BufferFull,
}

/// Main CPU execution loop.
///
/// # Safety
/// The caller must ensure `cpu.env_ptr()` points to a valid
/// CPU state struct matching the globals in `env.ir_ctx`.
pub unsafe fn cpu_exec_loop<B, C>(
    env: &mut ExecEnv<B>,
    cpu: &mut C,
) -> ExitReason
where
    B: HostCodeGen,
    C: GuestCpu,
{
    let mut next_tb_hint: Option<usize> = None;

    loop {
        env.stats.loop_iters += 1;

        let tb_idx = match next_tb_hint.take() {
            Some(idx) => {
                env.stats.hint_used += 1;
                idx
            }
            None => {
                let pc = cpu.get_pc();
                let flags = cpu.get_flags();
                match tb_find(env, cpu, pc, flags) {
                    Some(idx) => idx,
                    None => return ExitReason::BufferFull,
                }
            }
        };

        let raw_exit = cpu_tb_exec(env, cpu, tb_idx);
        let (last_tb, exit_code) = decode_tb_exit(raw_exit);
        let src_tb = last_tb.unwrap_or(tb_idx);

        match exit_code {
            v @ 0..=1 => {
                let slot = v;
                env.stats.chain_exit[slot] += 1;

                // Opt 2: cached cycle — reuse destination
                // as hint, skip tb_find + chain_reachable.
                let stb = env.tb_store.get(src_tb);
                if stb.jmp_nochain[slot] {
                    if let Some(dst) = stb.jmp_dest[slot] {
                        let dtb = env.tb_store.get(dst);
                        let pc = cpu.get_pc();
                        let flags = cpu.get_flags();
                        if !dtb.invalid && dtb.pc == pc && dtb.flags == flags {
                            next_tb_hint = Some(dst);
                            continue;
                        }
                    }
                }

                let pc = cpu.get_pc();
                let flags = cpu.get_flags();
                let dst = match tb_find(env, cpu, pc, flags) {
                    Some(idx) => idx,
                    None => return ExitReason::BufferFull,
                };

                if !chain_reachable(&env.tb_store, dst, src_tb) {
                    tb_add_jump(env, src_tb, slot, dst);
                } else {
                    // Opt 1: cache cycle result
                    env.stats.chain_cycle += 1;
                    let stb = env.tb_store.get_mut(src_tb);
                    stb.jmp_nochain[slot] = true;
                    stb.jmp_dest[slot] = Some(dst);
                }
                next_tb_hint = Some(dst);
            }
            v if v == TB_EXIT_NOCHAIN as usize => {
                env.stats.nochain_exit += 1;
                let pc = cpu.get_pc();
                let flags = cpu.get_flags();
                if let Some(dst) = env.tb_store.get(src_tb).exit_target {
                    let tb = env.tb_store.get(dst);
                    if !tb.invalid && tb.pc == pc && tb.flags == flags {
                        next_tb_hint = Some(dst);
                        continue;
                    }
                }
                let dst = match tb_find(env, cpu, pc, flags) {
                    Some(idx) => idx,
                    None => return ExitReason::BufferFull,
                };
                env.tb_store.get_mut(src_tb).exit_target = Some(dst);
                next_tb_hint = Some(dst);
            }
            _ => {
                env.stats.real_exit += 1;
                return ExitReason::Exit(exit_code);
            }
        }
    }
}

/// Find a TB for the given (pc, flags), translating if needed.
fn tb_find<B, C>(
    env: &mut ExecEnv<B>,
    cpu: &mut C,
    pc: u64,
    flags: u32,
) -> Option<usize>
where
    B: HostCodeGen,
    C: GuestCpu,
{
    // Fast path: jump cache
    if let Some(idx) = env.jump_cache.lookup(pc) {
        let tb = env.tb_store.get(idx);
        if !tb.invalid && tb.pc == pc && tb.flags == flags {
            env.stats.jc_hit += 1;
            return Some(idx);
        }
    }

    // Slow path: hash table
    if let Some(idx) = env.tb_store.lookup(pc, flags) {
        env.jump_cache.insert(pc, idx);
        env.stats.ht_hit += 1;
        return Some(idx);
    }

    // Miss: translate a new TB
    env.stats.translate += 1;
    tb_gen_code(env, cpu, pc, flags)
}

/// Translate guest code at `pc` into a new TB.
fn tb_gen_code<B, C>(
    env: &mut ExecEnv<B>,
    cpu: &mut C,
    pc: u64,
    flags: u32,
) -> Option<usize>
where
    B: HostCodeGen,
    C: GuestCpu,
{
    if env.code_buf.remaining() < MIN_CODE_BUF_REMAINING {
        return None;
    }

    let tb_idx = env.tb_store.alloc(pc, flags, 0);

    env.ir_ctx.reset();
    env.ir_ctx.tb_idx = tb_idx as u32;
    let guest_size = cpu.gen_code(
        &mut env.ir_ctx,
        pc,
        tcg_core::tb::TranslationBlock::max_insns(0),
    );
    env.tb_store.get_mut(tb_idx).size = guest_size;

    env.backend.clear_goto_tb_offsets();

    let host_offset =
        translate(&mut env.ir_ctx, &env.backend, &mut env.code_buf);
    let host_size = env.code_buf.offset() - host_offset;

    let tb = env.tb_store.get_mut(tb_idx);
    tb.host_offset = host_offset;
    tb.host_size = host_size;

    let offsets = env.backend.goto_tb_offsets();
    for (i, &(jmp, reset)) in offsets.iter().enumerate().take(2) {
        tb.set_jmp_insn_offset(i, jmp as u32);
        tb.set_jmp_reset_offset(i, reset as u32);
    }

    env.tb_store.insert(tb_idx);
    env.jump_cache.insert(pc, tb_idx);

    Some(tb_idx)
}

/// Execute a single TB and return the exit value.
unsafe fn cpu_tb_exec<B, C>(
    env: &mut ExecEnv<B>,
    cpu: &mut C,
    tb_idx: usize,
) -> usize
where
    B: HostCodeGen,
    C: GuestCpu,
{
    let tb = env.tb_store.get(tb_idx);
    let tb_ptr = env.code_buf.ptr_at(tb.host_offset);
    let env_ptr = cpu.env_ptr();

    let prologue_fn: unsafe extern "C" fn(*mut u8, *const u8) -> usize =
        core::mem::transmute(env.code_buf.base_ptr());
    prologue_fn(env_ptr, tb_ptr)
}

/// Check if `target` is reachable from `from` via chains.
fn chain_reachable(
    tb_store: &crate::TbStore,
    from: usize,
    target: usize,
) -> bool {
    fn walk(
        tb_store: &crate::TbStore,
        cur: usize,
        target: usize,
        depth: usize,
    ) -> bool {
        if cur == target {
            return true;
        }
        if depth == 0 {
            return false;
        }
        let tb = tb_store.get(cur);
        for slot in 0..2 {
            if let Some(next) = tb.jmp_dest[slot] {
                if walk(tb_store, next, target, depth - 1) {
                    return true;
                }
            }
        }
        false
    }
    walk(tb_store, from, target, 32)
}

/// Patch a goto_tb jump to directly chain src -> dst.
fn tb_add_jump<B: HostCodeGen>(
    env: &mut ExecEnv<B>,
    src: usize,
    slot: usize,
    dst: usize,
) {
    let src_tb = env.tb_store.get(src);
    let jmp_off = match src_tb.jmp_insn_offset[slot] {
        Some(off) => off as usize,
        None => return,
    };

    if env.tb_store.get(dst).invalid {
        return;
    }

    if env.tb_store.get(src).jmp_dest[slot] == Some(dst) {
        env.stats.chain_already += 1;
        return;
    }

    let abs_dst = env.tb_store.get(dst).host_offset;
    env.backend.patch_jump(&mut env.code_buf, jmp_off, abs_dst);

    env.tb_store.get_mut(src).jmp_dest[slot] = Some(dst);
    env.tb_store.get_mut(dst).jmp_list.push((src, slot));

    env.stats.chain_patched += 1;
}
