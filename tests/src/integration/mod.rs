use tcg_backend::code_buffer::CodeBuffer;
use tcg_backend::translate::translate_and_execute;
use tcg_backend::HostCodeGen;
use tcg_backend::X86_64CodeGen;
use tcg_core::types::Type;
use tcg_core::{Context, Op, Opcode, TempIdx};

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

fn split_u128(val: u128) -> (u64, u64) {
    (val as u64, (val >> 64) as u64)
}

fn split_i128(val: i128) -> (u64, u64) {
    split_u128(val as u128)
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
                let mem_offset = std::mem::offset_of!(RiscvCpuStateMem, mem)
                    as i64
                    + $offset;

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

#[test]
fn test_exec_alu_shift_cond_mov() {
    let mut cpu = RiscvCpuState::new();
    let a = 0x1234_5678_9ABC_DEF0u64;
    let b = 0x0F0F_0F0F_0F0F_0F0Fu64;
    let sar_val = 0x8000_0000_0000_0000u64;
    let sc_a = 0xFFFF_FFFF_FFFF_FFFFu64;
    let sc_b = 1u64;
    let shift = 5u64;

    cpu.regs[1] = a;
    cpu.regs[2] = b;
    cpu.regs[3] = sar_val;
    cpu.regs[4] = sc_a;
    cpu.regs[5] = sc_b;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_mov = ctx.new_temp(Type::I64);
        let t_add = ctx.new_temp(Type::I64);
        let t_sub = ctx.new_temp(Type::I64);
        let t_mul = ctx.new_temp(Type::I64);
        let t_and = ctx.new_temp(Type::I64);
        let t_or = ctx.new_temp(Type::I64);
        let t_xor = ctx.new_temp(Type::I64);
        let t_neg = ctx.new_temp(Type::I64);
        let t_not = ctx.new_temp(Type::I64);
        let t_shl = ctx.new_temp(Type::I64);
        let t_shr = ctx.new_temp(Type::I64);
        let t_sar = ctx.new_temp(Type::I64);
        let t_sc = ctx.new_temp(Type::I64);
        let c_shift = ctx.new_const(Type::I64, shift);

        ctx.gen_insn_start(0x5000);
        ctx.gen_mov(Type::I64, t_mov, regs[1]);
        ctx.gen_mov(Type::I64, regs[10], t_mov);

        ctx.gen_add(Type::I64, t_add, regs[1], regs[2]);
        ctx.gen_mov(Type::I64, regs[11], t_add);

        ctx.gen_sub(Type::I64, t_sub, regs[1], regs[2]);
        ctx.gen_mov(Type::I64, regs[12], t_sub);

        ctx.gen_mul(Type::I64, t_mul, regs[1], regs[2]);
        ctx.gen_mov(Type::I64, regs[13], t_mul);

        ctx.gen_and(Type::I64, t_and, regs[1], regs[2]);
        ctx.gen_mov(Type::I64, regs[14], t_and);

        ctx.gen_or(Type::I64, t_or, regs[1], regs[2]);
        ctx.gen_mov(Type::I64, regs[15], t_or);

        ctx.gen_xor(Type::I64, t_xor, regs[1], regs[2]);
        ctx.gen_mov(Type::I64, regs[16], t_xor);

        ctx.gen_neg(Type::I64, t_neg, regs[1]);
        ctx.gen_mov(Type::I64, regs[17], t_neg);

        ctx.gen_not(Type::I64, t_not, regs[1]);
        ctx.gen_mov(Type::I64, regs[18], t_not);

        ctx.gen_shl(Type::I64, t_shl, regs[1], c_shift);
        ctx.gen_mov(Type::I64, regs[19], t_shl);

        ctx.gen_shr(Type::I64, t_shr, regs[1], c_shift);
        ctx.gen_mov(Type::I64, regs[20], t_shr);

        ctx.gen_sar(Type::I64, t_sar, regs[3], c_shift);
        ctx.gen_mov(Type::I64, regs[21], t_sar);

        ctx.gen_setcond(Type::I64, t_sc, regs[4], regs[5], tcg_core::Cond::Lt);
        ctx.gen_mov(Type::I64, regs[22], t_sc);

        ctx.gen_exit_tb(0);
    });

    let sh = (shift & 63) as u32;
    let expected_shl = a.wrapping_shl(sh);
    let expected_shr = a >> sh;
    let expected_sar = ((sar_val as i64) >> sh) as u64;
    let expected_sc = if (sc_a as i64) < (sc_b as i64) { 1 } else { 0 };

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], a);
    assert_eq!(cpu.regs[11], a.wrapping_add(b));
    assert_eq!(cpu.regs[12], a.wrapping_sub(b));
    assert_eq!(cpu.regs[13], a.wrapping_mul(b));
    assert_eq!(cpu.regs[14], a & b);
    assert_eq!(cpu.regs[15], a | b);
    assert_eq!(cpu.regs[16], a ^ b);
    assert_eq!(cpu.regs[17], 0u64.wrapping_sub(a));
    assert_eq!(cpu.regs[18], !a);
    assert_eq!(cpu.regs[19], expected_shl);
    assert_eq!(cpu.regs[20], expected_shr);
    assert_eq!(cpu.regs[21], expected_sar);
    assert_eq!(cpu.regs[22], expected_sc);
}

