use tcg_backend::code_buffer::CodeBuffer;
use tcg_backend::translate::translate_and_execute;
use tcg_backend::HostCodeGen;
use tcg_backend::X86_64CodeGen;
use tcg_core::types::Type;
use tcg_core::{Context, TempIdx};

/// Minimal RISC-V CPU state for testing.
#[repr(C)]
struct RiscvCpuState {
    regs: [u64; 32], // x0-x31, offset 0..256
    pc: u64,         // offset 256
}

impl RiscvCpuState {
    fn new() -> Self {
        Self {
            regs: [0; 32],
            pc: 0,
        }
    }
}

/// Register globals for RISC-V x0-x31 and pc.
/// Returns (env_temp, reg_temps[0..32], pc_temp).
fn setup_riscv_globals(ctx: &mut Context) -> (TempIdx, [TempIdx; 32], TempIdx) {
    // env pointer is a fixed temp in RBP
    let env =
        ctx.new_fixed(Type::I64, tcg_backend::x86_64::Reg::Rbp as u8, "env");

    // x0-x31 as globals backed by RiscvCpuState.regs
    let mut reg_temps = [TempIdx(0); 32];
    for i in 0..32u32 {
        let offset = (i as i64) * 8; // regs[i] at offset i*8
        let name: &'static str = match i {
            0 => "x0",
            1 => "x1",
            2 => "x2",
            3 => "x3",
            4 => "x4",
            5 => "x5",
            _ => "xN",
        };
        reg_temps[i as usize] = ctx.new_global(Type::I64, env, offset, name);
    }

    // pc at offset 256
    let pc = ctx.new_global(Type::I64, env, 256, "pc");

    (env, reg_temps, pc)
}

/// Test: ADDI x1, x0, 42 → verify cpu.regs[1] == 42
#[test]
fn test_addi_x1_x0_42() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();

    // Emit prologue + epilogue
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    // Set up context with RISC-V globals
    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    // Generate IR: x1 = x0 + 42
    ctx.gen_insn_start(0x1000);

    // x0 is always 0 in RISC-V, but in our IR it's just a
    // global. We load it and add a constant.
    let imm42 = ctx.new_const(Type::I64, 42);
    let tmp = ctx.new_temp(Type::I64);
    ctx.gen_add(Type::I64, tmp, regs[0], imm42);
    ctx.gen_mov(Type::I64, regs[1], tmp);

    // Exit TB
    ctx.gen_exit_tb(0);

    // Execute
    let mut cpu = RiscvCpuState::new();
    cpu.regs[0] = 0; // x0 = 0

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0, "exit_tb should return 0");
    assert_eq!(cpu.regs[1], 42, "x1 should be 42");
}

/// Test: ADD x3, x1, x2 → verify x3 == x1 + x2
#[test]
fn test_add_x3_x1_x2() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    ctx.gen_insn_start(0x1000);
    let tmp = ctx.new_temp(Type::I64);
    ctx.gen_add(Type::I64, tmp, regs[1], regs[2]);
    ctx.gen_mov(Type::I64, regs[3], tmp);
    ctx.gen_exit_tb(0);

    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 100;
    cpu.regs[2] = 200;

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[3], 300, "x3 should be 100 + 200 = 300");
}

#[repr(C)]
struct ShiftCpuState {
    out: u64,
}

#[test]
fn test_shift_out_rcx_count_non_rcx() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);

    let env =
        ctx.new_fixed(Type::I64, tcg_backend::x86_64::Reg::Rbp as u8, "env");

    let c1 = ctx.new_const(Type::I64, 1);
    let cval = ctx.new_const(Type::I64, 0x10);
    let ccnt = ctx.new_const(Type::I64, 3);

    let t_hold = ctx.new_temp(Type::I64);
    let t_val = ctx.new_temp(Type::I64);
    let t_cnt = ctx.new_temp(Type::I64);
    let t_out = ctx.new_temp(Type::I64);
    let t_dummy = ctx.new_temp(Type::I64);

    ctx.gen_insn_start(0x2000);
    ctx.gen_mov(Type::I64, t_hold, c1);
    ctx.gen_mov(Type::I64, t_val, cval);
    ctx.gen_mov(Type::I64, t_cnt, ccnt);
    ctx.gen_shl(Type::I64, t_out, t_val, t_cnt);
    ctx.gen_add(Type::I64, t_dummy, t_hold, t_cnt);
    ctx.gen_st(Type::I64, t_out, env, 0);
    ctx.gen_exit_tb(0);

    let mut cpu = ShiftCpuState { out: 0 };
    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut ShiftCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.out, 0x10u64 << 3);
}

