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

/// RISC-V CPU state with a small memory window for load/store tests.
#[repr(C)]
struct RiscvCpuStateMem {
    regs: [u64; 32],
    pc: u64,
    mem: [u8; 64],
}

impl RiscvCpuStateMem {
    fn new() -> Self {
        Self {
            regs: [0; 32],
            pc: 0,
            mem: [0; 64],
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

fn run_riscv_tb<S, F>(cpu: &mut S, build: F) -> usize
where
    F: FnOnce(&mut Context, TempIdx, [TempIdx; 32], TempIdx),
{
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (env, regs, pc) = setup_riscv_globals(&mut ctx);

    build(&mut ctx, env, regs, pc);

    unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            cpu as *mut S as *mut u8,
        )
    }
}

macro_rules! riscv_bin_case {
    ($name:ident, $op:ident, $lhs:expr, $rhs:expr, $expect:expr) => {
        #[test]
        fn $name() {
            let mut cpu = RiscvCpuState::new();
            cpu.regs[1] = $lhs;
            cpu.regs[2] = $rhs;

            let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
                let tmp = ctx.new_temp(Type::I64);
                ctx.gen_insn_start(0x4000);
                ctx.$op(Type::I64, tmp, regs[1], regs[2]);
                ctx.gen_mov(Type::I64, regs[3], tmp);
                ctx.gen_exit_tb(0);
            });

            assert_eq!(exit_val, 0);
            assert_eq!(cpu.regs[3], $expect);
        }
    };
}

macro_rules! riscv_shift_case {
    ($name:ident, $op:ident, $val:expr, $shift:expr, $expect:expr) => {
        #[test]
        fn $name() {
            let mut cpu = RiscvCpuState::new();
            cpu.regs[1] = $val;
            cpu.regs[2] = $shift;

            let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
                let tmp = ctx.new_temp(Type::I64);
                ctx.gen_insn_start(0x4100);
                ctx.$op(Type::I64, tmp, regs[1], regs[2]);
                ctx.gen_mov(Type::I64, regs[3], tmp);
                ctx.gen_exit_tb(0);
            });

            assert_eq!(exit_val, 0);
            assert_eq!(cpu.regs[3], $expect);
        }
    };
}

macro_rules! riscv_setcond_case {
    ($name:ident, $cond:expr, $lhs:expr, $rhs:expr, $expect:expr) => {
        #[test]
        fn $name() {
            let mut cpu = RiscvCpuState::new();
            cpu.regs[1] = $lhs;
            cpu.regs[2] = $rhs;

            let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
                let tmp = ctx.new_temp(Type::I64);
                ctx.gen_insn_start(0x4200);
                ctx.gen_setcond(Type::I64, tmp, regs[1], regs[2], $cond);
                ctx.gen_mov(Type::I64, regs[3], tmp);
                ctx.gen_exit_tb(0);
            });

            assert_eq!(exit_val, 0);
            assert_eq!(cpu.regs[3], $expect);
        }
    };
}

macro_rules! riscv_branch_case {
    ($name:ident, $cond:expr, $lhs:expr, $rhs:expr, $taken:expr, $not:expr, $expect:expr) => {
        #[test]
        fn $name() {
            let mut cpu = RiscvCpuState::new();
            cpu.regs[1] = $lhs;
            cpu.regs[2] = $rhs;

            let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
                let label_taken = ctx.new_label();
                let label_end = ctx.new_label();
                let t_taken = ctx.new_temp(Type::I64);
                let t_not = ctx.new_temp(Type::I64);
                let c_taken = ctx.new_const(Type::I64, $taken);
                let c_not = ctx.new_const(Type::I64, $not);

                ctx.gen_insn_start(0x4300);
                ctx.gen_brcond(Type::I64, regs[1], regs[2], $cond, label_taken);
                ctx.gen_mov(Type::I64, t_not, c_not);
                ctx.gen_mov(Type::I64, regs[3], t_not);
                ctx.gen_br(label_end);

                ctx.gen_set_label(label_taken);
                ctx.gen_mov(Type::I64, t_taken, c_taken);
                ctx.gen_mov(Type::I64, regs[3], t_taken);

                ctx.gen_set_label(label_end);
                ctx.gen_exit_tb(0);
            });

            assert_eq!(exit_val, 0);
            assert_eq!(cpu.regs[3], $expect);
        }
    };
}

