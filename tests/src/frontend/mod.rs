//! Frontend translation tests — encode real RISC-V instructions,
//! run them through the full frontend→backend pipeline, and verify
//! the resulting CPU state.

mod difftest;

use tcg_backend::code_buffer::CodeBuffer;
use tcg_backend::translate::translate_and_execute;
use tcg_backend::HostCodeGen;
use tcg_backend::X86_64CodeGen;
use tcg_core::Context;
use tcg_frontend::riscv::cpu::RiscvCpu;
use tcg_frontend::riscv::{RiscvDisasContext, RiscvTranslator};
use tcg_frontend::translator_loop;

// ── Instruction encoding helpers ──────────────────────────────

fn rv_r(f7: u32, rs2: u32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
    (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}

fn rv_i(imm: i32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
    let imm = (imm as u32) & 0xFFF;
    (imm << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}

fn rv_u(imm: i32, rd: u32, op: u32) -> u32 {
    ((imm as u32) & 0xFFFF_F000) | (rd << 7) | op
}

fn rv_b(imm: i32, rs2: u32, rs1: u32, f3: u32) -> u32 {
    let i = imm as u32;
    let b12 = (i >> 12) & 1;
    let b11 = (i >> 11) & 1;
    let b10_5 = (i >> 5) & 0x3F;
    let b4_1 = (i >> 1) & 0xF;
    (b12 << 31)
        | (b10_5 << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (f3 << 12)
        | (b4_1 << 8)
        | (b11 << 7)
        | 0b1100011
}

fn rv_j(imm: i32, rd: u32) -> u32 {
    let i = imm as u32;
    let b20 = (i >> 20) & 1;
    let b10_1 = (i >> 1) & 0x3FF;
    let b11 = (i >> 11) & 1;
    let b19_12 = (i >> 12) & 0xFF;
    (b20 << 31)
        | (b10_1 << 21)
        | (b11 << 20)
        | (b19_12 << 12)
        | (rd << 7)
        | 0b1101111
}

// ── Specific instruction encoders ─────────────────────────────

const OP_LUI: u32 = 0b0110111;
const OP_AUIPC: u32 = 0b0010111;
const OP_IMM: u32 = 0b0010011;
const OP_REG: u32 = 0b0110011;
const OP_IMM32: u32 = 0b0011011;
const OP_REG32: u32 = 0b0111011;

fn lui(rd: u32, imm: i32) -> u32 {
    rv_u(imm, rd, OP_LUI)
}
fn auipc(rd: u32, imm: i32) -> u32 {
    rv_u(imm, rd, OP_AUIPC)
}
fn jal(rd: u32, imm: i32) -> u32 {
    rv_j(imm, rd)
}
fn jalr(rd: u32, rs1: u32, imm: i32) -> u32 {
    rv_i(imm, rs1, 0b000, rd, 0b1100111)
}
fn beq(rs1: u32, rs2: u32, imm: i32) -> u32 {
    rv_b(imm, rs2, rs1, 0b000)
}
fn bne(rs1: u32, rs2: u32, imm: i32) -> u32 {
    rv_b(imm, rs2, rs1, 0b001)
}
fn blt(rs1: u32, rs2: u32, imm: i32) -> u32 {
    rv_b(imm, rs2, rs1, 0b100)
}
fn bge(rs1: u32, rs2: u32, imm: i32) -> u32 {
    rv_b(imm, rs2, rs1, 0b101)
}
fn bltu(rs1: u32, rs2: u32, imm: i32) -> u32 {
    rv_b(imm, rs2, rs1, 0b110)
}
fn bgeu(rs1: u32, rs2: u32, imm: i32) -> u32 {
    rv_b(imm, rs2, rs1, 0b111)
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    rv_i(imm, rs1, 0b000, rd, OP_IMM)
}
fn slti(rd: u32, rs1: u32, imm: i32) -> u32 {
    rv_i(imm, rs1, 0b010, rd, OP_IMM)
}
fn sltiu(rd: u32, rs1: u32, imm: i32) -> u32 {
    rv_i(imm, rs1, 0b011, rd, OP_IMM)
}
fn xori(rd: u32, rs1: u32, imm: i32) -> u32 {
    rv_i(imm, rs1, 0b100, rd, OP_IMM)
}
fn ori(rd: u32, rs1: u32, imm: i32) -> u32 {
    rv_i(imm, rs1, 0b110, rd, OP_IMM)
}
fn andi(rd: u32, rs1: u32, imm: i32) -> u32 {
    rv_i(imm, rs1, 0b111, rd, OP_IMM)
}
fn slli(rd: u32, rs1: u32, sh: u32) -> u32 {
    rv_r(0b0000000, sh, rs1, 0b001, rd, OP_IMM)
}
fn srli(rd: u32, rs1: u32, sh: u32) -> u32 {
    rv_r(0b0000000, sh, rs1, 0b101, rd, OP_IMM)
}
fn srai(rd: u32, rs1: u32, sh: u32) -> u32 {
    rv_r(0b0100000, sh, rs1, 0b101, rd, OP_IMM)
}
fn add(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b000, rd, OP_REG)
}
fn sub(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0b0100000, rs2, rs1, 0b000, rd, OP_REG)
}
fn sll(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b001, rd, OP_REG)
}
fn slt(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b010, rd, OP_REG)
}
fn sltu(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b011, rd, OP_REG)
}
fn xor(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b100, rd, OP_REG)
}
fn srl(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b101, rd, OP_REG)
}
fn sra(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0b0100000, rs2, rs1, 0b101, rd, OP_REG)
}
fn or(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b110, rd, OP_REG)
}
fn and(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b111, rd, OP_REG)
}
fn fence() -> u32 {
    0x0ff0_000f
}
fn ecall() -> u32 {
    0x0000_0073
}
fn ebreak() -> u32 {
    0x0010_0073
}
// RV64I W-suffix
fn addiw(rd: u32, rs1: u32, imm: i32) -> u32 {
    rv_i(imm, rs1, 0b000, rd, OP_IMM32)
}
fn slliw(rd: u32, rs1: u32, sh: u32) -> u32 {
    rv_r(0, sh, rs1, 0b001, rd, OP_IMM32)
}
fn srliw(rd: u32, rs1: u32, sh: u32) -> u32 {
    rv_r(0, sh, rs1, 0b101, rd, OP_IMM32)
}
fn sraiw(rd: u32, rs1: u32, sh: u32) -> u32 {
    rv_r(0b0100000, sh, rs1, 0b101, rd, OP_IMM32)
}
fn addw(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b000, rd, OP_REG32)
}
fn subw(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0b0100000, rs2, rs1, 0b000, rd, OP_REG32)
}
fn sllw(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b001, rd, OP_REG32)
}
fn srlw(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0, rs2, rs1, 0b101, rd, OP_REG32)
}
fn sraw(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0b0100000, rs2, rs1, 0b101, rd, OP_REG32)
}

