//! RISC-V disassembler — RV64IMAC.
//!
//! Mirrors QEMU's `disas/riscv.c`. Covers RV64I base integer,
//! M (multiply/divide), A (atomics), and C (compressed) extensions.

// -- Register ABI names --

const REG_ABI: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1",
    "a2", "a3", "a4", "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7",
    "s8", "s9", "s10", "s11", "t3", "t4", "t5", "t6",
];

fn reg(r: u32) -> &'static str {
    REG_ABI[(r & 0x1f) as usize]
}

/// Compressed register (3-bit, maps to x8–x15).
fn creg(r: u32) -> &'static str {
    REG_ABI[(8 + (r & 0x7)) as usize]
}

fn sign_ext(val: u32, bits: u32) -> i64 {
    let shift = 32 - bits;
    ((val << shift) as i32 >> shift) as i64
}

/// Disassemble one RISC-V instruction at `pc`.
///
/// `data` must contain at least 2 bytes (4 for non-compressed).
/// Returns `(assembly_text, instruction_length_in_bytes)`.
///
/// This is the public entry point, analogous to QEMU's
/// `print_insn_riscv64()`.
pub fn print_insn_riscv64(pc: u64, data: &[u8]) -> (String, usize) {
    if data.len() < 2 {
        return (".byte ???".into(), 0);
    }
    let half = u16::from_le_bytes([data[0], data[1]]);
    if half & 0x3 != 0x3 {
        (disasm16(half as u32, pc), 2)
    } else {
        if data.len() < 4 {
            return (".byte ???".into(), 0);
        }
        let insn = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        (disasm32(insn, pc), 4)
    }
}

// ================================================================
// 32-bit instruction disassembly
// ================================================================

fn disasm32(insn: u32, pc: u64) -> String {
    let opcode = insn & 0x7f;
    let rd = (insn >> 7) & 0x1f;
    let funct3 = (insn >> 12) & 0x7;
    let rs1 = (insn >> 15) & 0x1f;
    let rs2 = (insn >> 20) & 0x1f;
    let funct7 = insn >> 25;

    match opcode {
        0x37 => {
            let imm = (insn & 0xfffff000) as i32;
            format!("lui {}, {:#x}", reg(rd), imm >> 12)
        }
        0x17 => {
            let imm = (insn & 0xfffff000) as i32 as i64;
            let target = pc.wrapping_add(imm as u64);
            format!(
                "auipc {}, {:#x}  # {target:#x}",
                reg(rd),
                (imm >> 12) & 0xfffff,
            )
        }
        0x6f => {
            let imm = jtype_imm(insn);
            let target = pc.wrapping_add(imm as u64);
            if rd == 0 {
                format!("j {target:#x}")
            } else {
                format!("jal {}, {target:#x}", reg(rd))
            }
        }
        0x67 => {
            let imm = sign_ext(insn >> 20, 12);
            if rd == 0 && imm == 0 {
                format!("jr {}", reg(rs1))
            } else if rd == 1 && imm == 0 {
                format!("jalr {}", reg(rs1))
            } else {
                format!("jalr {}, {imm}({})", reg(rd), reg(rs1))
            }
        }
        0x63 => disasm_branch(insn, pc, funct3, rs1, rs2),
        0x03 => disasm_load(insn, funct3, rd, rs1),
        0x23 => disasm_store(insn, funct3, rs1, rs2),
        0x13 => disasm_op_imm(insn, funct3, rd, rs1),
        0x33 => disasm_op(funct3, funct7, rd, rs1, rs2),
        0x1b => disasm_op_imm32(insn, funct3, rd, rs1),
        0x3b => disasm_op32(funct3, funct7, rd, rs1, rs2),
        0x2f => disasm_amo(insn, funct3, rd, rs1, rs2),
        0x73 => disasm_system(insn, rd, rs1, funct3),
        0x0f => {
            if funct3 == 0 {
                "fence".into()
            } else {
                "fence.i".into()
            }
        }
        _ => format!(".word {insn:#010x}"),
    }
}

// -- Immediate extraction --

fn jtype_imm(insn: u32) -> i64 {
    let b20 = (insn >> 31) & 1;
    let b10_1 = (insn >> 21) & 0x3ff;
    let b11 = (insn >> 20) & 1;
    let b19_12 = (insn >> 12) & 0xff;
    let raw = (b20 << 20) | (b19_12 << 12) | (b11 << 11) | (b10_1 << 1);
    sign_ext(raw, 21)
}