#[test]
fn test_exec_mem_and_ext() {
    let mut cpu = RiscvCpuStateMem::new();
    cpu.mem[0] = 0x80;
    cpu.mem[2..4].copy_from_slice(&0x1234u16.to_le_bytes());
    cpu.mem[4..6].copy_from_slice(&0x8000u16.to_le_bytes());
    cpu.mem[8..12].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    cpu.mem[12..16].copy_from_slice(&0x8000_0000u32.to_le_bytes());
    cpu.mem[16..24].copy_from_slice(&0x0123_4567_89AB_CDEFu64.to_le_bytes());

    let exit_val = run_riscv_tb(&mut cpu, |ctx, env, regs, _pc| {
        let mem_offset = std::mem::offset_of!(RiscvCpuStateMem, mem) as i64;
        let t_ld8u = ctx.new_temp(Type::I64);
        let t_ld8s = ctx.new_temp(Type::I64);
        let t_ld16u = ctx.new_temp(Type::I64);
        let t_ld16s = ctx.new_temp(Type::I64);
        let t_ld32u = ctx.new_temp(Type::I64);
        let t_ld32s = ctx.new_temp(Type::I64);
        let t_ld = ctx.new_temp(Type::I64);

        let c_st64 = ctx.new_const(Type::I64, 0xDEAD_BEEF_DEAD_BEEFu64);
        let c_st32 = ctx.new_const(Type::I64, 0xAABB_CCDDu64);
        let c_st16 = ctx.new_const(Type::I64, 0xEEFFu64);
        let c_st8 = ctx.new_const(Type::I64, 0x11u64);

        let c_i32_neg = ctx.new_const(Type::I32, 0xFFFF_FF80u64);
        let c_u32 = ctx.new_const(Type::I32, 0xFFFF_FFFFu64);
        let c_i64 = ctx.new_const(Type::I64, 0x1234_5678_9ABC_DEF0u64);
        let t_ext_s = ctx.new_temp(Type::I64);
        let t_ext_u = ctx.new_temp(Type::I64);
        let t_extrl = ctx.new_temp(Type::I32);

        ctx.gen_insn_start(0x5100);

        ctx.gen_ld8u(Type::I64, t_ld8u, env, mem_offset + 0);
        ctx.gen_mov(Type::I64, regs[10], t_ld8u);
        ctx.gen_ld8s(Type::I64, t_ld8s, env, mem_offset + 0);
        ctx.gen_mov(Type::I64, regs[11], t_ld8s);

        ctx.gen_ld16u(Type::I64, t_ld16u, env, mem_offset + 2);
        ctx.gen_mov(Type::I64, regs[12], t_ld16u);
        ctx.gen_ld16s(Type::I64, t_ld16s, env, mem_offset + 4);
        ctx.gen_mov(Type::I64, regs[13], t_ld16s);

        ctx.gen_ld32u(Type::I64, t_ld32u, env, mem_offset + 8);
        ctx.gen_mov(Type::I64, regs[14], t_ld32u);
        ctx.gen_ld32s(Type::I64, t_ld32s, env, mem_offset + 12);
        ctx.gen_mov(Type::I64, regs[15], t_ld32s);

        ctx.gen_ld(Type::I64, t_ld, env, mem_offset + 16);
        ctx.gen_mov(Type::I64, regs[16], t_ld);

        ctx.gen_st(Type::I64, c_st64, env, mem_offset + 32);
        ctx.gen_st32(Type::I64, c_st32, env, mem_offset + 40);
        ctx.gen_st16(Type::I64, c_st16, env, mem_offset + 44);
        ctx.gen_st8(Type::I64, c_st8, env, mem_offset + 46);

        ctx.gen_ext_i32_i64(t_ext_s, c_i32_neg);
        ctx.gen_mov(Type::I64, regs[20], t_ext_s);
        ctx.gen_ext_u32_i64(t_ext_u, c_u32);
        ctx.gen_mov(Type::I64, regs[21], t_ext_u);

        ctx.gen_extrl_i64_i32(t_extrl, c_i64);
        ctx.gen_st32(Type::I32, t_extrl, env, mem_offset + 48);

        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], 0x80);
    assert_eq!(cpu.regs[11], 0xFFFF_FFFF_FFFF_FF80u64);
    assert_eq!(cpu.regs[12], 0x1234);
    assert_eq!(cpu.regs[13], 0xFFFF_FFFF_FFFF_8000u64);
    assert_eq!(cpu.regs[14], 0x1234_5678);
    assert_eq!(cpu.regs[15], 0xFFFF_FFFF_8000_0000u64);
    assert_eq!(cpu.regs[16], 0x0123_4567_89AB_CDEFu64);
    assert_eq!(cpu.regs[20], 0xFFFF_FFFF_FFFF_FF80u64);
    assert_eq!(cpu.regs[21], 0x0000_0000_FFFF_FFFFu64);

    let mem = &cpu.mem;
    assert_eq!(
        u64::from_le_bytes(mem[32..40].try_into().unwrap()),
        0xDEAD_BEEF_DEAD_BEEFu64
    );
    assert_eq!(
        u32::from_le_bytes(mem[40..44].try_into().unwrap()),
        0xAABB_CCDDu32
    );
    assert_eq!(
        u16::from_le_bytes(mem[44..46].try_into().unwrap()),
        0xEEFFu16
    );
    assert_eq!(mem[46], 0x11u8);
    assert_eq!(
        u32::from_le_bytes(mem[48..52].try_into().unwrap()),
        0x9ABC_DEF0u32
    );
}

#[test]
fn test_exec_control_flow_ops() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 1;
    cpu.regs[2] = 2;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c1 = ctx.new_const(Type::I64, 1);
        let c2 = ctx.new_const(Type::I64, 2);
        let label_br = ctx.new_label();
        let label_taken = ctx.new_label();
        let label_end = ctx.new_label();

        ctx.gen_insn_start(0x5200);
        let nop = Op::with_args(ctx.next_op_idx(), Opcode::Nop, Type::I64, &[]);
        ctx.emit_op(nop);

        ctx.gen_br(label_br);
        ctx.gen_mov(Type::I64, regs[10], c2);
        ctx.gen_set_label(label_br);
        ctx.gen_mov(Type::I64, regs[10], c1);

        ctx.gen_brcond(
            Type::I64,
            regs[1],
            regs[2],
            tcg_core::Cond::Lt,
            label_taken,
        );
        ctx.gen_mov(Type::I64, regs[11], c2);
        ctx.gen_br(label_end);
        ctx.gen_set_label(label_taken);
        ctx.gen_mov(Type::I64, regs[11], c1);
        ctx.gen_set_label(label_end);

        ctx.gen_goto_tb(0);

        ctx.gen_exit_tb(0x1234);
    });

    assert_eq!(exit_val, 0x1234);
    assert_eq!(cpu.regs[10], 1);
    assert_eq!(cpu.regs[11], 1);
}

#[test]
fn test_exec_rotate_and_bitfield_ops() {
    let mut cpu = RiscvCpuState::new();
    let a = 0x0123_4567_89AB_CDEFu64;
    let shift = 8u64;
    let sext_val = 0x0000_0000_0000_8001u64;
    let dep_a = 0x1122_3344_5566_7788u64;
    let dep_b = 0xAAu64;
    let dep_b16 = 0xBEEF_u64;
    let ex2_al = 0x1122_3344_5566_7788u64;
    let ex2_ah = 0x99AA_BBCC_DDEE_FF00u64;
    let ex2_shift = 8u32;
    let ex32_val = 0xFFFF_FFFF_1234_5678u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c_a = ctx.new_const(Type::I64, a);
        let c_shift = ctx.new_const(Type::I64, shift);
        let c_sext = ctx.new_const(Type::I64, sext_val);
        let c_dep_a = ctx.new_const(Type::I64, dep_a);
        let c_dep_b = ctx.new_const(Type::I64, dep_b);
        let c_dep_b16 = ctx.new_const(Type::I64, dep_b16);
        let c_ex2_al = ctx.new_const(Type::I64, ex2_al);
        let c_ex2_ah = ctx.new_const(Type::I64, ex2_ah);
        let c_ex32 = ctx.new_const(Type::I64, ex32_val);

        let t_rotl = ctx.new_temp(Type::I64);
        let t_rotr = ctx.new_temp(Type::I64);
        let t_extract8 = ctx.new_temp(Type::I64);
        let t_extract32 = ctx.new_temp(Type::I64);
        let t_sextract16 = ctx.new_temp(Type::I64);
        let t_deposit8 = ctx.new_temp(Type::I64);
        let t_deposit16 = ctx.new_temp(Type::I64);
        let t_extract2 = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5300);
        ctx.gen_rotl(Type::I64, t_rotl, c_a, c_shift);
        ctx.gen_rotr(Type::I64, t_rotr, c_a, c_shift);
        ctx.gen_extract(Type::I64, t_extract8, c_a, 0, 8);
        ctx.gen_extract(Type::I64, t_extract32, c_ex32, 0, 32);
        ctx.gen_sextract(Type::I64, t_sextract16, c_sext, 0, 16);
        ctx.gen_deposit(Type::I64, t_deposit8, c_dep_a, c_dep_b, 0, 8);
        ctx.gen_deposit(Type::I64, t_deposit16, c_dep_a, c_dep_b16, 0, 16);
        ctx.gen_extract2(Type::I64, t_extract2, c_ex2_al, c_ex2_ah, ex2_shift);

        ctx.gen_mov(Type::I64, regs[10], t_rotl);
        ctx.gen_mov(Type::I64, regs[11], t_rotr);
        ctx.gen_mov(Type::I64, regs[12], t_extract8);
        ctx.gen_mov(Type::I64, regs[13], t_extract32);
        ctx.gen_mov(Type::I64, regs[14], t_sextract16);
        ctx.gen_mov(Type::I64, regs[15], t_deposit8);
        ctx.gen_mov(Type::I64, regs[16], t_deposit16);
        ctx.gen_mov(Type::I64, regs[17], t_extract2);
        ctx.gen_exit_tb(0);
    });

    let expected_rotl = a.rotate_left(shift as u32);
    let expected_rotr = a.rotate_right(shift as u32);
    let expected_extract8 = a & 0xFF;
    let expected_extract32 = ex32_val & 0xFFFF_FFFF;
    let expected_sextract16 = (sext_val as i16) as i64 as u64;
    let expected_deposit8 = (dep_a & !0xFF) | (dep_b & 0xFF);
    let expected_deposit16 = (dep_a & !0xFFFF) | (dep_b16 & 0xFFFF);
    let expected_extract2 =
        (ex2_al >> ex2_shift) | (ex2_ah << (64 - ex2_shift));

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], expected_rotl);
    assert_eq!(cpu.regs[11], expected_rotr);
    assert_eq!(cpu.regs[12], expected_extract8);
    assert_eq!(cpu.regs[13], expected_extract32);
    assert_eq!(cpu.regs[14], expected_sextract16);
    assert_eq!(cpu.regs[15], expected_deposit8);
    assert_eq!(cpu.regs[16], expected_deposit16);
    assert_eq!(cpu.regs[17], expected_extract2);
}