// ── Test runner ───────────────────────────────────────────────

/// Translate one RISC-V instruction at PC=0 and execute it.
fn run_rv(cpu: &mut RiscvCpu, insn: u32) -> usize {
    run_rv_insns(cpu, &[insn])
}

/// Translate a sequence of RISC-V instructions starting at
/// PC=0 and execute the resulting TB.
fn run_rv_insns(cpu: &mut RiscvCpu, insns: &[u32]) -> usize {
    let code: Vec<u8> = insns.iter().flat_map(|i| i.to_le_bytes()).collect();
    let guest_base = code.as_ptr();

    let mut backend = X86_64CodeGen::new();
    let mut buf = CodeBuffer::new(4096).unwrap();
    backend.emit_prologue(&mut buf);
    backend.emit_epilogue(&mut buf);

    let mut ctx = Context::new();
    backend.init_context(&mut ctx);

    let mut disas = RiscvDisasContext::new(0, guest_base);
    disas.base.max_insns = insns.len() as u32;
    translator_loop::<RiscvTranslator>(&mut disas, &mut ctx);

    unsafe {
        translate_and_execute(
            &mut ctx,
            &backend,
            &mut buf,
            cpu as *mut RiscvCpu as *mut u8,
        )
    }
}

// ── RV32I: Upper immediate ────────────────────────────────────