fn btype_imm(insn: u32) -> i64 {
    let b12 = (insn >> 31) & 1;
    let b10_5 = (insn >> 25) & 0x3f;
    let b4_1 = (insn >> 8) & 0xf;
    let b11 = (insn >> 7) & 1;
    let raw = (b12 << 12) | (b11 << 11) | (b10_5 << 5) | (b4_1 << 1);
    sign_ext(raw, 13)
}

fn stype_imm(insn: u32) -> i64 {
    let hi = (insn >> 25) & 0x7f;
    let lo = (insn >> 7) & 0x1f;
    sign_ext((hi << 5) | lo, 12)
}

fn itype_imm(insn: u32) -> i64 {
    sign_ext(insn >> 20, 12)
}

// -- Per-format disassembly --

fn disasm_branch(insn: u32, pc: u64, f3: u32, rs1: u32, rs2: u32) -> String {
    let imm = btype_imm(insn);
    let target = pc.wrapping_add(imm as u64);
    let op = match f3 {
        0 => "beq",
        1 => "bne",
        4 => "blt",
        5 => "bge",
        6 => "bltu",
        7 => "bgeu",
        _ => return format!(".word {insn:#010x}"),
    };
    // Pseudo-instructions
    if f3 == 0 && rs2 == 0 {
        format!("beqz {}, {target:#x}", reg(rs1))
    } else if f3 == 1 && rs2 == 0 {
        format!("bnez {}, {target:#x}", reg(rs1))
    } else {
        format!("{op} {}, {}, {target:#x}", reg(rs1), reg(rs2))
    }
}

fn disasm_load(insn: u32, f3: u32, rd: u32, rs1: u32) -> String {
    let imm = itype_imm(insn);
    let op = match f3 {
        0 => "lb",
        1 => "lh",
        2 => "lw",
        3 => "ld",
        4 => "lbu",
        5 => "lhu",
        6 => "lwu",
        _ => return format!(".word {insn:#010x}"),
    };
    format!("{op} {}, {imm}({})", reg(rd), reg(rs1))
}

fn disasm_store(insn: u32, f3: u32, rs1: u32, rs2: u32) -> String {
    let imm = stype_imm(insn);
    let op = match f3 {
        0 => "sb",
        1 => "sh",
        2 => "sw",
        3 => "sd",
        _ => return format!(".word {insn:#010x}"),
    };
    format!("{op} {}, {imm}({})", reg(rs2), reg(rs1))
}

fn disasm_op_imm(insn: u32, f3: u32, rd: u32, rs1: u32) -> String {
    let imm = itype_imm(insn);
    let shamt = (insn >> 20) & 0x3f;
    match f3 {
        0 if rs1 == 0 => format!("li {}, {imm}", reg(rd)),
        0 if imm == 0 => {
            format!("mv {}, {}", reg(rd), reg(rs1))
        }
        0 => format!("addi {}, {}, {imm}", reg(rd), reg(rs1)),
        1 => {
            format!("slli {}, {}, {shamt}", reg(rd), reg(rs1))
        }
        2 => {
            format!("slti {}, {}, {imm}", reg(rd), reg(rs1))
        }
        3 if imm == 1 => {
            format!("seqz {}, {}", reg(rd), reg(rs1))
        }
        3 => {
            format!("sltiu {}, {}, {imm}", reg(rd), reg(rs1))
        }
        4 if imm == -1 => {
            format!("not {}, {}", reg(rd), reg(rs1))
        }
        4 => {
            format!("xori {}, {}, {imm}", reg(rd), reg(rs1))
        }
        5 => {
            if insn >> 26 == 0 {
                format!("srli {}, {}, {shamt}", reg(rd), reg(rs1))
            } else {
                format!("srai {}, {}, {shamt}", reg(rd), reg(rs1))
            }
        }
        6 => {
            format!("ori {}, {}, {imm}", reg(rd), reg(rs1))
        }
        7 => {
            format!("andi {}, {}, {imm}", reg(rd), reg(rs1))
        }
        _ => unreachable!(),
    }
}