macro_rules! riscv_mem_case {
    ($name:ident, $offset:expr, $value:expr) => {
        #[test]
        fn $name() {
            let mut cpu = RiscvCpuStateMem::new();
            let exit_val = run_riscv_tb(&mut cpu, |ctx, env, regs, _pc| {
                let t_val = ctx.new_temp(Type::I64);
                let t_load = ctx.new_temp(Type::I64);
                let cval = ctx.new_const(Type::I64, $value);
                let mem_offset =
                    std::mem::offset_of!(RiscvCpuStateMem, mem) as i64 + $offset;

                ctx.gen_insn_start(0x4400);
                ctx.gen_mov(Type::I64, t_val, cval);
                ctx.gen_st(Type::I64, t_val, env, mem_offset);
                ctx.gen_ld(Type::I64, t_load, env, mem_offset);
                ctx.gen_mov(Type::I64, regs[4], t_load);
                ctx.gen_exit_tb(0);
            });

            assert_eq!(exit_val, 0);
            assert_eq!(cpu.regs[4], $value);
            let start = $offset as usize;
            let end = start + 8;
            let stored =
                u64::from_le_bytes(cpu.mem[start..end].try_into().unwrap());
            assert_eq!(stored, $value);
        }
    };
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

/// Test: combine AND/XOR/OR/ADD in one TB (AND, XOR, OR, ADD).
#[test]
fn test_alu_mix_and_or_xor_add() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    ctx.gen_insn_start(0x3000);
    let t_and = ctx.new_temp(Type::I64);
    let t_xor = ctx.new_temp(Type::I64);
    let t_or = ctx.new_temp(Type::I64);
    let t_add = ctx.new_temp(Type::I64);

    ctx.gen_and(Type::I64, t_and, regs[1], regs[2]);
    ctx.gen_xor(Type::I64, t_xor, regs[3], regs[4]);
    ctx.gen_or(Type::I64, t_or, t_and, t_xor);
    ctx.gen_add(Type::I64, t_add, t_or, t_and);
    ctx.gen_mov(Type::I64, regs[5], t_add);
    ctx.gen_exit_tb(0);

    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0x0F0F;
    cpu.regs[2] = 0xFF00;
    cpu.regs[3] = 0x1234;
    cpu.regs[4] = 0x00FF;

    let expected_and = cpu.regs[1] & cpu.regs[2];
    let expected_xor = cpu.regs[3] ^ cpu.regs[4];
    let expected_or = expected_and | expected_xor;
    let expected_add = expected_or.wrapping_add(expected_and);

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[5], expected_add);
}

/// Test: MUL/ADD/NEG/NOT chain in one TB.
#[test]
fn test_mul_add_neg_not_chain() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    let t_mul = ctx.new_temp(Type::I64);
    let t_add = ctx.new_temp(Type::I64);
    let t_neg = ctx.new_temp(Type::I64);
    let t_not = ctx.new_temp(Type::I64);

    ctx.gen_insn_start(0x3050);
    ctx.gen_mul(Type::I64, t_mul, regs[1], regs[2]);
    ctx.gen_add(Type::I64, t_add, t_mul, regs[3]);
    ctx.gen_neg(Type::I64, t_neg, t_add);
    ctx.gen_not(Type::I64, t_not, t_neg);
    ctx.gen_mov(Type::I64, regs[6], t_neg);
    ctx.gen_mov(Type::I64, regs[7], t_not);
    ctx.gen_exit_tb(0);

    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 6;
    cpu.regs[2] = 7;
    cpu.regs[3] = 5;

    let expected_mul = cpu.regs[1].wrapping_mul(cpu.regs[2]);
    let expected_add = expected_mul.wrapping_add(cpu.regs[3]);
    let expected_neg = 0u64.wrapping_sub(expected_add);
    let expected_not = !expected_neg;

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[6], expected_neg);
    assert_eq!(cpu.regs[7], expected_not);
}

/// Test: SLT/SLTU using SetCond for signed and unsigned compares.
#[test]
fn test_slt_sltu() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    let a = ctx.new_const(Type::I64, 0xFFFF_FFFF_FFFF_FFFF);
    let b = ctx.new_const(Type::I64, 1);
    let t_slt = ctx.new_temp(Type::I64);
    let t_sltu = ctx.new_temp(Type::I64);

    ctx.gen_insn_start(0x3200);
    ctx.gen_setcond(Type::I64, t_slt, a, b, tcg_core::Cond::Lt);
    ctx.gen_setcond(Type::I64, t_sltu, a, b, tcg_core::Cond::Ltu);
    ctx.gen_mov(Type::I64, regs[8], t_slt);
    ctx.gen_mov(Type::I64, regs[9], t_sltu);
    ctx.gen_exit_tb(0);

    let mut cpu = RiscvCpuState::new();

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[8], 1);
    assert_eq!(cpu.regs[9], 0);
}