#[test]
fn test_lui() {
    let mut cpu = RiscvCpu::new();
    run_rv(&mut cpu, lui(1, 0x12345_000u32 as i32));
    assert_eq!(cpu.gpr[1], 0x12345000);
}

#[test]
fn test_lui_negative() {
    let mut cpu = RiscvCpu::new();
    run_rv(&mut cpu, lui(1, 0xFFFFF_000u32 as i32));
    assert_eq!(cpu.gpr[1], 0xFFFF_FFFF_FFFF_F000);
}

#[test]
fn test_auipc() {
    let mut cpu = RiscvCpu::new();
    // PC=0, auipc x1, 0x2000 → x1 = 0 + 0x2000000
    run_rv(&mut cpu, auipc(1, 0x2000_000u32 as i32));
    assert_eq!(cpu.gpr[1], 0x0200_0000);
}

// ── RV32I: Jumps ──────────────────────────────────────────────

#[test]
fn test_jal() {
    let mut cpu = RiscvCpu::new();
    // jal x1, 8 → x1 = PC+4 = 4, PC = 0+8 = 8
    run_rv(&mut cpu, jal(1, 8));
    assert_eq!(cpu.gpr[1], 4);
    assert_eq!(cpu.pc, 8);
}

#[test]
fn test_jalr() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[2] = 0x1000;
    // jalr x1, x2, 4 → x1 = PC+4 = 4, PC = (0x1000+4)&~1 = 0x1004
    run_rv(&mut cpu, jalr(1, 2, 4));
    assert_eq!(cpu.gpr[1], 4);
    assert_eq!(cpu.pc, 0x1004);
}

// ── RV32I: Branches ───────────────────────────────────────────

#[test]
fn test_beq_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 42;
    cpu.gpr[2] = 42;
    run_rv(&mut cpu, beq(1, 2, 16));
    assert_eq!(cpu.pc, 16); // taken
}

#[test]
fn test_beq_not_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 42;
    cpu.gpr[2] = 43;
    run_rv(&mut cpu, beq(1, 2, 16));
    assert_eq!(cpu.pc, 4); // not taken
}

#[test]
fn test_bne_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 1;
    cpu.gpr[2] = 2;
    run_rv(&mut cpu, bne(1, 2, 20));
    assert_eq!(cpu.pc, 20);
}

#[test]
fn test_bne_not_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 5;
    cpu.gpr[2] = 5;
    run_rv(&mut cpu, bne(1, 2, 20));
    assert_eq!(cpu.pc, 4);
}

#[test]
fn test_blt_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = (-1i64) as u64; // -1 < 0
    cpu.gpr[2] = 0;
    run_rv(&mut cpu, blt(1, 2, 12));
    assert_eq!(cpu.pc, 12);
}

#[test]
fn test_blt_not_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 5;
    cpu.gpr[2] = 3;
    run_rv(&mut cpu, blt(1, 2, 12));
    assert_eq!(cpu.pc, 4);
}

#[test]
fn test_bge_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 10;
    cpu.gpr[2] = 10;
    run_rv(&mut cpu, bge(1, 2, 8));
    assert_eq!(cpu.pc, 8);
}

#[test]
fn test_bge_not_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = (-5i64) as u64;
    cpu.gpr[2] = 0;
    run_rv(&mut cpu, bge(1, 2, 8));
    assert_eq!(cpu.pc, 4);
}

