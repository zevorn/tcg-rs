use crate::{ExecEnv, GuestCpu, MIN_CODE_BUF_REMAINING};
use tcg_backend::translate::translate;
use tcg_backend::HostCodeGen;
use tcg_core::tb::TB_EXIT_NOCHAIN;

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

        let exit_val = cpu_tb_exec(env, cpu, tb_idx);
        match exit_val {
            v @ 0..=1 => {
                // goto_tb slot 0 or 1 — chainable direct branch.
                // QEMU: tb_add_jump(last_tb, tb_exit, next_tb).
                let slot = v;
                let pc = cpu.get_pc();
                let flags = cpu.get_flags();
                if let Some(dst) =
                    env.tb_store.get(tb_idx).jmp_target[slot]
                {
                    let tb = env.tb_store.get(dst);
                    if !tb.invalid
                        && tb.pc == pc
                        && tb.flags == flags
                    {
                        next_tb_hint = Some(dst);
                        continue;
                    }
                }
                let dst = match tb_find(env, cpu, pc, flags) {
                    Some(idx) => idx,
                    None => return ExitReason::BufferFull,
                };
                env.tb_store.get_mut(tb_idx).jmp_target[slot] =
                    Some(dst);
                next_tb_hint = Some(dst);
            }
            v if v == TB_EXIT_NOCHAIN as usize => {
                // Indirect jump (JALR etc.) — simplified
                // lookup_and_goto_ptr: single-entry cache per TB.
                let pc = cpu.get_pc();
                let flags = cpu.get_flags();
                if let Some(dst) =
                    env.tb_store.get(tb_idx).exit_target
                {
                    let tb = env.tb_store.get(dst);
                    if !tb.invalid
                        && tb.pc == pc
                        && tb.flags == flags
                    {
                        next_tb_hint = Some(dst);
                        continue;
                    }
                }
                let dst = match tb_find(env, cpu, pc, flags) {
                    Some(idx) => idx,
                    None => return ExitReason::BufferFull,
                };
                env.tb_store.get_mut(tb_idx).exit_target =
                    Some(dst);
                next_tb_hint = Some(dst);
            }
            _ => return ExitReason::Exit(exit_val),
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
    let guest_size = cpu.gen_code(
        &mut env.ir_ctx,
        pc,
        tcg_core::tb::TranslationBlock::max_insns(0),
    );
    env.tb_store.get_mut(tb_idx).size = guest_size;

    // Clear goto_tb tracking
    env.backend.clear_goto_tb_offsets();

    // Generate host code
    env.code_buf.set_writable().expect("set_writable failed");
    let host_offset =
        translate(&mut env.ir_ctx, &env.backend, &mut env.code_buf);
    let host_size = env.code_buf.offset() - host_offset;
    env.code_buf
        .set_executable()
        .expect("set_executable failed");

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