#[test]
fn test_exec_andc() {
    if !std::is_x86_feature_detected!("bmi1") {
        return;
    }
    let mut cpu = RiscvCpuState::new();
    let a = 0xFF00_FF00_FF00_FF00u64;
    let b = 0x0F0F_0F0F_0F0F_0F0Fu64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c_a = ctx.new_const(Type::I64, a);
        let c_b = ctx.new_const(Type::I64, b);
        let t_andc = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5310);
        ctx.gen_andc(Type::I64, t_andc, c_a, c_b);
        ctx.gen_mov(Type::I64, regs[10], t_andc);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], a & !b);
}

#[test]
fn test_exec_bswap_ops() {
    let mut cpu = RiscvCpuState::new();
    let v16 = 0xA1B2u64;
    let v32 = 0x8000_00FFu64;
    let v64 = 0x0102_0304_0506_0708u64;
    let flags_os = 4u32;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c_v16 = ctx.new_const(Type::I64, v16);
        let c_v32 = ctx.new_const(Type::I64, v32);
        let c_v64 = ctx.new_const(Type::I64, v64);
        let t_bswap16 = ctx.new_temp(Type::I64);
        let t_bswap32 = ctx.new_temp(Type::I64);
        let t_bswap64 = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5320);
        ctx.gen_bswap16(Type::I64, t_bswap16, c_v16, 0);
        ctx.gen_bswap32(Type::I64, t_bswap32, c_v32, flags_os);
        ctx.gen_bswap64(Type::I64, t_bswap64, c_v64, 0);
        ctx.gen_mov(Type::I64, regs[10], t_bswap16);
        ctx.gen_mov(Type::I64, regs[11], t_bswap32);
        ctx.gen_mov(Type::I64, regs[12], t_bswap64);
        ctx.gen_exit_tb(0);
    });

    let expected_bswap16 = 0xB2A1u64;
    let expected_bswap32 = 0xFFFF_FFFF_FF00_0080u64;
    let expected_bswap64 = 0x0807_0605_0403_0201u64;

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], expected_bswap16);
    assert_eq!(cpu.regs[11], expected_bswap32);
    assert_eq!(cpu.regs[12], expected_bswap64);
}

#[test]
fn test_exec_clz_ctz_ctpop() {
    if !std::is_x86_feature_detected!("lzcnt")
        || !std::is_x86_feature_detected!("bmi1")
        || !std::is_x86_feature_detected!("popcnt")
    {
        return;
    }

    let mut cpu = RiscvCpuState::new();
    let val_clz = 0x0010_0000_0000_0000u64;
    let val_ctz = 0x0000_0000_0000_0100u64;
    let val_pop = 0xF0F0_F00F_0001u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c_clz = ctx.new_const(Type::I64, val_clz);
        let c_ctz = ctx.new_const(Type::I64, val_ctz);
        let c_pop = ctx.new_const(Type::I64, val_pop);
        let c_fallback = ctx.new_const(Type::I64, 0x1234);
        let t_clz = ctx.new_temp(Type::I64);
        let t_ctz = ctx.new_temp(Type::I64);
        let t_pop = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5330);
        ctx.gen_clz(Type::I64, t_clz, c_clz, c_fallback);
        ctx.gen_ctz(Type::I64, t_ctz, c_ctz, c_fallback);
        ctx.gen_ctpop(Type::I64, t_pop, c_pop);
        ctx.gen_mov(Type::I64, regs[10], t_clz);
        ctx.gen_mov(Type::I64, regs[11], t_ctz);
        ctx.gen_mov(Type::I64, regs[12], t_pop);
        ctx.gen_exit_tb(0);
    });

    let expected_clz = val_clz.leading_zeros() as u64;
    let expected_ctz = val_ctz.trailing_zeros() as u64;
    let expected_pop = val_pop.count_ones() as u64;

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], expected_clz);
    assert_eq!(cpu.regs[11], expected_ctz);
    assert_eq!(cpu.regs[12], expected_pop);
}

#[test]
fn test_exec_muls2() {
    let mut cpu = RiscvCpuState::new();
    let a_s: i64 = -3;
    let b_s: i64 = 5;

    let prod_s = (a_s as i128) * (b_s as i128);
    let (muls_lo, muls_hi) = split_i128(prod_s);

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c_a_s = ctx.new_const(Type::I64, a_s as u64);
        let c_b_s = ctx.new_const(Type::I64, b_s as u64);
        let t_muls_lo = ctx.new_temp(Type::I64);
        let t_muls_hi = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5340);
        ctx.gen_muls2(Type::I64, t_muls_lo, t_muls_hi, c_a_s, c_b_s);
        ctx.gen_mov(Type::I64, regs[10], t_muls_lo);
        ctx.gen_mov(Type::I64, regs[11], t_muls_hi);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], muls_lo);
    assert_eq!(cpu.regs[11], muls_hi);
}