fn disasm_op(f3: u32, f7: u32, rd: u32, rs1: u32, rs2: u32) -> String {
    // M extension
    if f7 == 1 {
        let op = match f3 {
            0 => "mul",
            1 => "mulh",
            2 => "mulhsu",
            3 => "mulhu",
            4 => "div",
            5 => "divu",
            6 => "rem",
            7 => "remu",
            _ => unreachable!(),
        };
        return format!("{op} {}, {}, {}", reg(rd), reg(rs1), reg(rs2));
    }
    let op = match (f3, f7) {
        (0, 0) => "add",
        (0, 0x20) => "sub",
        (1, 0) => "sll",
        (2, 0) => "slt",
        (3, 0) => "sltu",
        (4, 0) => "xor",
        (5, 0) => "srl",
        (5, 0x20) => "sra",
        (6, 0) => "or",
        (7, 0) => "and",
        _ => {
            return format!("op f3={f3} f7={f7:#x}");
        }
    };
    // Pseudo: snez rd, rs2
    if f3 == 3 && rs1 == 0 {
        format!("snez {}, {}", reg(rd), reg(rs2))
    } else {
        format!("{op} {}, {}, {}", reg(rd), reg(rs1), reg(rs2))
    }
}

fn disasm_op_imm32(insn: u32, f3: u32, rd: u32, rs1: u32) -> String {
    let imm = itype_imm(insn);
    let shamt = (insn >> 20) & 0x1f;
    match f3 {
        0 if imm == 0 => {
            format!("sext.w {}, {}", reg(rd), reg(rs1))
        }
        0 => {
            format!("addiw {}, {}, {imm}", reg(rd), reg(rs1))
        }
        1 => {
            format!("slliw {}, {}, {shamt}", reg(rd), reg(rs1))
        }
        5 => {
            if insn >> 25 == 0 {
                format!("srliw {}, {}, {shamt}", reg(rd), reg(rs1))
            } else {
                format!("sraiw {}, {}, {shamt}", reg(rd), reg(rs1))
            }
        }
        _ => format!(".word {insn:#010x}"),
    }
}

fn disasm_op32(f3: u32, f7: u32, rd: u32, rs1: u32, rs2: u32) -> String {
    if f7 == 1 {
        let op = match f3 {
            0 => "mulw",
            4 => "divw",
            5 => "divuw",
            6 => "remw",
            7 => "remuw",
            _ => {
                return format!("op32 f3={f3} f7={f7:#x}");
            }
        };
        return format!("{op} {}, {}, {}", reg(rd), reg(rs1), reg(rs2));
    }
    let op = match (f3, f7) {
        (0, 0) => "addw",
        (0, 0x20) => "subw",
        (1, 0) => "sllw",
        (5, 0) => "srlw",
        (5, 0x20) => "sraw",
        _ => {
            return format!("op32 f3={f3} f7={f7:#x}");
        }
    };
    format!("{op} {}, {}, {}", reg(rd), reg(rs1), reg(rs2))
}

fn disasm_amo(insn: u32, f3: u32, rd: u32, rs1: u32, rs2: u32) -> String {
    let funct5 = insn >> 27;
    let aq = (insn >> 26) & 1;
    let rl = (insn >> 25) & 1;
    let suffix = match f3 {
        2 => ".w",
        3 => ".d",
        _ => return format!(".word {insn:#010x}"),
    };
    let aqrl = match (aq, rl) {
        (0, 0) => "",
        (1, 0) => ".aq",
        (0, 1) => ".rl",
        _ => ".aqrl",
    };
    match funct5 {
        0x02 => {
            format!("lr{suffix}{aqrl} {}, ({})", reg(rd), reg(rs1))
        }
        0x03 => {
            format!(
                "sc{suffix}{aqrl} {}, {}, ({})",
                reg(rd),
                reg(rs2),
                reg(rs1)
            )
        }
        _ => {
            let op = match funct5 {
                0x01 => "amoswap",
                0x00 => "amoadd",
                0x04 => "amoxor",
                0x0c => "amoand",
                0x08 => "amoor",
                0x10 => "amomin",
                0x14 => "amomax",
                0x18 => "amominu",
                0x1c => "amomaxu",
                _ => return format!(".word {insn:#010x}"),
            };
            format!(
                "{op}{suffix}{aqrl} {}, {}, ({})",
                reg(rd),
                reg(rs2),
                reg(rs1)
            )
        }
    }
}