#[test]
fn test_bltu_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 3;
    cpu.gpr[2] = 100;
    run_rv(&mut cpu, bltu(1, 2, 16));
    assert_eq!(cpu.pc, 16);
}

#[test]
fn test_bltu_not_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = (-1i64) as u64; // max unsigned
    cpu.gpr[2] = 5;
    run_rv(&mut cpu, bltu(1, 2, 16));
    assert_eq!(cpu.pc, 4);
}

#[test]
fn test_bgeu_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = (-1i64) as u64;
    cpu.gpr[2] = 5;
    run_rv(&mut cpu, bgeu(1, 2, 24));
    assert_eq!(cpu.pc, 24);
}

#[test]
fn test_bgeu_not_taken() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 3;
    cpu.gpr[2] = 100;
    run_rv(&mut cpu, bgeu(1, 2, 24));
    assert_eq!(cpu.pc, 4);
}

// ── RV32I: ALU immediate ──────────────────────────────────────

#[test]
fn test_addi() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 100;
    run_rv(&mut cpu, addi(3, 1, 42));
    assert_eq!(cpu.gpr[3], 142);
}

#[test]
fn test_addi_negative() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 100;
    run_rv(&mut cpu, addi(3, 1, -10));
    assert_eq!(cpu.gpr[3], 90);
}

#[test]
fn test_slti_true() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = (-5i64) as u64;
    run_rv(&mut cpu, slti(3, 1, 0));
    assert_eq!(cpu.gpr[3], 1);
}

#[test]
fn test_slti_false() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 10;
    run_rv(&mut cpu, slti(3, 1, 5));
    assert_eq!(cpu.gpr[3], 0);
}

#[test]
fn test_sltiu_true() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 3;
    run_rv(&mut cpu, sltiu(3, 1, 10));
    assert_eq!(cpu.gpr[3], 1);
}

#[test]
fn test_sltiu_false() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 100;
    run_rv(&mut cpu, sltiu(3, 1, 10));
    assert_eq!(cpu.gpr[3], 0);
}

#[test]
fn test_xori() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0xFF;
    run_rv(&mut cpu, xori(3, 1, 0x0F));
    assert_eq!(cpu.gpr[3], 0xF0);
}

#[test]
fn test_ori() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0xF0;
    run_rv(&mut cpu, ori(3, 1, 0x0F));
    assert_eq!(cpu.gpr[3], 0xFF);
}

#[test]
fn test_andi() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0xFF;
    run_rv(&mut cpu, andi(3, 1, 0x0F));
    assert_eq!(cpu.gpr[3], 0x0F);
}

// ── RV32I: Shift immediate ────────────────────────────────────

#[test]
fn test_slli() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 1;
    run_rv(&mut cpu, slli(3, 1, 4));
    assert_eq!(cpu.gpr[3], 16);
}

#[test]
fn test_srli() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0x80;
    run_rv(&mut cpu, srli(3, 1, 4));
    assert_eq!(cpu.gpr[3], 0x08);
}

#[test]
fn test_srai() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = (-16i64) as u64;
    run_rv(&mut cpu, srai(3, 1, 2));
    assert_eq!(cpu.gpr[3], (-4i64) as u64);
}

// ── RV32I: R-type ALU ─────────────────────────────────────────

#[test]
fn test_add() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 100;
    cpu.gpr[2] = 200;
    run_rv(&mut cpu, add(3, 1, 2));
    assert_eq!(cpu.gpr[3], 300);
}

#[test]
fn test_sub() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 200;
    cpu.gpr[2] = 50;
    run_rv(&mut cpu, sub(3, 1, 2));
    assert_eq!(cpu.gpr[3], 150);
}

#[test]
fn test_sll() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 1;
    cpu.gpr[2] = 8;
    run_rv(&mut cpu, sll(3, 1, 2));
    assert_eq!(cpu.gpr[3], 256);
}

#[test]
fn test_slt_true() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = (-1i64) as u64;
    cpu.gpr[2] = 0;
    run_rv(&mut cpu, slt(3, 1, 2));
    assert_eq!(cpu.gpr[3], 1);
}