#[test]
fn test_exec_mulu2() {
    let mut cpu = RiscvCpuState::new();
    let a_u: u64 = 0x1_0000_0000;
    let b_u: u64 = 3;

    let prod_u = (a_u as u128) * (b_u as u128);
    let (mulu_lo, mulu_hi) = split_u128(prod_u);

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c_a_u = ctx.new_const(Type::I64, a_u);
        let c_b_u = ctx.new_const(Type::I64, b_u);
        let t_mulu_lo = ctx.new_temp(Type::I64);
        let t_mulu_hi = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5341);
        ctx.gen_mulu2(Type::I64, t_mulu_lo, t_mulu_hi, c_a_u, c_b_u);
        ctx.gen_mov(Type::I64, regs[10], t_mulu_lo);
        ctx.gen_mov(Type::I64, regs[11], t_mulu_hi);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], mulu_lo);
    assert_eq!(cpu.regs[11], mulu_hi);
}

#[test]
fn test_exec_divs2() {
    let mut cpu = RiscvCpuState::new();
    let divs_al: i64 = 100;
    let divs_ah: i64 = 0;
    let divs_b: i64 = 7;

    let divs_dividend =
        ((divs_ah as i128) << 64) | (divs_al as u64 as i128);
    let divs_q = divs_dividend / (divs_b as i128);
    let divs_r = divs_dividend % (divs_b as i128);
    let divs_q_lo = divs_q as u64;
    let divs_r_hi = divs_r as u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c_divs_al = ctx.new_const(Type::I64, divs_al as u64);
        let c_divs_ah = ctx.new_const(Type::I64, divs_ah as u64);
        let c_divs_b = ctx.new_const(Type::I64, divs_b as u64);
        let t_divs_lo = ctx.new_temp(Type::I64);
        let t_divs_hi = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5342);
        ctx.gen_divs2(
            Type::I64,
            t_divs_lo,
            t_divs_hi,
            c_divs_al,
            c_divs_ah,
            c_divs_b,
        );
        ctx.gen_mov(Type::I64, regs[10], t_divs_lo);
        ctx.gen_mov(Type::I64, regs[11], t_divs_hi);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], divs_q_lo);
    assert_eq!(cpu.regs[11], divs_r_hi);
}

#[test]
fn test_exec_divu2() {
    let mut cpu = RiscvCpuState::new();
    let divu_al: u64 = 0x1_0000_0000;
    let divu_ah: u64 = 0;
    let divu_b: u64 = 3;

    let divu_dividend =
        ((divu_ah as u128) << 64) | (divu_al as u128);
    let divu_q = divu_dividend / (divu_b as u128);
    let divu_r = divu_dividend % (divu_b as u128);
    let divu_q_lo = divu_q as u64;
    let divu_r_hi = divu_r as u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c_divu_al = ctx.new_const(Type::I64, divu_al);
        let c_divu_ah = ctx.new_const(Type::I64, divu_ah);
        let c_divu_b = ctx.new_const(Type::I64, divu_b);
        let t_divu_lo = ctx.new_temp(Type::I64);
        let t_divu_hi = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5343);
        ctx.gen_divu2(
            Type::I64,
            t_divu_lo,
            t_divu_hi,
            c_divu_al,
            c_divu_ah,
            c_divu_b,
        );
        ctx.gen_mov(Type::I64, regs[10], t_divu_lo);
        ctx.gen_mov(Type::I64, regs[11], t_divu_hi);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], divu_q_lo);
    assert_eq!(cpu.regs[11], divu_r_hi);
}

#[test]
fn test_exec_carry_borrow_ops() {
    let mut cpu = RiscvCpuState::new();
    let max = u64::MAX;
    let one = 1u64;
    let three = 3u64;
    let five = 5u64;
    let six = 6u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c_max = ctx.new_const(Type::I64, max);
        let c_one = ctx.new_const(Type::I64, one);
        let c_three = ctx.new_const(Type::I64, three);
        let c_five = ctx.new_const(Type::I64, five);
        let c_six = ctx.new_const(Type::I64, six);

        let t_addco1 = ctx.new_temp(Type::I64);
        let t_addci1 = ctx.new_temp(Type::I64);
        let t_addco2 = ctx.new_temp(Type::I64);
        let t_addcio = ctx.new_temp(Type::I64);
        let t_addci2 = ctx.new_temp(Type::I64);
        let t_addc1o = ctx.new_temp(Type::I64);

        let t_subbo = ctx.new_temp(Type::I64);
        let t_subbi1 = ctx.new_temp(Type::I64);
        let t_subbio = ctx.new_temp(Type::I64);
        let t_subbi2 = ctx.new_temp(Type::I64);
        let t_subb1o = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5350);
        ctx.gen_addco(Type::I64, t_addco1, c_max, c_one);
        ctx.gen_mov(Type::I64, regs[10], t_addco1);
        ctx.gen_addci(Type::I64, t_addci1, c_five, c_six);
        ctx.gen_mov(Type::I64, regs[11], t_addci1);
        ctx.gen_addco(Type::I64, t_addco2, c_max, c_one);
        ctx.gen_mov(Type::I64, regs[12], t_addco2);
        ctx.gen_addcio(Type::I64, t_addcio, c_max, c_one);
        ctx.gen_mov(Type::I64, regs[13], t_addcio);
        ctx.gen_addci(Type::I64, t_addci2, c_five, c_six);
        ctx.gen_mov(Type::I64, regs[14], t_addci2);
        ctx.gen_addc1o(Type::I64, t_addc1o, c_five, c_six);
        ctx.gen_mov(Type::I64, regs[15], t_addc1o);

        ctx.gen_subbo(Type::I64, t_subbo, c_one, c_three);
        ctx.gen_mov(Type::I64, regs[16], t_subbo);
        ctx.gen_subbi(Type::I64, t_subbi1, c_five, c_three);
        ctx.gen_mov(Type::I64, regs[17], t_subbi1);
        ctx.gen_subbo(Type::I64, t_subbo, c_one, c_three);
        ctx.gen_subbio(Type::I64, t_subbio, c_one, c_three);
        ctx.gen_mov(Type::I64, regs[18], t_subbio);
        ctx.gen_subbi(Type::I64, t_subbi2, c_five, c_three);
        ctx.gen_mov(Type::I64, regs[19], t_subbi2);
        ctx.gen_subb1o(Type::I64, t_subb1o, c_five, c_three);
        ctx.gen_mov(Type::I64, regs[20], t_subb1o);

        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], 0);
    assert_eq!(cpu.regs[11], 12);
    assert_eq!(cpu.regs[12], 0);
    assert_eq!(cpu.regs[13], 1);
    assert_eq!(cpu.regs[14], 12);
    assert_eq!(cpu.regs[15], 12);
    assert_eq!(cpu.regs[16], 0xFFFF_FFFF_FFFF_FFFEu64);
    assert_eq!(cpu.regs[17], 1);
    assert_eq!(cpu.regs[18], 0xFFFF_FFFF_FFFF_FFFDu64);
    assert_eq!(cpu.regs[19], 1);
    assert_eq!(cpu.regs[20], 1);
}