/// Test: AUIPC/LUI style sequences using pc + imm and imm << 12.
#[test]
fn test_auipc_lui() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, pc) = setup_riscv_globals(&mut ctx);

    let imm20 = 0x12345u64;
    let imm = imm20 << 12;
    let cimm = ctx.new_const(Type::I64, imm);
    let t_auipc = ctx.new_temp(Type::I64);

    ctx.gen_insn_start(0x3300);
    ctx.gen_add(Type::I64, t_auipc, pc, cimm);
    ctx.gen_mov(Type::I64, regs[10], t_auipc);
    ctx.gen_mov(Type::I64, regs[11], cimm);
    ctx.gen_exit_tb(0);

    let mut cpu = RiscvCpuState::new();
    cpu.pc = 0x1000;

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], cpu.pc.wrapping_add(imm));
    assert_eq!(cpu.regs[11], imm);
}

/// Test: store/load via env base, then move back to a register.
#[test]
fn test_load_store_64() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (env, regs, _pc) = setup_riscv_globals(&mut ctx);

    let value = 0xDEAD_BEEF_DEAD_BEEFu64;
    let cval = ctx.new_const(Type::I64, value);
    let t_val = ctx.new_temp(Type::I64);
    let t_load = ctx.new_temp(Type::I64);
    let mem_offset = std::mem::offset_of!(RiscvCpuStateMem, mem) as i64;

    ctx.gen_insn_start(0x3400);
    ctx.gen_mov(Type::I64, t_val, cval);
    ctx.gen_st(Type::I64, t_val, env, mem_offset);
    ctx.gen_ld(Type::I64, t_load, env, mem_offset);
    ctx.gen_mov(Type::I64, regs[12], t_load);
    ctx.gen_exit_tb(0);

    let mut cpu = RiscvCpuStateMem::new();

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuStateMem as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[12], value);
    let stored = u64::from_le_bytes(cpu.mem[0..8].try_into().unwrap());
    assert_eq!(stored, value);
}

/// Test: signed vs unsigned branches with two compare paths.
#[test]
fn test_signed_unsigned_branches() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (_env, regs, _pc) = setup_riscv_globals(&mut ctx);

    let label_signed = ctx.new_label();
    let label_signed_end = ctx.new_label();
    let label_unsigned = ctx.new_label();
    let label_unsigned_end = ctx.new_label();

    let imm1 = ctx.new_const(Type::I64, 1);
    let imm2 = ctx.new_const(Type::I64, 2);
    let imm3 = ctx.new_const(Type::I64, 3);
    let imm4 = ctx.new_const(Type::I64, 4);
    let t1 = ctx.new_temp(Type::I64);
    let t2 = ctx.new_temp(Type::I64);
    let t3 = ctx.new_temp(Type::I64);
    let t4 = ctx.new_temp(Type::I64);

    ctx.gen_insn_start(0x3500);
    ctx.gen_brcond(
        Type::I64,
        regs[1],
        regs[2],
        tcg_core::Cond::Lt,
        label_signed,
    );
    ctx.gen_mov(Type::I64, t2, imm2);
    ctx.gen_mov(Type::I64, regs[13], t2);
    ctx.gen_br(label_signed_end);

    ctx.gen_set_label(label_signed);
    ctx.gen_mov(Type::I64, t1, imm1);
    ctx.gen_mov(Type::I64, regs[13], t1);
    ctx.gen_set_label(label_signed_end);

    ctx.gen_brcond(
        Type::I64,
        regs[1],
        regs[2],
        tcg_core::Cond::Ltu,
        label_unsigned,
    );
    ctx.gen_mov(Type::I64, t4, imm4);
    ctx.gen_mov(Type::I64, regs[14], t4);
    ctx.gen_br(label_unsigned_end);

    ctx.gen_set_label(label_unsigned);
    ctx.gen_mov(Type::I64, t3, imm3);
    ctx.gen_mov(Type::I64, regs[14], t3);
    ctx.gen_set_label(label_unsigned_end);
    ctx.gen_exit_tb(0);

    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0xFFFF_FFFF_FFFF_FFFF;
    cpu.regs[2] = 1;

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuState as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[13], 1);
    assert_eq!(cpu.regs[14], 4);
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