fn disasm_system(insn: u32, rd: u32, rs1: u32, f3: u32) -> String {
    if f3 == 0 {
        return match insn {
            0x0000_0073 => "ecall".into(),
            0x0010_0073 => "ebreak".into(),
            _ => format!(".word {insn:#010x}"),
        };
    }
    let csr = insn >> 20;
    let op = match f3 {
        1 => "csrrw",
        2 => "csrrs",
        3 => "csrrc",
        5 => "csrrwi",
        6 => "csrrsi",
        7 => "csrrci",
        _ => return format!(".word {insn:#010x}"),
    };
    if f3 >= 5 {
        format!("{op} {}, {csr:#x}, {rs1}", reg(rd))
    } else {
        format!("{op} {}, {csr:#x}, {}", reg(rd), reg(rs1))
    }
}

// ================================================================
// 16-bit compressed instruction disassembly (C extension)
// ================================================================

fn disasm16(h: u32, pc: u64) -> String {
    let quadrant = h & 0x3;
    let funct3 = (h >> 13) & 0x7;

    match quadrant {
        0 => disasm_c_q0(h, funct3),
        1 => disasm_c_q1(h, funct3, pc),
        2 => disasm_c_q2(h, funct3),
        _ => format!(".half {h:#06x}"),
    }
}

fn disasm_c_q0(h: u32, f3: u32) -> String {
    let rd = creg((h >> 2) & 0x7);
    let rs1 = creg((h >> 7) & 0x7);
    match f3 {
        0 => {
            // C.ADDI4SPN
            let nzuimm = ((h >> 1) & 0x40)
                | ((h >> 7) & 0x30)
                | ((h >> 2) & 0x8)
                | ((h >> 4) & 0x4);
            if nzuimm == 0 {
                return format!(".half {h:#06x}");
            }
            format!("c.addi4spn {rd}, sp, {nzuimm}")
        }
        2 => {
            let off = c_lw_off(h);
            format!("c.lw {rd}, {off}({rs1})")
        }
        3 => {
            let off = c_ld_off(h);
            format!("c.ld {rd}, {off}({rs1})")
        }
        6 => {
            let off = c_lw_off(h);
            format!("c.sw {rd}, {off}({rs1})")
        }
        7 => {
            let off = c_ld_off(h);
            format!("c.sd {rd}, {off}({rs1})")
        }
        _ => format!(".half {h:#06x}"),
    }
}

fn disasm_c_q1(h: u32, f3: u32, pc: u64) -> String {
    match f3 {
        0 => {
            let rd = (h >> 7) & 0x1f;
            let imm = c_imm6(h);
            if rd == 0 {
                "c.nop".into()
            } else {
                format!("c.addi {}, {imm}", reg(rd))
            }
        }
        1 => {
            let rd = (h >> 7) & 0x1f;
            let imm = c_imm6(h);
            format!("c.addiw {}, {imm}", reg(rd))
        }
        2 => {
            let rd = (h >> 7) & 0x1f;
            let imm = c_imm6(h);
            format!("c.li {}, {imm}", reg(rd))
        }
        3 => {
            let rd = (h >> 7) & 0x1f;
            if rd == 2 {
                let imm = c_addi16sp_imm(h);
                format!("c.addi16sp sp, {imm}")
            } else {
                let imm = c_imm6(h);
                format!("c.lui {}, {imm:#x}", reg(rd))
            }
        }
        4 => disasm_c_alu(h),
        5 => {
            let off = c_j_off(h);
            let target = pc.wrapping_add(off as u64);
            format!("c.j {target:#x}")
        }
        6 => {
            let rs1 = creg((h >> 7) & 0x7);
            let off = c_b_off(h);
            let target = pc.wrapping_add(off as u64);
            format!("c.beqz {rs1}, {target:#x}")
        }
        7 => {
            let rs1 = creg((h >> 7) & 0x7);
            let off = c_b_off(h);
            let target = pc.wrapping_add(off as u64);
            format!("c.bnez {rs1}, {target:#x}")
        }
        _ => format!(".half {h:#06x}"),
    }
}