#[test]
fn test_exec_negsetcond_movcond() {
    let mut cpu = RiscvCpuState::new();

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let c5 = ctx.new_const(Type::I64, 5);
        let c6 = ctx.new_const(Type::I64, 6);
        let v1a = ctx.new_const(Type::I64, 0x1111);
        let v2a = ctx.new_const(Type::I64, 0x2222);
        let v1b = ctx.new_const(Type::I64, 0xAAAA);
        let v2b = ctx.new_const(Type::I64, 0xBBBB);

        let t_nsc_true = ctx.new_temp(Type::I64);
        let t_nsc_false = ctx.new_temp(Type::I64);
        let t_mov_true = ctx.new_temp(Type::I64);
        let t_mov_false = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5360);
        ctx.gen_negsetcond(Type::I64, t_nsc_true, c5, c5, tcg_core::Cond::Eq);
        ctx.gen_mov(Type::I64, regs[10], t_nsc_true);
        ctx.gen_negsetcond(Type::I64, t_nsc_false, c5, c6, tcg_core::Cond::Eq);
        ctx.gen_mov(Type::I64, regs[11], t_nsc_false);

        ctx.gen_movcond(Type::I64, t_mov_true, c5, c5, v1a, v2a, tcg_core::Cond::Eq);
        ctx.gen_mov(Type::I64, regs[12], t_mov_true);
        ctx.gen_movcond(Type::I64, t_mov_false, c5, c6, v1b, v2b, tcg_core::Cond::Eq);
        ctx.gen_mov(Type::I64, regs[13], t_mov_false);

        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], 0u64.wrapping_sub(1));
    assert_eq!(cpu.regs[11], 0);
    assert_eq!(cpu.regs[12], 0x1111);
    assert_eq!(cpu.regs[13], 0xBBBB);
}

#[test]
fn test_exec_extrh_i64_i32() {
    let mut cpu = RiscvCpuStateMem::new();
    let value = 0x1122_3344_5566_7788u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, env, _regs, _pc| {
        let mem_offset = std::mem::offset_of!(RiscvCpuStateMem, mem) as i64;
        let c_val = ctx.new_const(Type::I64, value);
        let t_extrh = ctx.new_temp(Type::I32);

        ctx.gen_insn_start(0x5370);
        ctx.gen_extrh_i64_i32(t_extrh, c_val);
        ctx.gen_st32(Type::I32, t_extrh, env, mem_offset);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(
        u32::from_le_bytes(cpu.mem[0..4].try_into().unwrap()),
        0x1122_3344u32
    );
}

#[test]
fn test_exec_goto_ptr() {
    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);
    let (env, _regs, _pc) = setup_riscv_globals(&mut ctx);
    let mem_offset = std::mem::offset_of!(RiscvCpuStateMem, mem) as i64;

    let c_mark = ctx.new_const(Type::I64, 0x55);
    let c_after = ctx.new_const(Type::I64, 0xAA);
    let t_ptr = ctx.new_temp(Type::I64);

    ctx.gen_insn_start(0x5380);
    ctx.gen_st(Type::I64, c_mark, env, mem_offset + 8);
    ctx.gen_ld(Type::I64, t_ptr, env, mem_offset);
    ctx.gen_goto_ptr(t_ptr);
    ctx.gen_st(Type::I64, c_after, env, mem_offset + 16);
    ctx.gen_exit_tb(0x9999);

    let mut cpu = RiscvCpuStateMem::new();
    let target = buf.ptr_at(backend.epilogue_return_zero_offset) as u64;
    cpu.mem[0..8].copy_from_slice(&target.to_le_bytes());

    let exit_val = unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            &mut cpu as *mut RiscvCpuStateMem as *mut u8,
        )
    };

    assert_eq!(exit_val, 0);
    assert_eq!(
        u64::from_le_bytes(cpu.mem[8..16].try_into().unwrap()),
        0x55u64
    );
    assert_eq!(
        u64::from_le_bytes(cpu.mem[16..24].try_into().unwrap()),
        0u64
    );
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

// ==========================================================
// Additional IR TB cases
// ==========================================================

riscv_bin_case!(test_add_case_small, gen_add, 1u64, 2u64, 3u64);
riscv_bin_case!(
    test_add_case_wrap,
    gen_add,
    0xFFFF_FFFF_FFFF_FFFFu64,
    1u64,
    0u64
);
riscv_bin_case!(
    test_add_case_large,
    gen_add,
    0x1234_5678_9ABC_DEF0u64,
    0x1111_1111_1111_1111u64,
    0x2345_6789_ABCD_F001u64
);
riscv_bin_case!(
    test_add_case_carry,
    gen_add,
    0xFFFF_FFFF_FFFF_F000u64,
    0x1000u64,
    0u64
);
riscv_bin_case!(
    test_add_case_mixed,
    gen_add,
    0x8000_0000_0000_0000u64,
    0x7FFF_FFFF_FFFF_FFFFu64,
    0xFFFF_FFFF_FFFF_FFFFu64
);

riscv_bin_case!(test_sub_case_small, gen_sub, 10u64, 3u64, 7u64);
riscv_bin_case!(
    test_sub_case_wrap,
    gen_sub,
    0u64,
    1u64,
    0xFFFF_FFFF_FFFF_FFFFu64
);
riscv_bin_case!(
    test_sub_case_large,
    gen_sub,
    0x1234_0000_0000_0000u64,
    0x22u64,
    0x1233_FFFF_FFFF_FFDEu64
);
riscv_bin_case!(
    test_sub_case_neg,
    gen_sub,
    0x8000_0000_0000_0000u64,
    1u64,
    0x7FFF_FFFF_FFFF_FFFFu64
);
riscv_bin_case!(
    test_sub_case_equal,
    gen_sub,
    0xDEAD_BEEF_DEAD_BEEFu64,
    0xDEAD_BEEF_DEAD_BEEFu64,
    0u64
);

riscv_bin_case!(
    test_and_case_basic,
    gen_and,
    0xF0F0u64,
    0x0FF0u64,
    0x00F0u64
);
riscv_bin_case!(test_and_case_zero, gen_and, 0x1234u64, 0u64, 0u64);
riscv_bin_case!(
    test_and_case_high,
    gen_and,
    0xFFFF_0000_0000_FFFFu64,
    0x0F0F_F0F0_00FF_FF00u64,
    0x0F0F_0000_0000_FF00u64
);

riscv_bin_case!(test_or_case_basic, gen_or, 0xF0u64, 0x0Fu64, 0xFFu64);
riscv_bin_case!(
    test_or_case_zero,
    gen_or,
    0u64,
    0x1234_5678u64,
    0x1234_5678u64
);
riscv_bin_case!(
    test_or_case_high,
    gen_or,
    0x8000_0000_0000_0000u64,
    0x1u64,
    0x8000_0000_0000_0001u64
);

riscv_bin_case!(
    test_xor_case_small,
    gen_xor,
    0xFF00u64,
    0x00FFu64,
    0xFFFFu64
);
riscv_bin_case!(test_xor_case_self, gen_xor, 0x1234u64, 0x1234u64, 0u64);
riscv_bin_case!(test_xor_case_alt, gen_xor, 0xAAAAu64, 0x5555u64, 0xFFFFu64);
riscv_bin_case!(
    test_xor_case_large,
    gen_xor,
    0xFFFF_0000_0000_FFFFu64,
    0x0000_FFFF_0000_FFFFu64,
    0xFFFF_FFFF_0000_0000u64
);
riscv_bin_case!(
    test_xor_case_sign,
    gen_xor,
    0x8000_0000_0000_0000u64,
    0xFFFF_FFFF_FFFF_FFFFu64,
    0x7FFF_FFFF_FFFF_FFFFu64
);