#[test]
fn test_slt_false() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 5;
    cpu.gpr[2] = 3;
    run_rv(&mut cpu, slt(3, 1, 2));
    assert_eq!(cpu.gpr[3], 0);
}

#[test]
fn test_sltu_true() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 3;
    cpu.gpr[2] = 100;
    run_rv(&mut cpu, sltu(3, 1, 2));
    assert_eq!(cpu.gpr[3], 1);
}

#[test]
fn test_sltu_false() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = (-1i64) as u64;
    cpu.gpr[2] = 5;
    run_rv(&mut cpu, sltu(3, 1, 2));
    assert_eq!(cpu.gpr[3], 0);
}

#[test]
fn test_xor() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0xFF00;
    cpu.gpr[2] = 0x0FF0;
    run_rv(&mut cpu, xor(3, 1, 2));
    assert_eq!(cpu.gpr[3], 0xF0F0);
}

#[test]
fn test_srl() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0x100;
    cpu.gpr[2] = 4;
    run_rv(&mut cpu, srl(3, 1, 2));
    assert_eq!(cpu.gpr[3], 0x10);
}

#[test]
fn test_sra() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = (-64i64) as u64;
    cpu.gpr[2] = 3;
    run_rv(&mut cpu, sra(3, 1, 2));
    assert_eq!(cpu.gpr[3], (-8i64) as u64);
}

#[test]
fn test_or() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0xF0;
    cpu.gpr[2] = 0x0F;
    run_rv(&mut cpu, or(3, 1, 2));
    assert_eq!(cpu.gpr[3], 0xFF);
}

#[test]
fn test_and() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0xFF;
    cpu.gpr[2] = 0x0F;
    run_rv(&mut cpu, and(3, 1, 2));
    assert_eq!(cpu.gpr[3], 0x0F);
}

// ── RV32I: Fence / System ─────────────────────────────────────

#[test]
fn test_fence_nop() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 42;
    // fence is a NOP; the TB falls through to tb_stop
    run_rv(&mut cpu, fence());
    assert_eq!(cpu.gpr[1], 42); // unchanged
}

#[test]
fn test_ecall_exit() {
    let mut cpu = RiscvCpu::new();
    let exit = run_rv(&mut cpu, ecall());
    assert_eq!(exit, 1);
    assert_eq!(cpu.pc, 0); // PC synced to insn PC
}

#[test]
fn test_ebreak_exit() {
    let mut cpu = RiscvCpu::new();
    let exit = run_rv(&mut cpu, ebreak());
    assert_eq!(exit, 2);
    assert_eq!(cpu.pc, 0);
}

// ── RV64I: W-suffix ALU ───────────────────────────────────────

#[test]
fn test_addiw() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 100;
    run_rv(&mut cpu, addiw(3, 1, 50));
    assert_eq!(cpu.gpr[3], 150);
}

#[test]
fn test_addiw_sign_extend() {
    let mut cpu = RiscvCpu::new();
    // 0x7FFF_FFFF + 1 = 0x8000_0000 → sext = 0xFFFF_FFFF_8000_0000
    cpu.gpr[1] = 0x7FFF_FFFF;
    run_rv(&mut cpu, addiw(3, 1, 1));
    assert_eq!(cpu.gpr[3], 0xFFFF_FFFF_8000_0000u64);
}

#[test]
fn test_slliw() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 1;
    run_rv(&mut cpu, slliw(3, 1, 31));
    // 1 << 31 = 0x8000_0000 → sext = 0xFFFF_FFFF_8000_0000
    assert_eq!(cpu.gpr[3], 0xFFFF_FFFF_8000_0000u64);
}