fn disasm_c_q2(h: u32, f3: u32) -> String {
    let rd = (h >> 7) & 0x1f;
    match f3 {
        0 => {
            let shamt = ((h >> 7) & 0x20) | ((h >> 2) & 0x1f);
            format!("c.slli {}, {shamt}", reg(rd))
        }
        2 => {
            let off = ((h >> 2) & 0x1c) | ((h << 4) & 0x20) | ((h >> 7) & 0x40);
            format!("c.lwsp {}, {off}(sp)", reg(rd))
        }
        3 => {
            let off = ((h >> 2) & 0x18) | ((h << 4) & 0x20) | ((h >> 7) & 0xc0);
            format!("c.ldsp {}, {off}(sp)", reg(rd))
        }
        4 => {
            let rs2 = (h >> 2) & 0x1f;
            let bit12 = (h >> 12) & 1;
            if bit12 == 0 {
                if rs2 == 0 {
                    format!("c.jr {}", reg(rd))
                } else {
                    format!("c.mv {}, {}", reg(rd), reg(rs2))
                }
            } else if rs2 == 0 {
                if rd == 0 {
                    "c.ebreak".into()
                } else {
                    format!("c.jalr {}", reg(rd))
                }
            } else {
                format!("c.add {}, {}", reg(rd), reg(rs2))
            }
        }
        6 => {
            let rs2 = (h >> 2) & 0x1f;
            let off = ((h >> 7) & 0x3c) | ((h >> 1) & 0x40);
            format!("c.swsp {}, {off}(sp)", reg(rs2))
        }
        7 => {
            let rs2 = (h >> 2) & 0x1f;
            let off = ((h >> 7) & 0x38) | ((h >> 1) & 0xc0);
            format!("c.sdsp {}, {off}(sp)", reg(rs2))
        }
        _ => format!(".half {h:#06x}"),
    }
}

fn disasm_c_alu(h: u32) -> String {
    let rd = creg((h >> 7) & 0x7);
    let f2 = (h >> 10) & 0x3;
    match f2 {
        0 => {
            let shamt = ((h >> 7) & 0x20) | ((h >> 2) & 0x1f);
            format!("c.srli {rd}, {shamt}")
        }
        1 => {
            let shamt = ((h >> 7) & 0x20) | ((h >> 2) & 0x1f);
            format!("c.srai {rd}, {shamt}")
        }
        2 => {
            let imm = c_imm6(h);
            format!("c.andi {rd}, {imm}")
        }
        3 => {
            let rs2 = creg((h >> 2) & 0x7);
            let bit12 = (h >> 12) & 1;
            let f2b = (h >> 5) & 0x3;
            let op = match (bit12, f2b) {
                (0, 0) => "c.sub",
                (0, 1) => "c.xor",
                (0, 2) => "c.or",
                (0, 3) => "c.and",
                (1, 0) => "c.subw",
                (1, 1) => "c.addw",
                _ => return format!(".half {h:#06x}"),
            };
            format!("{op} {rd}, {rs2}")
        }
        _ => unreachable!(),
    }
}

// -- Compressed immediate extraction helpers --

fn c_imm6(h: u32) -> i64 {
    let raw = ((h >> 7) & 0x20) | ((h >> 2) & 0x1f);
    sign_ext(raw, 6)
}

fn c_addi16sp_imm(h: u32) -> i64 {
    let raw = ((h >> 3) & 0x200)
        | ((h >> 2) & 0x10)
        | ((h << 1) & 0x40)
        | ((h << 4) & 0x180)
        | ((h << 3) & 0x20);
    sign_ext(raw, 10)
}

fn c_j_off(h: u32) -> i64 {
    let raw = ((h >> 1) & 0x800)
        | ((h >> 7) & 0x10)
        | ((h >> 1) & 0x300)
        | ((h << 2) & 0x400)
        | ((h >> 1) & 0x40)
        | ((h << 1) & 0x80)
        | ((h >> 2) & 0xe)
        | ((h << 3) & 0x20);
    sign_ext(raw, 12)
}

fn c_b_off(h: u32) -> i64 {
    let raw = ((h >> 4) & 0x100)
        | ((h >> 7) & 0x18)
        | ((h << 1) & 0xc0)
        | ((h >> 2) & 0x6)
        | ((h << 3) & 0x20);
    sign_ext(raw, 9)
}

fn c_lw_off(h: u32) -> u32 {
    ((h >> 7) & 0x38) | ((h >> 4) & 0x4) | ((h << 1) & 0x40)
}

fn c_ld_off(h: u32) -> u32 {
    ((h >> 7) & 0x38) | ((h << 1) & 0xc0)
}