riscv_bin_case!(test_mul_case_small, gen_mul, 6u64, 7u64, 42u64);
riscv_bin_case!(test_mul_case_zero, gen_mul, 0u64, 0x1234u64, 0u64);
riscv_bin_case!(
    test_mul_case_wrap,
    gen_mul,
    0xFFFF_FFFF_FFFF_FFFFu64,
    2u64,
    0xFFFF_FFFF_FFFF_FFFEu64
);
riscv_bin_case!(
    test_mul_case_large,
    gen_mul,
    0x1000_0000u64,
    0x1000u64,
    0x100_0000_0000u64
);
riscv_bin_case!(
    test_mul_case_mixed,
    gen_mul,
    0x1_0000_0000u64,
    3u64,
    0x3_0000_0000u64
);

riscv_shift_case!(test_shl_case_1, gen_shl, 0x1u64, 4u64, 0x10u64);
riscv_shift_case!(
    test_shl_case_2,
    gen_shl,
    0x1u64,
    63u64,
    0x8000_0000_0000_0000u64
);
riscv_shift_case!(
    test_shl_case_3,
    gen_shl,
    0x8000_0000_0000_0000u64,
    1u64,
    0u64
);
riscv_shift_case!(test_shl_case_4, gen_shl, 0x1234u64, 0u64, 0x1234u64);

riscv_shift_case!(test_shr_case_1, gen_shr, 0x10u64, 4u64, 0x1u64);
riscv_shift_case!(
    test_shr_case_2,
    gen_shr,
    0x8000_0000_0000_0000u64,
    63u64,
    0x1u64
);
riscv_shift_case!(
    test_shr_case_3,
    gen_shr,
    0xFFFF_0000_0000_0000u64,
    16u64,
    0x0000_FFFF_0000_0000u64
);

riscv_shift_case!(
    test_sar_case_1,
    gen_sar,
    0xFFFF_FFFF_FFFF_F000u64,
    4u64,
    0xFFFF_FFFF_FFFF_FF00u64
);
riscv_shift_case!(
    test_sar_case_2,
    gen_sar,
    0x7FFF_FFFF_FFFF_FFFFu64,
    63u64,
    0u64
);
riscv_shift_case!(
    test_sar_case_3,
    gen_sar,
    0x8000_0000_0000_0000u64,
    63u64,
    0xFFFF_FFFF_FFFF_FFFFu64
);

riscv_setcond_case!(test_setcond_eq_case, tcg_core::Cond::Eq, 5u64, 5u64, 1u64);
riscv_setcond_case!(test_setcond_ne_case, tcg_core::Cond::Ne, 5u64, 6u64, 1u64);
riscv_setcond_case!(
    test_setcond_lt_case,
    tcg_core::Cond::Lt,
    0xFFFF_FFFF_FFFF_FFFFu64,
    1u64,
    1u64
);
riscv_setcond_case!(
    test_setcond_ge_case,
    tcg_core::Cond::Ge,
    5u64,
    0xFFFF_FFFF_FFFF_FFFFu64,
    1u64
);
riscv_setcond_case!(
    test_setcond_le_case,
    tcg_core::Cond::Le,
    0xFFFF_FFFF_FFFF_FFFEu64,
    0xFFFF_FFFF_FFFF_FFFFu64,
    1u64
);
riscv_setcond_case!(test_setcond_gt_case, tcg_core::Cond::Gt, 2u64, 1u64, 1u64);
riscv_setcond_case!(
    test_setcond_ltu_case,
    tcg_core::Cond::Ltu,
    0xFFFF_FFFF_FFFF_FFFFu64,
    1u64,
    0u64
);
riscv_setcond_case!(
    test_setcond_geu_case,
    tcg_core::Cond::Geu,
    0xFFFF_FFFF_FFFF_FFFFu64,
    1u64,
    1u64
);
riscv_setcond_case!(
    test_setcond_leu_case,
    tcg_core::Cond::Leu,
    1u64,
    0xFFFF_FFFF_FFFF_FFFFu64,
    1u64
);
riscv_setcond_case!(
    test_setcond_gtu_case,
    tcg_core::Cond::Gtu,
    2u64,
    3u64,
    0u64
);

riscv_branch_case!(
    test_bne_taken_extra,
    tcg_core::Cond::Ne,
    10u64,
    11u64,
    1u64,
    2u64,
    1u64
);
riscv_branch_case!(
    test_bne_not_taken_extra,
    tcg_core::Cond::Ne,
    10u64,
    10u64,
    1u64,
    2u64,
    2u64
);
riscv_branch_case!(
    test_blt_taken_extra,
    tcg_core::Cond::Lt,
    0xFFFF_FFFF_FFFF_FFFEu64,
    1u64,
    3u64,
    4u64,
    3u64
);
riscv_branch_case!(
    test_bge_not_taken_extra,
    tcg_core::Cond::Ge,
    1u64,
    2u64,
    3u64,
    4u64,
    4u64
);
riscv_branch_case!(
    test_bltu_taken_extra,
    tcg_core::Cond::Ltu,
    1u64,
    2u64,
    5u64,
    6u64,
    5u64
);
riscv_branch_case!(
    test_bgeu_taken_extra,
    tcg_core::Cond::Geu,
    0xFFFF_FFFF_FFFF_FFFFu64,
    1u64,
    7u64,
    8u64,
    7u64
);

riscv_mem_case!(test_mem_case_0, 0i64, 0x1111_1111_1111_1111u64);
riscv_mem_case!(test_mem_case_8, 8i64, 0x2222_2222_2222_2222u64);
riscv_mem_case!(test_mem_case_16, 16i64, 0x3333_3333_3333_3333u64);
riscv_mem_case!(test_mem_case_24, 24i64, 0x4444_4444_4444_4444u64);
riscv_mem_case!(test_mem_case_32, 32i64, 0x5555_5555_5555_5555u64);
riscv_mem_case!(test_mem_case_40, 40i64, 0x6666_6666_6666_6666u64);

#[test]
fn test_complex_addi_andi_slli() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0x1234u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_add = ctx.new_temp(Type::I64);
        let t_and = ctx.new_temp(Type::I64);
        let t_shl = ctx.new_temp(Type::I64);
        let imm_add = ctx.new_const(Type::I64, 0x100u64);
        let mask = ctx.new_const(Type::I64, 0xFFu64);
        let shamt = ctx.new_const(Type::I64, 4u64);

        ctx.gen_insn_start(0x5000);
        ctx.gen_add(Type::I64, t_add, regs[1], imm_add);
        ctx.gen_and(Type::I64, t_and, t_add, mask);
        ctx.gen_shl(Type::I64, t_shl, t_and, shamt);
        ctx.gen_mov(Type::I64, regs[5], t_shl);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[5], 0x340u64);
}

