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
/// Repeatedly looks up or translates TBs and executes them
/// until a TB returns a non-zero exit value or the code buffer
/// is exhausted.
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
        let tb_idx = match next_tb_hint.take() {
            Some(idx) => idx,
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
        // After direct chaining, the TB that actually exited
        // may differ from the one we called.  Use the decoded
        // source TB for linking; fall back to tb_idx when the
        // exit value carries no TB marker (real exits).
        let src_tb = last_tb.unwrap_or(tb_idx);

        match exit_code {
            v @ 0..=1 => {
                // goto_tb slot 0 or 1 — chainable direct
                // branch.  Find destination TB, then patch
                // the jump for direct chaining (subsequent
                // executions skip the exec loop entirely).
                let slot = v;
                let pc = cpu.get_pc();
                let flags = cpu.get_flags();
                let dst = match tb_find(env, cpu, pc, flags) {
                    Some(idx) => idx,
                    None => return ExitReason::BufferFull,
                };
                // Don't chain if src_tb is reachable from
                // dst — that would close a cycle and cause
                // an infinite loop in generated code.
                if !chain_reachable(&env.tb_store, dst, src_tb) {
                    tb_add_jump(env, src_tb, slot, dst);
                }
                next_tb_hint = Some(dst);
            }
            v if v == TB_EXIT_NOCHAIN as usize => {
                // Indirect jump (JALR etc.) — simplified
                // lookup_and_goto_ptr: single-entry cache
                // per TB.
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
            _ => return ExitReason::Exit(exit_code),
        }
    }
}

/// Find a TB for the given (pc, flags), translating if needed.
///
/// Returns `None` if the code buffer is too full to translate.
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
            return Some(idx);
        }
    }

    // Slow path: hash table
    if let Some(idx) = env.tb_store.lookup(pc, flags) {
        env.jump_cache.insert(pc, idx);
        return Some(idx);
    }

    // Miss: translate a new TB
    tb_gen_code(env, cpu, pc, flags)
}

/// Translate guest code at `pc` into a new TB.
///
/// Returns `None` if the code buffer has insufficient space.
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

    // Allocate TB
    let tb_idx = env.tb_store.alloc(pc, flags, 0);

    // Generate IR
    env.ir_ctx.reset();
    env.ir_ctx.tb_idx = tb_idx as u32;
    let guest_size = cpu.gen_code(
        &mut env.ir_ctx,
        pc,
        tcg_core::tb::TranslationBlock::max_insns(0),
    );
    env.tb_store.get_mut(tb_idx).size = guest_size;

    // Clear goto_tb tracking
    env.backend.clear_goto_tb_offsets();

    // Generate host code (buffer is RWX, no permission
    // switch needed)
    let host_offset =
        translate(&mut env.ir_ctx, &env.backend, &mut env.code_buf);
    let host_size = env.code_buf.offset() - host_offset;

    // Record host code location in TB
    let tb = env.tb_store.get_mut(tb_idx);
    tb.host_offset = host_offset;
    tb.host_size = host_size;

    // Record goto_tb offsets for future TB chaining
    let offsets = env.backend.goto_tb_offsets();
    for (i, &(jmp, reset)) in offsets.iter().enumerate().take(2) {
        tb.set_jmp_insn_offset(i, jmp as u32);
        tb.set_jmp_reset_offset(i, reset as u32);
    }

    // Insert into caches
    env.tb_store.insert(tb_idx);
    env.jump_cache.insert(pc, tb_idx);

    Some(tb_idx)
}

/// Execute a single TB and return the exit value.
///
/// # Safety
/// Called from the unsafe `cpu_exec_loop`.
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

    // Prologue signature:
    //   fn(env: *mut u8, tb_ptr: *const u8) -> usize
    let prologue_fn: unsafe extern "C" fn(*mut u8, *const u8) -> usize =
        core::mem::transmute(env.code_buf.base_ptr());
    prologue_fn(env_ptr, tb_ptr)
}

/// Check if `target` is reachable from `from` by following
/// existing direct chains.  Used to prevent creating cycles
/// that would cause infinite loops in generated code.
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

/// Patch a goto_tb jump to directly chain `src` → `dst`.
///
/// After patching, the host JMP at slot `slot` in `src` jumps
/// directly to `dst`'s host code, bypassing the exec loop.
/// Also records the reverse link so `dst` can unlink on
/// invalidation.
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

    // Already linked to the same target? Skip.
    if env.tb_store.get(src).jmp_dest[slot] == Some(dst) {
        return;
    }

    // Patch the JMP instruction to target dst's host code.
    // jmp_off is already an absolute code buffer offset.
    let abs_dst = env.tb_store.get(dst).host_offset;
    env.backend.patch_jump(&mut env.code_buf, jmp_off, abs_dst);

    // Record outgoing edge.
    env.tb_store.get_mut(src).jmp_dest[slot] = Some(dst);

    // Record incoming edge (reverse link).
    env.tb_store.get_mut(dst).jmp_list.push((src, slot));
}