#[test]
fn test_srliw() {
    let mut cpu = RiscvCpu::new();
    // Only low 32 bits matter: 0xFFFF_FFFF → >> 16 = 0x0000_FFFF
    cpu.gpr[1] = 0xFFFF_FFFF_FFFF_FFFFu64;
    run_rv(&mut cpu, srliw(3, 1, 16));
    assert_eq!(cpu.gpr[3], 0x0000_FFFF);
}

#[test]
fn test_sraiw() {
    let mut cpu = RiscvCpu::new();
    // Low 32 bits = 0x8000_0000 (negative in i32)
    cpu.gpr[1] = 0x0000_0000_8000_0000u64;
    run_rv(&mut cpu, sraiw(3, 1, 4));
    // i32: 0x8000_0000 >> 4 = 0xF800_0000 → sext
    assert_eq!(cpu.gpr[3], 0xFFFF_FFFF_F800_0000u64);
}

#[test]
fn test_addw() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0x7FFF_FFFF;
    cpu.gpr[2] = 1;
    run_rv(&mut cpu, addw(3, 1, 2));
    assert_eq!(cpu.gpr[3], 0xFFFF_FFFF_8000_0000u64);
}

#[test]
fn test_subw() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0;
    cpu.gpr[2] = 1;
    run_rv(&mut cpu, subw(3, 1, 2));
    // 0 - 1 = 0xFFFF_FFFF → sext = 0xFFFF_FFFF_FFFF_FFFF
    assert_eq!(cpu.gpr[3], 0xFFFF_FFFF_FFFF_FFFFu64);
}

#[test]
fn test_sllw() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0xFF;
    cpu.gpr[2] = 24;
    run_rv(&mut cpu, sllw(3, 1, 2));
    // 0xFF << 24 = 0xFF00_0000 → sext = 0xFFFF_FFFF_FF00_0000
    assert_eq!(cpu.gpr[3], 0xFFFF_FFFF_FF00_0000u64);
}

#[test]
fn test_srlw() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0xFFFF_FFFF_FFFF_FFFFu64;
    cpu.gpr[2] = 16;
    run_rv(&mut cpu, srlw(3, 1, 2));
    // low32 = 0xFFFF_FFFF >> 16 = 0x0000_FFFF → sext positive
    assert_eq!(cpu.gpr[3], 0x0000_FFFF);
}

#[test]
fn test_sraw() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 0x8000_0000u64; // negative in i32
    cpu.gpr[2] = 4;
    run_rv(&mut cpu, sraw(3, 1, 2));
    assert_eq!(cpu.gpr[3], 0xFFFF_FFFF_F800_0000u64);
}

// ── x0 hardwired zero ─────────────────────────────────────────

#[test]
fn test_x0_write_ignored() {
    let mut cpu = RiscvCpu::new();
    cpu.gpr[1] = 42;
    // addi x0, x1, 100 → should NOT change x0
    run_rv(&mut cpu, addi(0, 1, 100));
    assert_eq!(cpu.gpr[0], 0);
}

#[test]
fn test_x0_read_zero() {
    let mut cpu = RiscvCpu::new();
    // addi x3, x0, 77 → x3 = 0 + 77 = 77
    run_rv(&mut cpu, addi(3, 0, 77));
    assert_eq!(cpu.gpr[3], 77);
}

// ── Multi-instruction sequences ───────────────────────────────

#[test]
fn test_addi_addi_sequence() {
    let mut cpu = RiscvCpu::new();
    // addi x1, x0, 10; addi x2, x1, 20
    run_rv_insns(&mut cpu, &[addi(1, 0, 10), addi(2, 1, 20)]);
    assert_eq!(cpu.gpr[1], 10);
    assert_eq!(cpu.gpr[2], 30);
}

#[test]
fn test_lui_addi_combo() {
    let mut cpu = RiscvCpu::new();
    // lui x1, 0x12345000; addi x1, x1, 0x678
    run_rv_insns(
        &mut cpu,
        &[lui(1, 0x12345_000u32 as i32), addi(1, 1, 0x678)],
    );
    assert_eq!(cpu.gpr[1], 0x12345678);
}