#[test]
fn test_complex_mul_add_xor() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 7u64;
    cpu.regs[2] = 9u64;
    cpu.regs[3] = 5u64;
    cpu.regs[4] = 0xFFu64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_mul = ctx.new_temp(Type::I64);
        let t_add = ctx.new_temp(Type::I64);
        let t_xor = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5010);
        ctx.gen_mul(Type::I64, t_mul, regs[1], regs[2]);
        ctx.gen_add(Type::I64, t_add, t_mul, regs[3]);
        ctx.gen_xor(Type::I64, t_xor, t_add, regs[4]);
        ctx.gen_mov(Type::I64, regs[6], t_xor);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[6], (7u64 * 9u64 + 5u64) ^ 0xFFu64);
}

#[test]
fn test_complex_slt_branch_select() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0xFFFF_FFFF_FFFF_FFFEu64;
    cpu.regs[2] = 1u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let label_taken = ctx.new_label();
        let label_end = ctx.new_label();
        let t_cond = ctx.new_temp(Type::I64);
        let c_yes = ctx.new_const(Type::I64, 0x11u64);
        let c_no = ctx.new_const(Type::I64, 0x22u64);
        let t_yes = ctx.new_temp(Type::I64);
        let t_no = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5020);
        ctx.gen_setcond(
            Type::I64,
            t_cond,
            regs[1],
            regs[2],
            tcg_core::Cond::Lt,
        );
        ctx.gen_brcond(
            Type::I64,
            t_cond,
            regs[0],
            tcg_core::Cond::Ne,
            label_taken,
        );

        ctx.gen_mov(Type::I64, t_no, c_no);
        ctx.gen_mov(Type::I64, regs[7], t_no);
        ctx.gen_br(label_end);

        ctx.gen_set_label(label_taken);
        ctx.gen_mov(Type::I64, t_yes, c_yes);
        ctx.gen_mov(Type::I64, regs[7], t_yes);

        ctx.gen_set_label(label_end);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[7], 0x11u64);
}

#[test]
fn test_complex_auipc_addi() {
    let mut cpu = RiscvCpuState::new();
    cpu.pc = 0x2000u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, pc| {
        let imm20 = 0xABCDEu64;
        let imm = ctx.new_const(Type::I64, imm20 << 12);
        let addi = ctx.new_const(Type::I64, 0x123u64);
        let t_auipc = ctx.new_temp(Type::I64);
        let t_add = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5030);
        ctx.gen_add(Type::I64, t_auipc, pc, imm);
        ctx.gen_add(Type::I64, t_add, t_auipc, addi);
        ctx.gen_mov(Type::I64, regs[8], t_add);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(
        cpu.regs[8],
        cpu.pc.wrapping_add(0xABCDEu64 << 12).wrapping_add(0x123u64)
    );
}

#[test]
fn test_complex_bitfield_extract() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0xF0F0_F0F0_1234_5678u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_shr = ctx.new_temp(Type::I64);
        let t_and = ctx.new_temp(Type::I64);
        let shamt = ctx.new_const(Type::I64, 12u64);
        let mask = ctx.new_const(Type::I64, 0xFFFFu64);

        ctx.gen_insn_start(0x5040);
        ctx.gen_shr(Type::I64, t_shr, regs[1], shamt);
        ctx.gen_and(Type::I64, t_and, t_shr, mask);
        ctx.gen_mov(Type::I64, regs[9], t_and);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[9], 0x2345u64);
}

#[test]
fn test_complex_load_add_store() {
    let mut cpu = RiscvCpuStateMem::new();
    cpu.mem[0..8].copy_from_slice(&0x10u64.to_le_bytes());

    let exit_val = run_riscv_tb(&mut cpu, |ctx, env, regs, _pc| {
        let t_load = ctx.new_temp(Type::I64);
        let t_add = ctx.new_temp(Type::I64);
        let c_add = ctx.new_const(Type::I64, 0x20u64);
        let mem_offset = std::mem::offset_of!(RiscvCpuStateMem, mem) as i64;

        ctx.gen_insn_start(0x5050);
        ctx.gen_ld(Type::I64, t_load, env, mem_offset);
        ctx.gen_add(Type::I64, t_add, t_load, c_add);
        ctx.gen_st(Type::I64, t_add, env, mem_offset + 8);
        ctx.gen_mov(Type::I64, regs[10], t_add);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[10], 0x30u64);
    let stored = u64::from_le_bytes(cpu.mem[8..16].try_into().unwrap());
    assert_eq!(stored, 0x30u64);
}

#[test]
fn test_complex_shift_or() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0x1u64;
    cpu.regs[2] = 8u64;
    cpu.regs[3] = 0xFF00u64;
    cpu.regs[4] = 4u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_shl = ctx.new_temp(Type::I64);
        let t_shr = ctx.new_temp(Type::I64);
        let t_or = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5060);
        ctx.gen_shl(Type::I64, t_shl, regs[1], regs[2]);
        ctx.gen_shr(Type::I64, t_shr, regs[3], regs[4]);
        ctx.gen_or(Type::I64, t_or, t_shl, t_shr);
        ctx.gen_mov(Type::I64, regs[11], t_or);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[11], (0x1u64 << 8) | (0xFF00u64 >> 4));
}

#[test]
fn test_complex_xor_sub_and() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0xAAAAu64;
    cpu.regs[2] = 0x5555u64;
    cpu.regs[3] = 0xFF00u64;
    cpu.regs[4] = 0x0F0Fu64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_xor = ctx.new_temp(Type::I64);
        let t_and = ctx.new_temp(Type::I64);
        let t_sub = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5070);
        ctx.gen_xor(Type::I64, t_xor, regs[1], regs[2]);
        ctx.gen_and(Type::I64, t_and, regs[3], regs[4]);
        ctx.gen_sub(Type::I64, t_sub, t_xor, t_and);
        ctx.gen_mov(Type::I64, regs[12], t_sub);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(
        cpu.regs[12],
        (0xAAAAu64 ^ 0x5555u64).wrapping_sub(0xFF00u64 & 0x0F0Fu64)
    );
}

#[test]
fn test_complex_branch_fallthrough() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0x10u64;
    cpu.regs[2] = 0x10u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let label_taken = ctx.new_label();
        let label_end = ctx.new_label();
        let c1 = ctx.new_const(Type::I64, 1u64);
        let c2 = ctx.new_const(Type::I64, 2u64);
        let t1 = ctx.new_temp(Type::I64);
        let t2 = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5080);
        ctx.gen_brcond(
            Type::I64,
            regs[1],
            regs[2],
            tcg_core::Cond::Ne,
            label_taken,
        );
        ctx.gen_mov(Type::I64, t1, c1);
        ctx.gen_mov(Type::I64, regs[13], t1);
        ctx.gen_br(label_end);

        ctx.gen_set_label(label_taken);
        ctx.gen_mov(Type::I64, t2, c2);
        ctx.gen_mov(Type::I64, regs[13], t2);

        ctx.gen_set_label(label_end);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[13], 1u64);
}

#[test]
fn test_complex_pc_relative_mask() {
    let mut cpu = RiscvCpuState::new();
    cpu.pc = 0x8000u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, pc| {
        let imm = ctx.new_const(Type::I64, 0x100u64);
        let mask = ctx.new_const(Type::I64, 0xFFFu64);
        let t_add = ctx.new_temp(Type::I64);
        let t_and = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5090);
        ctx.gen_add(Type::I64, t_add, pc, imm);
        ctx.gen_and(Type::I64, t_and, t_add, mask);
        ctx.gen_mov(Type::I64, regs[14], t_and);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[14], (cpu.pc + 0x100u64) & 0xFFFu64);
}