/// Test: SUB x3, x1, x2
#[test]
fn test_sub_x3_x1_x2() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    ctx.gen_insn_start(0x1000);
    let tmp = ctx.new_temp(Type::I64);
    ctx.gen_sub(Type::I64, tmp, regs[1], regs[2]);
    ctx.gen_mov(Type::I64, regs[3], tmp);
    ctx.gen_exit_tb(0);

    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 500;
    cpu.regs[2] = 200;

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[3], 300, "x3 should be 500 - 200 = 300");
}

/// Test: BEQ branch taken
#[test]
fn test_beq_taken() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    // if x1 == x2: x3 = 1; else: x3 = 2
    let label_eq = ctx.new_label();
    let label_end = ctx.new_label();

    ctx.gen_insn_start(0x1000);
    ctx.gen_brcond(Type::I64, regs[1], regs[2], tcg_core::Cond::Eq, label_eq);

    // Not equal path: x3 = 2
    let imm2 = ctx.new_const(Type::I64, 2);
    let tmp = ctx.new_temp(Type::I64);
    ctx.gen_mov(Type::I64, tmp, imm2);
    ctx.gen_mov(Type::I64, regs[3], tmp);
    ctx.gen_br(label_end);

    // Equal path: x3 = 1
    ctx.gen_set_label(label_eq);
    let imm1 = ctx.new_const(Type::I64, 1);
    let tmp2 = ctx.new_temp(Type::I64);
    ctx.gen_mov(Type::I64, tmp2, imm1);
    ctx.gen_mov(Type::I64, regs[3], tmp2);

    ctx.gen_set_label(label_end);
    ctx.gen_exit_tb(0);

    // x1 == x2 → should take equal path
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 42;
    cpu.regs[2] = 42;

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[3], 1, "branch should be taken, x3 = 1");
}

/// Test: BEQ branch not taken
#[test]
fn test_beq_not_taken() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    let label_eq = ctx.new_label();
    let label_end = ctx.new_label();

    ctx.gen_insn_start(0x1000);
    ctx.gen_brcond(Type::I64, regs[1], regs[2], tcg_core::Cond::Eq, label_eq);

    // Not equal path: x3 = 2
    let imm2 = ctx.new_const(Type::I64, 2);
    let tmp = ctx.new_temp(Type::I64);
    ctx.gen_mov(Type::I64, tmp, imm2);
    ctx.gen_mov(Type::I64, regs[3], tmp);
    ctx.gen_br(label_end);

    // Equal path: x3 = 1
    ctx.gen_set_label(label_eq);
    let imm1 = ctx.new_const(Type::I64, 1);
    let tmp2 = ctx.new_temp(Type::I64);
    ctx.gen_mov(Type::I64, tmp2, imm1);
    ctx.gen_mov(Type::I64, regs[3], tmp2);

    ctx.gen_set_label(label_end);
    ctx.gen_exit_tb(0);

    // x1 != x2 → should take not-equal path
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 42;
    cpu.regs[2] = 99;

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[3], 2, "branch not taken, x3 = 2");
}

/// Test: compute sum 1..5 using a loop
#[test]
fn test_sum_loop() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    // x1 = sum (accumulator), x2 = counter, x3 = limit
    // Loop: sum += counter; counter++; if counter <= limit goto loop
    let label_loop = ctx.new_label();
    let label_end = ctx.new_label();

    ctx.gen_insn_start(0x1000);

    // Loop header
    ctx.gen_set_label(label_loop);

    // sum += counter: x1 = x1 + x2
    let tmp_sum = ctx.new_temp(Type::I64);
    ctx.gen_add(Type::I64, tmp_sum, regs[1], regs[2]);
    ctx.gen_mov(Type::I64, regs[1], tmp_sum);

    // counter++: x2 = x2 + 1
    let imm1 = ctx.new_const(Type::I64, 1);
    let tmp_cnt = ctx.new_temp(Type::I64);
    ctx.gen_add(Type::I64, tmp_cnt, regs[2], imm1);
    ctx.gen_mov(Type::I64, regs[2], tmp_cnt);

    // if counter <= limit goto loop
    ctx.gen_brcond(Type::I64, regs[2], regs[3], tcg_core::Cond::Le, label_loop);

    ctx.gen_set_label(label_end);
    ctx.gen_exit_tb(0);

    // sum = 0, counter = 1, limit = 5
    // Expected: 1+2+3+4+5 = 15
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0; // sum
    cpu.regs[2] = 1; // counter
    cpu.regs[3] = 5; // limit

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[1], 15, "sum of 1..5 should be 15");
    assert_eq!(cpu.regs[2], 6, "counter should be 6 after loop");
}