#[test]
fn test_neg_basic() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0x1234u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_neg = ctx.new_temp(Type::I64);
        ctx.gen_insn_start(0x5100);
        ctx.gen_neg(Type::I64, t_neg, regs[1]);
        ctx.gen_mov(Type::I64, regs[15], t_neg);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[15], 0u64.wrapping_sub(0x1234u64));
}

#[test]
fn test_not_basic() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0x00FF_00FF_00FF_00FFu64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_not = ctx.new_temp(Type::I64);
        ctx.gen_insn_start(0x5110);
        ctx.gen_not(Type::I64, t_not, regs[1]);
        ctx.gen_mov(Type::I64, regs[16], t_not);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[16], !0x00FF_00FF_00FF_00FFu64);
}

#[test]
fn test_mov_chain() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0xA5A5_5AA5_A5A5_5AA5u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        ctx.gen_insn_start(0x5120);
        ctx.gen_mov(Type::I64, regs[2], regs[1]);
        ctx.gen_mov(Type::I64, regs[3], regs[2]);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[2], cpu.regs[1]);
    assert_eq!(cpu.regs[3], cpu.regs[1]);
}

#[test]
fn test_brcond_on_temp_eq() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 10u64;
    cpu.regs[2] = 20u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let label_eq = ctx.new_label();
        let label_end = ctx.new_label();
        let t_add = ctx.new_temp(Type::I64);
        let c30 = ctx.new_const(Type::I64, 30u64);
        let c1 = ctx.new_const(Type::I64, 1u64);
        let c0 = ctx.new_const(Type::I64, 0u64);
        let t_out = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5130);
        ctx.gen_add(Type::I64, t_add, regs[1], regs[2]);
        ctx.gen_brcond(Type::I64, t_add, c30, tcg_core::Cond::Eq, label_eq);
        ctx.gen_mov(Type::I64, t_out, c0);
        ctx.gen_mov(Type::I64, regs[4], t_out);
        ctx.gen_br(label_end);

        ctx.gen_set_label(label_eq);
        ctx.gen_mov(Type::I64, t_out, c1);
        ctx.gen_mov(Type::I64, regs[4], t_out);
        ctx.gen_set_label(label_end);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[4], 1u64);
}

#[test]
fn test_countdown_loop_sum() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 4u64;
    cpu.regs[2] = 0u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let label_loop = ctx.new_label();
        let c1 = ctx.new_const(Type::I64, 1u64);
        let c0 = ctx.new_const(Type::I64, 0u64);
        let t_sum = ctx.new_temp(Type::I64);
        let t_cnt = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5140);
        ctx.gen_set_label(label_loop);
        ctx.gen_add(Type::I64, t_sum, regs[2], regs[1]);
        ctx.gen_mov(Type::I64, regs[2], t_sum);
        ctx.gen_sub(Type::I64, t_cnt, regs[1], c1);
        ctx.gen_mov(Type::I64, regs[1], t_cnt);
        ctx.gen_brcond(Type::I64, regs[1], c0, tcg_core::Cond::Ne, label_loop);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[2], 10u64);
    assert_eq!(cpu.regs[1], 0u64);
}

#[test]
fn test_mem_store_overwrite() {
    let mut cpu = RiscvCpuStateMem::new();
    let exit_val = run_riscv_tb(&mut cpu, |ctx, env, regs, _pc| {
        let v1 = ctx.new_const(Type::I64, 0x1111_2222_3333_4444u64);
        let v2 = ctx.new_const(Type::I64, 0xAAAA_BBBB_CCCC_DDDDu64);
        let t1 = ctx.new_temp(Type::I64);
        let t2 = ctx.new_temp(Type::I64);
        let t_load = ctx.new_temp(Type::I64);
        let mem_offset =
            std::mem::offset_of!(RiscvCpuStateMem, mem) as i64 + 16;

        ctx.gen_insn_start(0x5150);
        ctx.gen_mov(Type::I64, t1, v1);
        ctx.gen_st(Type::I64, t1, env, mem_offset);
        ctx.gen_mov(Type::I64, t2, v2);
        ctx.gen_st(Type::I64, t2, env, mem_offset);
        ctx.gen_ld(Type::I64, t_load, env, mem_offset);
        ctx.gen_mov(Type::I64, regs[1], t_load);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[1], 0xAAAA_BBBB_CCCC_DDDDu64);
    let stored = u64::from_le_bytes(cpu.mem[16..24].try_into().unwrap());
    assert_eq!(stored, 0xAAAA_BBBB_CCCC_DDDDu64);
}

#[test]
fn test_mem_load_add_sum() {
    let mut cpu = RiscvCpuStateMem::new();
    cpu.mem[0..8].copy_from_slice(&0x10u64.to_le_bytes());
    cpu.mem[8..16].copy_from_slice(&0x20u64.to_le_bytes());

    let exit_val = run_riscv_tb(&mut cpu, |ctx, env, regs, _pc| {
        let t0 = ctx.new_temp(Type::I64);
        let t1 = ctx.new_temp(Type::I64);
        let t_sum = ctx.new_temp(Type::I64);
        let mem_offset = std::mem::offset_of!(RiscvCpuStateMem, mem) as i64;

        ctx.gen_insn_start(0x5160);
        ctx.gen_ld(Type::I64, t0, env, mem_offset);
        ctx.gen_ld(Type::I64, t1, env, mem_offset + 8);
        ctx.gen_add(Type::I64, t_sum, t0, t1);
        ctx.gen_mov(Type::I64, regs[2], t_sum);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[2], 0x30u64);
}

#[test]
fn test_shift_count_computed() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 0x2u64;
    cpu.regs[2] = 3u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_cnt = ctx.new_temp(Type::I64);
        let t_out = ctx.new_temp(Type::I64);
        let c1 = ctx.new_const(Type::I64, 1u64);

        ctx.gen_insn_start(0x5170);
        ctx.gen_add(Type::I64, t_cnt, regs[2], c1);
        ctx.gen_shl(Type::I64, t_out, regs[1], t_cnt);
        ctx.gen_mov(Type::I64, regs[5], t_out);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[5], 0x2u64 << 4);
}

#[test]
fn test_mul_sub_mix() {
    let mut cpu = RiscvCpuState::new();
    cpu.regs[1] = 9u64;
    cpu.regs[2] = 7u64;
    cpu.regs[3] = 10u64;

    let exit_val = run_riscv_tb(&mut cpu, |ctx, _env, regs, _pc| {
        let t_mul = ctx.new_temp(Type::I64);
        let t_sub = ctx.new_temp(Type::I64);

        ctx.gen_insn_start(0x5180);
        ctx.gen_mul(Type::I64, t_mul, regs[1], regs[2]);
        ctx.gen_sub(Type::I64, t_sub, t_mul, regs[3]);
        ctx.gen_mov(Type::I64, regs[6], t_sub);
        ctx.gen_exit_tb(0);
    });

    assert_eq!(exit_val, 0);
    assert_eq!(cpu.regs[6], (9u64 * 7u64).wrapping_sub(10u64));
}
