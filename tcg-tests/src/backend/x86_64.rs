use tcg_backend::code_buffer::CodeBuffer;
use tcg_backend::x86_64::emitter::*;
use tcg_backend::x86_64::regs::*;
use tcg_backend::x86_64::X86_64CodeGen;
use tcg_backend::HostCodeGen;
use tcg_core::{Context, Op, OpIdx, Opcode, Type};

// -- regs tests --

#[test]
fn reg_encoding() {
    assert_eq!(Reg::Rax.low3(), 0);
    assert_eq!(Reg::Rsp.low3(), 4);
    assert_eq!(Reg::R8.low3(), 0);
    assert_eq!(Reg::R15.low3(), 7);
}

#[test]
fn reg_needs_rex() {
    assert!(!Reg::Rax.needs_rex());
    assert!(!Reg::Rdi.needs_rex());
    assert!(Reg::R8.needs_rex());
    assert!(Reg::R15.needs_rex());
}

#[test]
fn areg0_is_rbp() {
    assert_eq!(TCG_AREG0, Reg::Rbp);
}

#[test]
fn reserved_regs_contains_rsp_rbp() {
    assert!(RESERVED_REGS.contains(Reg::Rsp as u8));
    assert!(RESERVED_REGS.contains(Reg::Rbp as u8));
    assert!(!RESERVED_REGS.contains(Reg::Rax as u8));
}

#[test]
fn frame_size_aligned() {
    assert_eq!(FRAME_SIZE % STACK_ALIGN, 0);
}

#[test]
fn stack_addend_positive() {
    assert!(STACK_ADDEND > 0);
    // After pushes + sub, total should be FRAME_SIZE
    assert_eq!(PUSH_SIZE + STACK_ADDEND, FRAME_SIZE);
}

#[test]
fn callee_saved_order() {
    // First should be RBP (env pointer)
    assert_eq!(CALLEE_SAVED[0], Reg::Rbp);
}

// -- emitter tests --

fn gen_prologue_epilogue() -> (CodeBuffer, X86_64CodeGen) {
    let mut buf = CodeBuffer::new(4096).unwrap();
    let mut gen = X86_64CodeGen::new();
    gen.emit_prologue(&mut buf);
    gen.emit_epilogue(&mut buf);
    (buf, gen)
}

#[test]
fn prologue_starts_with_push_rbp() {
    let (buf, _) = gen_prologue_epilogue();
    // push rbp = 0x55
    assert_eq!(
        buf.as_slice()[0],
        0x55,
        "prologue should start with push rbp"
    );
}

#[test]
fn epilogue_ends_with_ret() {
    let (buf, _) = gen_prologue_epilogue();
    let code = buf.as_slice();
    assert_eq!(code[code.len() - 1], 0xC3, "epilogue should end with ret");
}

#[test]
fn prologue_contains_jmp_rsi() {
    let (buf, gen) = gen_prologue_epilogue();
    let prologue = &buf.as_slice()[..gen.code_gen_start];
    // jmp *%rsi = FF E6
    let found = prologue.windows(2).any(|w| w[0] == 0xFF && w[1] == 0xE6);
    assert!(found, "prologue should contain jmp *%rsi");
}

#[test]
fn epilogue_contains_xor_eax() {
    let (buf, gen) = gen_prologue_epilogue();
    let zero_offset = gen.epilogue_return_zero_offset;
    // xor %eax, %eax = 31 C0
    assert_eq!(buf.as_slice()[zero_offset], 0x31);
    assert_eq!(buf.as_slice()[zero_offset + 1], 0xC0);
}

#[test]
fn tb_ret_after_zero_return() {
    let (_, gen) = gen_prologue_epilogue();
    // tb_ret should come right after the xor eax,eax (2 bytes)
    assert_eq!(gen.tb_ret_offset, gen.epilogue_return_zero_offset + 2);
}

#[test]
fn prologue_contains_sub_rsp() {
    let (buf, gen) = gen_prologue_epilogue();
    let prologue = &buf.as_slice()[..gen.code_gen_start];
    // Look for REX.W (0x48) followed by SUB opcode
    let has_sub = prologue
        .windows(2)
        .any(|w| w[0] == 0x48 && (w[1] == 0x81 || w[1] == 0x83));
    assert!(has_sub, "prologue should contain sub rsp, imm");
}

#[test]
fn epilogue_contains_add_rsp() {
    let (buf, gen) = gen_prologue_epilogue();
    let epilogue = &buf.as_slice()[gen.tb_ret_offset..];
    let has_add = epilogue.windows(3).any(|w| {
        w[0] == 0x48 && (w[1] == 0x81 || w[1] == 0x83) && w[2] == 0xC4
    });
    assert!(has_add, "epilogue should contain add rsp, imm");
}

#[test]
fn epilogue_pop_count_matches_push() {
    let (buf, gen) = gen_prologue_epilogue();
    let epilogue = &buf.as_slice()[gen.tb_ret_offset..];
    // Count pop instructions
    // 0x58-0x5F base, 0x41 0x58-0x5F extended
    let mut pop_count = 0;
    let mut i = 0;
    while i < epilogue.len() {
        if epilogue[i] == 0x41
            && i + 1 < epilogue.len()
            && (0x58..=0x5F).contains(&epilogue[i + 1])
        {
            pop_count += 1;
            i += 2;
        } else if (0x58..=0x5F).contains(&epilogue[i]) {
            pop_count += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    assert_eq!(
        pop_count,
        CALLEE_SAVED.len(),
        "pop count should match callee-saved count"
    );
}

#[test]
fn exit_tb_zero() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    let mut gen = X86_64CodeGen::new();
    gen.emit_prologue(&mut buf);
    gen.emit_epilogue(&mut buf);

    let exit_offset = buf.offset();
    gen.emit_exit_tb(&mut buf, 0);
    let code = &buf.as_slice()[exit_offset..];
    // Should be a jmp rel32 (E9 xx xx xx xx)
    assert_eq!(code[0], 0xE9, "exit_tb(0) should emit jmp rel32");
}

#[test]
fn exit_tb_nonzero() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    let mut gen = X86_64CodeGen::new();
    gen.emit_prologue(&mut buf);
    gen.emit_epilogue(&mut buf);

    let exit_offset = buf.offset();
    gen.emit_exit_tb(&mut buf, 0x1234);
    let code = &buf.as_slice()[exit_offset..];
    // Should start with mov eax, imm32 (B8 34 12 00 00) since val fits in u32
    assert_eq!(code[0], 0xB8, "exit_tb(nonzero) should emit mov eax, imm32");
}

#[test]
fn goto_tb_alignment() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    let gen = X86_64CodeGen::new();
    let (jmp_offset, reset_offset) = gen.emit_goto_tb(&mut buf);

    // The displacement field (at jmp_offset + 1) should be 4-byte aligned
    assert_eq!(
        (jmp_offset + 1) % 4,
        0,
        "goto_tb displacement should be 4-byte aligned"
    );
    // Reset offset should be 5 bytes after jmp_offset (E9 + 4 bytes)
    assert_eq!(reset_offset, jmp_offset + 5);
}

#[test]
fn goto_ptr_emits_jmp_reg() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    X86_64CodeGen::emit_goto_ptr(&mut buf, Reg::Rax);
    let code = buf.as_slice();
    // jmp *%rax = FF E0
    assert_eq!(code[0], 0xFF);
    assert_eq!(code[1], 0xE0);
}

#[test]
fn goto_ptr_extended_reg() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    X86_64CodeGen::emit_goto_ptr(&mut buf, Reg::R12);
    let code = buf.as_slice();
    // jmp *%r12 = 41 FF E4
    assert_eq!(code[0], 0x41); // REX.B
    assert_eq!(code[1], 0xFF);
    assert_eq!(code[2], 0xE4);
}

#[test]
fn patch_jump_forward() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    let mut gen = X86_64CodeGen::new();

    let jmp_offset = buf.offset();
    buf.emit_u8(0xE9);
    buf.emit_u32(0); // placeholder

    // Emit some padding
    for _ in 0..10 {
        buf.emit_u8(0x90);
    }
    let target = buf.offset();

    gen.patch_jump(&mut buf, jmp_offset, target);

    // Verify displacement: target - (jmp_offset + 5)
    let expected_disp = (target as i32) - (jmp_offset as i32 + 5);
    assert_eq!(buf.read_u32(jmp_offset + 1), expected_disp as u32);
}

#[test]
fn init_context_sets_reserved_regs() {
    let gen = X86_64CodeGen::new();
    let mut ctx = tcg_core::Context::new();
    gen.init_context(&mut ctx);

    assert!(ctx.reserved_regs.contains(Reg::Rsp as u8));
    assert!(ctx.reserved_regs.contains(Reg::Rbp as u8));
    assert!(ctx.frame_reg.is_some());
}

// ==========================================================
// Instruction emitter tests
// ==========================================================

fn emit_bytes(f: impl FnOnce(&mut CodeBuffer)) -> Vec<u8> {
    let mut buf = CodeBuffer::new(4096).unwrap();
    f(&mut buf);
    buf.as_slice().to_vec()
}

// -- Arithmetic tests --

#[test]
fn arith_add_rr_32() {
    // add eax, ecx => 03 C1
    let code = emit_bytes(|b| {
        emit_arith_rr(b, ArithOp::Add, false, Reg::Rax, Reg::Rcx)
    });
    assert_eq!(code, [0x03, 0xC1]);
}

#[test]
fn arith_add_rr_64() {
    // add rax, rcx => 48 03 C1
    let code = emit_bytes(|b| {
        emit_arith_rr(b, ArithOp::Add, true, Reg::Rax, Reg::Rcx)
    });
    assert_eq!(code, [0x48, 0x03, 0xC1]);
}

#[test]
fn arith_add_rr_extended() {
    // add r8, r9 => 4D 03 C1
    let code =
        emit_bytes(|b| emit_arith_rr(b, ArithOp::Add, true, Reg::R8, Reg::R9));
    assert_eq!(code, [0x4D, 0x03, 0xC1]);
}

#[test]
fn arith_sub_ri_imm8() {
    // sub eax, 0x10 => 83 E8 10
    let code =
        emit_bytes(|b| emit_arith_ri(b, ArithOp::Sub, false, Reg::Rax, 0x10));
    assert_eq!(code, [0x83, 0xE8, 0x10]);
}

#[test]
fn arith_sub_ri_imm32() {
    // sub rax, 0x1000 => 48 81 E8 00 10 00 00
    let code =
        emit_bytes(|b| emit_arith_ri(b, ArithOp::Sub, true, Reg::Rax, 0x1000));
    assert_eq!(code, [0x48, 0x81, 0xE8, 0x00, 0x10, 0x00, 0x00]);
}

#[test]
fn arith_xor_rr_32() {
    // xor eax, eax => 33 C0
    let code = emit_bytes(|b| {
        emit_arith_rr(b, ArithOp::Xor, false, Reg::Rax, Reg::Rax)
    });
    assert_eq!(code, [0x33, 0xC0]);
}

#[test]
fn arith_cmp_rr_64() {
    // cmp rdi, rsi => 48 3B FE
    let code = emit_bytes(|b| {
        emit_arith_rr(b, ArithOp::Cmp, true, Reg::Rdi, Reg::Rsi)
    });
    assert_eq!(code, [0x48, 0x3B, 0xFE]);
}

#[test]
fn arith_and_ri_imm8() {
    // and ecx, 0x7F => 83 E1 7F
    let code =
        emit_bytes(|b| emit_arith_ri(b, ArithOp::And, false, Reg::Rcx, 0x7F));
    assert_eq!(code, [0x83, 0xE1, 0x7F]);
}

#[test]
fn arith_or_rr() {
    // or edx, ebx => 0B D3
    let code = emit_bytes(|b| {
        emit_arith_rr(b, ArithOp::Or, false, Reg::Rdx, Reg::Rbx)
    });
    assert_eq!(code, [0x0B, 0xD3]);
}

#[test]
fn arith_adc_rr() {
    // adc rax, rdx => 48 13 C2
    let code = emit_bytes(|b| {
        emit_arith_rr(b, ArithOp::Adc, true, Reg::Rax, Reg::Rdx)
    });
    assert_eq!(code, [0x48, 0x13, 0xC2]);
}

#[test]
fn arith_sbb_rr() {
    // sbb rax, rdx => 48 1B C2
    let code = emit_bytes(|b| {
        emit_arith_rr(b, ArithOp::Sbb, true, Reg::Rax, Reg::Rdx)
    });
    assert_eq!(code, [0x48, 0x1B, 0xC2]);
}

#[test]
fn neg_32() {
    // neg eax => F7 D8
    let code = emit_bytes(|b| emit_neg(b, false, Reg::Rax));
    assert_eq!(code, [0xF7, 0xD8]);
}

#[test]
fn neg_64_extended() {
    // neg r8 => 49 F7 D8
    let code = emit_bytes(|b| emit_neg(b, true, Reg::R8));
    assert_eq!(code, [0x49, 0xF7, 0xD8]);
}

#[test]
fn not_32() {
    // not ecx => F7 D1
    let code = emit_bytes(|b| emit_not(b, false, Reg::Rcx));
    assert_eq!(code, [0xF7, 0xD1]);
}

// -- Shift tests --

#[test]
fn shift_shl_ri_1() {
    // shl eax, 1 => D1 E0
    let code =
        emit_bytes(|b| emit_shift_ri(b, ShiftOp::Shl, false, Reg::Rax, 1));
    assert_eq!(code, [0xD1, 0xE0]);
}

#[test]
fn shift_shl_ri_n() {
    // shl eax, 4 => C1 E0 04
    let code =
        emit_bytes(|b| emit_shift_ri(b, ShiftOp::Shl, false, Reg::Rax, 4));
    assert_eq!(code, [0xC1, 0xE0, 0x04]);
}

#[test]
fn shift_shr_ri_64() {
    // shr rax, 8 => 48 C1 E8 08
    let code =
        emit_bytes(|b| emit_shift_ri(b, ShiftOp::Shr, true, Reg::Rax, 8));
    assert_eq!(code, [0x48, 0xC1, 0xE8, 0x08]);
}

#[test]
fn shift_sar_cl() {
    // sar eax, cl => D3 F8
    let code = emit_bytes(|b| emit_shift_cl(b, ShiftOp::Sar, false, Reg::Rax));
    assert_eq!(code, [0xD3, 0xF8]);
}

#[test]
fn shift_rol_ri() {
    // rol ecx, 3 => C1 C1 03
    let code =
        emit_bytes(|b| emit_shift_ri(b, ShiftOp::Rol, false, Reg::Rcx, 3));
    assert_eq!(code, [0xC1, 0xC1, 0x03]);
}

#[test]
fn shift_ror_ri() {
    // ror edx, 5 => C1 CA 05
    let code =
        emit_bytes(|b| emit_shift_ri(b, ShiftOp::Ror, false, Reg::Rdx, 5));
    assert_eq!(code, [0xC1, 0xCA, 0x05]);
}

// -- Data movement tests --

#[test]
fn mov_rr_32() {
    // mov eax, ecx => 89 C8
    let code = emit_bytes(|b| emit_mov_rr(b, false, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x89, 0xC8]);
}

#[test]
fn mov_rr_64() {
    // mov rax, rcx => 48 89 C8
    let code = emit_bytes(|b| emit_mov_rr(b, true, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x48, 0x89, 0xC8]);
}

#[test]
fn mov_rr_extended() {
    // mov r8, r9 => 4D 89 C8
    let code = emit_bytes(|b| emit_mov_rr(b, true, Reg::R8, Reg::R9));
    assert_eq!(code, [0x4D, 0x89, 0xC8]);
}

#[test]
fn mov_ri_zero() {
    // xor eax, eax => 31 C0
    let code = emit_bytes(|b| emit_mov_ri(b, true, Reg::Rax, 0));
    assert_eq!(code, [0x31, 0xC0]);
}

#[test]
fn mov_ri_u32() {
    // mov eax, 0x1234 => B8 34 12 00 00
    let code = emit_bytes(|b| emit_mov_ri(b, true, Reg::Rax, 0x1234));
    assert_eq!(code, [0xB8, 0x34, 0x12, 0x00, 0x00]);
}

#[test]
fn mov_ri_imm64() {
    // movabs rax, 0x123456789ABCDEF0 => 48 B8 F0 DE BC 9A 78 56 34 12
    let code =
        emit_bytes(|b| emit_mov_ri(b, true, Reg::Rax, 0x123456789ABCDEF0));
    assert_eq!(code[0], 0x48); // REX.W
    assert_eq!(code[1], 0xB8); // MOV rax, imm64
    assert_eq!(code.len(), 10);
}

#[test]
fn mov_ri_sign_ext_imm32() {
    // mov rax, -1 (sign-extended) => 48 C7 C0 FF FF FF FF
    let code = emit_bytes(|b| emit_mov_ri(b, true, Reg::Rax, u64::MAX));
    // -1 as i64 fits in i32, so uses sign-extended imm32
    assert_eq!(code, [0x48, 0xC7, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn movzx_byte() {
    // movzbl eax, cl => 0F B6 C1
    let code = emit_bytes(|b| emit_movzx(b, OPC_MOVZBL, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x0F, 0xB6, 0xC1]);
}

#[test]
fn movzx_word() {
    // movzwl eax, cx => 0F B7 C1
    let code = emit_bytes(|b| emit_movzx(b, OPC_MOVZWL, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x0F, 0xB7, 0xC1]);
}

#[test]
fn movsx_byte() {
    // movsbl eax, cl => 0F BE C1
    let code = emit_bytes(|b| emit_movsx(b, OPC_MOVSBL, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x0F, 0xBE, 0xC1]);
}

#[test]
fn movsx_word() {
    // movswl eax, cx => 0F BF C1
    let code = emit_bytes(|b| emit_movsx(b, OPC_MOVSWL, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x0F, 0xBF, 0xC1]);
}

#[test]
fn movslq_test() {
    // movslq rax, ecx => 48 63 C1
    let code = emit_bytes(|b| emit_movsx(b, OPC_MOVSLQ, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x48, 0x63, 0xC1]);
}

#[test]
fn bswap_32() {
    // bswap eax => 0F C8
    let code = emit_bytes(|b| emit_bswap(b, false, Reg::Rax));
    assert_eq!(code, [0x0F, 0xC8]);
}

#[test]
fn bswap_64() {
    // bswap rax => 48 0F C8
    let code = emit_bytes(|b| emit_bswap(b, true, Reg::Rax));
    assert_eq!(code, [0x48, 0x0F, 0xC8]);
}

#[test]
fn bswap_extended() {
    // bswap r8d => 41 0F C8
    let code = emit_bytes(|b| emit_bswap(b, false, Reg::R8));
    assert_eq!(code, [0x41, 0x0F, 0xC8]);
}

// -- Memory operation tests --

#[test]
fn load_64_base_offset() {
    // mov rax, [rcx+0x10] => 48 8B 41 10
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::Rcx, 0x10));
    assert_eq!(code, [0x48, 0x8B, 0x41, 0x10]);
}

#[test]
fn load_64_base_zero() {
    // mov rax, [rcx] => 48 8B 01
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::Rcx, 0));
    assert_eq!(code, [0x48, 0x8B, 0x01]);
}

#[test]
fn load_64_rbp_zero() {
    // mov rax, [rbp+0] => 48 8B 45 00 (RBP needs explicit disp8)
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::Rbp, 0));
    assert_eq!(code, [0x48, 0x8B, 0x45, 0x00]);
}

#[test]
fn load_64_rsp_offset() {
    // mov rax, [rsp+0x10] => 48 8B 44 24 10 (RSP needs SIB)
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::Rsp, 0x10));
    assert_eq!(code, [0x48, 0x8B, 0x44, 0x24, 0x10]);
}

#[test]
fn store_64_base_offset() {
    // mov [rcx+0x10], rax => 48 89 41 10
    let code = emit_bytes(|b| emit_store(b, true, Reg::Rax, Reg::Rcx, 0x10));
    assert_eq!(code, [0x48, 0x89, 0x41, 0x10]);
}

#[test]
fn load_disp32() {
    // mov rax, [rcx+0x1000] => 48 8B 81 00 10 00 00
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::Rcx, 0x1000));
    assert_eq!(code, [0x48, 0x8B, 0x81, 0x00, 0x10, 0x00, 0x00]);
}

#[test]
fn load_disp8_negative_boundary() {
    // mov rax, [rcx-128] => 48 8B 41 80
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::Rcx, -128));
    assert_eq!(code, [0x48, 0x8B, 0x41, 0x80]);
}

#[test]
fn load_disp32_negative_boundary() {
    // mov rax, [rcx-129] => 48 8B 81 7F FF FF FF
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::Rcx, -129));
    assert_eq!(code, [0x48, 0x8B, 0x81, 0x7F, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn lea_base_offset() {
    // lea rax, [rcx+0x10] => 48 8D 41 10
    let code = emit_bytes(|b| emit_lea(b, true, Reg::Rax, Reg::Rcx, 0x10));
    assert_eq!(code, [0x48, 0x8D, 0x41, 0x10]);
}

#[test]
fn store_imm_test() {
    // mov dword [rcx+0x10], 0x42 => C7 41 10 42 00 00 00
    let code = emit_bytes(|b| emit_store_imm(b, false, Reg::Rcx, 0x10, 0x42));
    assert_eq!(code, [0xC7, 0x41, 0x10, 0x42, 0x00, 0x00, 0x00]);
}

// -- Multiply / Divide tests --

#[test]
fn mul_32() {
    // mul ecx => F7 E1
    let code = emit_bytes(|b| emit_mul(b, false, Reg::Rcx));
    assert_eq!(code, [0xF7, 0xE1]);
}

#[test]
fn imul1_64() {
    // imul rcx => 48 F7 E9
    let code = emit_bytes(|b| emit_imul1(b, true, Reg::Rcx));
    assert_eq!(code, [0x48, 0xF7, 0xE9]);
}

#[test]
fn imul_rr_32() {
    // imul eax, ecx => 0F AF C1
    let code = emit_bytes(|b| emit_imul_rr(b, false, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x0F, 0xAF, 0xC1]);
}

#[test]
fn imul_ri_imm8() {
    // imul eax, ecx, 10 => 6B C1 0A
    let code = emit_bytes(|b| emit_imul_ri(b, false, Reg::Rax, Reg::Rcx, 10));
    assert_eq!(code, [0x6B, 0xC1, 0x0A]);
}

#[test]
fn imul_ri_imm32() {
    // imul eax, ecx, 0x1000 => 69 C1 00 10 00 00
    let code =
        emit_bytes(|b| emit_imul_ri(b, false, Reg::Rax, Reg::Rcx, 0x1000));
    assert_eq!(code, [0x69, 0xC1, 0x00, 0x10, 0x00, 0x00]);
}

#[test]
fn div_32() {
    // div ecx => F7 F1
    let code = emit_bytes(|b| emit_div(b, false, Reg::Rcx));
    assert_eq!(code, [0xF7, 0xF1]);
}

#[test]
fn idiv_64() {
    // idiv rcx => 48 F7 F9
    let code = emit_bytes(|b| emit_idiv(b, true, Reg::Rcx));
    assert_eq!(code, [0x48, 0xF7, 0xF9]);
}

#[test]
fn cdq_test() {
    let code = emit_bytes(|b| emit_cdq(b));
    assert_eq!(code, [0x99]);
}

#[test]
fn cqo_test() {
    let code = emit_bytes(|b| emit_cqo(b));
    assert_eq!(code, [0x48, 0x99]);
}

// -- Bit operation tests --

#[test]
fn bsf_32() {
    // bsf eax, ecx => 0F BC C1
    let code = emit_bytes(|b| emit_bsf(b, false, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x0F, 0xBC, 0xC1]);
}

#[test]
fn bsr_64() {
    // bsr rax, rcx => 48 0F BD C1
    let code = emit_bytes(|b| emit_bsr(b, true, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x48, 0x0F, 0xBD, 0xC1]);
}

#[test]
fn lzcnt_32() {
    // lzcnt eax, ecx => F3 0F BD C1
    let code = emit_bytes(|b| emit_lzcnt(b, false, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0xF3, 0x0F, 0xBD, 0xC1]);
}

#[test]
fn tzcnt_32() {
    // tzcnt eax, ecx => F3 0F BC C1
    let code = emit_bytes(|b| emit_tzcnt(b, false, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0xF3, 0x0F, 0xBC, 0xC1]);
}

#[test]
fn popcnt_64() {
    // popcnt rax, rcx => F3 48 0F B8 C1
    let code = emit_bytes(|b| emit_popcnt(b, true, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0xF3, 0x48, 0x0F, 0xB8, 0xC1]);
}

#[test]
fn bt_ri_test() {
    // bt eax, 5 => 0F BA E0 05
    let code = emit_bytes(|b| emit_bt_ri(b, false, Reg::Rax, 5));
    assert_eq!(code, [0x0F, 0xBA, 0xE0, 0x05]);
}

#[test]
fn bts_ri_test() {
    // bts eax, 5 => 0F BA E8 05
    let code = emit_bytes(|b| emit_bts_ri(b, false, Reg::Rax, 5));
    assert_eq!(code, [0x0F, 0xBA, 0xE8, 0x05]);
}

#[test]
fn btr_ri_test() {
    // btr eax, 5 => 0F BA F0 05
    let code = emit_bytes(|b| emit_btr_ri(b, false, Reg::Rax, 5));
    assert_eq!(code, [0x0F, 0xBA, 0xF0, 0x05]);
}

#[test]
fn btc_ri_test() {
    // btc eax, 5 => 0F BA F8 05
    let code = emit_bytes(|b| emit_btc_ri(b, false, Reg::Rax, 5));
    assert_eq!(code, [0x0F, 0xBA, 0xF8, 0x05]);
}

// -- Branch and comparison tests --

#[test]
fn jcc_je() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    // Emit some padding, then a Jcc forward
    for _ in 0..10 {
        buf.emit_u8(0x90);
    }
    let target = 100;
    emit_jcc(&mut buf, X86Cond::Je, target);
    let code = buf.as_slice();
    // 0F 84 xx xx xx xx
    assert_eq!(code[10], 0x0F);
    assert_eq!(code[11], 0x84);
}

#[test]
fn jcc_backward_disp32() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    for _ in 0..10 {
        buf.emit_u8(0x90);
    }
    emit_jcc(&mut buf, X86Cond::Je, 0);
    let code = buf.as_slice();
    // after = 10 + 2 + 4 = 16, disp = -16 => F0 FF FF FF
    assert_eq!(&code[10..16], [0x0F, 0x84, 0xF0, 0xFF, 0xFF, 0xFF]);
}

#[test]
#[should_panic(expected = "jcc displacement out of i32 range")]
fn jcc_out_of_range_panics() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    emit_jcc(&mut buf, X86Cond::Je, usize::MAX);
}

#[test]
fn jmp_rel32() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    emit_jmp(&mut buf, 100);
    let code = buf.as_slice();
    assert_eq!(code[0], 0xE9);
    // disp = 100 - 5 = 95 = 0x5F
    assert_eq!(code[1], 0x5F);
}

#[test]
fn jmp_backward_disp32() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    for _ in 0..10 {
        buf.emit_u8(0x90);
    }
    emit_jmp(&mut buf, 0);
    let code = buf.as_slice();
    // after = 10 + 1 + 4 = 15, disp = -15 => F1 FF FF FF
    assert_eq!(&code[10..15], [0xE9, 0xF1, 0xFF, 0xFF, 0xFF]);
}

#[test]
#[should_panic(expected = "jmp displacement out of i32 range")]
fn jmp_out_of_range_panics() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    emit_jmp(&mut buf, usize::MAX);
}

#[test]
fn call_rel32() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    emit_call(&mut buf, 100);
    let code = buf.as_slice();
    assert_eq!(code[0], 0xE8);
    // disp = 100 - 5 = 95 = 0x5F
    assert_eq!(code[1], 0x5F);
}

#[test]
fn call_backward_disp32() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    for _ in 0..10 {
        buf.emit_u8(0x90);
    }
    emit_call(&mut buf, 0);
    let code = buf.as_slice();
    // after = 10 + 1 + 4 = 15, disp = -15 => F1 FF FF FF
    assert_eq!(&code[10..15], [0xE8, 0xF1, 0xFF, 0xFF, 0xFF]);
}

#[test]
#[should_panic(expected = "call displacement out of i32 range")]
fn call_out_of_range_panics() {
    let mut buf = CodeBuffer::new(4096).unwrap();
    emit_call(&mut buf, usize::MAX);
}

#[test]
fn jmp_reg_test() {
    // jmp *rax => FF E0
    let code = emit_bytes(|b| emit_jmp_reg(b, Reg::Rax));
    assert_eq!(code, [0xFF, 0xE0]);
}

#[test]
fn jmp_reg_extended() {
    // jmp *r12 => 41 FF E4
    let code = emit_bytes(|b| emit_jmp_reg(b, Reg::R12));
    assert_eq!(code, [0x41, 0xFF, 0xE4]);
}

#[test]
fn call_reg_test() {
    // call *rax => FF D0
    let code = emit_bytes(|b| emit_call_reg(b, Reg::Rax));
    assert_eq!(code, [0xFF, 0xD0]);
}

#[test]
fn call_reg_extended() {
    // call *r12 => 41 FF D4
    let code = emit_bytes(|b| emit_call_reg(b, Reg::R12));
    assert_eq!(code, [0x41, 0xFF, 0xD4]);
}

#[test]
fn setcc_test() {
    // sete al => 0F 94 C0
    let code = emit_bytes(|b| emit_setcc(b, X86Cond::Je, Reg::Rax));
    assert_eq!(code, [0x0F, 0x94, 0xC0]);
}

#[test]
fn setcc_extended() {
    // sete r8b => 41 0F 94 C0
    let code = emit_bytes(|b| emit_setcc(b, X86Cond::Je, Reg::R8));
    assert_eq!(code, [0x41, 0x0F, 0x94, 0xC0]);
}

#[test]
fn cmovcc_test() {
    // cmove eax, ecx => 0F 44 C1
    let code =
        emit_bytes(|b| emit_cmovcc(b, X86Cond::Je, false, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x0F, 0x44, 0xC1]);
}

#[test]
fn cmovcc_64() {
    // cmovne rax, rcx => 48 0F 45 C1
    let code =
        emit_bytes(|b| emit_cmovcc(b, X86Cond::Jne, true, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x48, 0x0F, 0x45, 0xC1]);
}

#[test]
fn test_rr_32() {
    // test eax, ecx => 85 C1
    let code = emit_bytes(|b| emit_test_rr(b, false, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x85, 0xC1]);
}

#[test]
fn test_rr_64() {
    // test rax, rcx => 48 85 C1
    let code = emit_bytes(|b| emit_test_rr(b, true, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x48, 0x85, 0xC1]);
}

// -- Miscellaneous tests --

#[test]
fn xchg_test() {
    // xchg eax, ecx => 87 C1
    let code = emit_bytes(|b| emit_xchg(b, false, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x87, 0xC1]);
}

#[test]
fn push_reg_test() {
    // push rax => 50
    let code = emit_bytes(|b| emit_push(b, Reg::Rax));
    assert_eq!(code, [0x50]);
}

#[test]
fn push_extended_reg() {
    // push r8 => 41 50
    let code = emit_bytes(|b| emit_push(b, Reg::R8));
    assert_eq!(code, [0x41, 0x50]);
}

#[test]
fn pop_reg_test() {
    // pop rax => 58
    let code = emit_bytes(|b| emit_pop(b, Reg::Rax));
    assert_eq!(code, [0x58]);
}

#[test]
fn pop_extended_reg() {
    // pop r8 => 41 58
    let code = emit_bytes(|b| emit_pop(b, Reg::R8));
    assert_eq!(code, [0x41, 0x58]);
}

#[test]
fn push_imm8() {
    // push 0x42 => 6A 42
    let code = emit_bytes(|b| emit_push_imm(b, 0x42));
    assert_eq!(code, [0x6A, 0x42]);
}

#[test]
fn push_imm8_upper_boundary() {
    // push 127 => 6A 7F
    let code = emit_bytes(|b| emit_push_imm(b, 127));
    assert_eq!(code, [0x6A, 0x7F]);
}

#[test]
fn push_imm8_lower_boundary() {
    // push -128 => 6A 80
    let code = emit_bytes(|b| emit_push_imm(b, -128));
    assert_eq!(code, [0x6A, 0x80]);
}

#[test]
fn push_imm32_boundary() {
    // push 128 => 68 80 00 00 00
    let code = emit_bytes(|b| emit_push_imm(b, 128));
    assert_eq!(code, [0x68, 0x80, 0x00, 0x00, 0x00]);
}

#[test]
fn push_imm32() {
    // push 0x1000 => 68 00 10 00 00
    let code = emit_bytes(|b| emit_push_imm(b, 0x1000));
    assert_eq!(code, [0x68, 0x00, 0x10, 0x00, 0x00]);
}

#[test]
fn ret_test() {
    let code = emit_bytes(|b| emit_ret(b));
    assert_eq!(code, [0xC3]);
}

#[test]
fn mfence_test() {
    let code = emit_bytes(|b| emit_mfence(b));
    assert_eq!(code, [0x0F, 0xAE, 0xF0]);
}

#[test]
fn ud2_test() {
    let code = emit_bytes(|b| emit_ud2(b));
    assert_eq!(code, [0x0F, 0x0B]);
}

#[test]
fn nop_1() {
    let code = emit_bytes(|b| emit_nops(b, 1));
    assert_eq!(code, [0x90]);
}

#[test]
fn nop_0() {
    let code = emit_bytes(|b| emit_nops(b, 0));
    assert!(code.is_empty());
}

#[test]
fn nop_2() {
    let code = emit_bytes(|b| emit_nops(b, 2));
    assert_eq!(code, [0x66, 0x90]);
}

#[test]
fn nop_8() {
    let code = emit_bytes(|b| emit_nops(b, 8));
    assert_eq!(code.len(), 8);
    assert_eq!(code[0], 0x0F);
    assert_eq!(code[1], 0x1F);
}

#[test]
fn inc_32() {
    // inc eax => FF C0
    let code = emit_bytes(|b| emit_inc(b, false, Reg::Rax));
    assert_eq!(code, [0xFF, 0xC0]);
}

#[test]
fn dec_64() {
    // dec rax => 48 FF C8
    let code = emit_bytes(|b| emit_dec(b, true, Reg::Rax));
    assert_eq!(code, [0x48, 0xFF, 0xC8]);
}

#[test]
fn shld_ri_test() {
    // shld eax, ecx, 4 => 0F A4 C8 04
    let code = emit_bytes(|b| emit_shld_ri(b, false, Reg::Rax, Reg::Rcx, 4));
    assert_eq!(code, [0x0F, 0xA4, 0xC8, 0x04]);
}

#[test]
fn shrd_ri_test() {
    // shrd eax, ecx, 4 => 0F AC C8 04
    let code = emit_bytes(|b| emit_shrd_ri(b, false, Reg::Rax, Reg::Rcx, 4));
    assert_eq!(code, [0x0F, 0xAC, 0xC8, 0x04]);
}

// -- X86Cond tests --

#[test]
fn x86cond_from_tcg() {
    assert_eq!(X86Cond::from_tcg(tcg_core::Cond::Eq), X86Cond::Je);
    assert_eq!(X86Cond::from_tcg(tcg_core::Cond::Ne), X86Cond::Jne);
    assert_eq!(X86Cond::from_tcg(tcg_core::Cond::Lt), X86Cond::Jl);
    assert_eq!(X86Cond::from_tcg(tcg_core::Cond::Ge), X86Cond::Jge);
    assert_eq!(X86Cond::from_tcg(tcg_core::Cond::Ltu), X86Cond::Jb);
    assert_eq!(X86Cond::from_tcg(tcg_core::Cond::Geu), X86Cond::Jae);
}

#[test]
fn x86cond_from_tcg_always_never_fallback() {
    assert_eq!(X86Cond::from_tcg(tcg_core::Cond::Always), X86Cond::Je);
    assert_eq!(X86Cond::from_tcg(tcg_core::Cond::Never), X86Cond::Jne);
}

#[test]
fn x86cond_invert() {
    assert_eq!(X86Cond::Je.invert(), X86Cond::Jne);
    assert_eq!(X86Cond::Jne.invert(), X86Cond::Je);
    assert_eq!(X86Cond::Jb.invert(), X86Cond::Jae);
    assert_eq!(X86Cond::Jl.invert(), X86Cond::Jge);
}

// -- Core encoding tests --

#[test]
fn modrm_offset_rsp_sib() {
    // Verify RSP base always gets SIB byte
    let code = emit_bytes(|b| {
        emit_modrm_offset(b, OPC_MOVL_GvEv, Reg::Rax, Reg::Rsp, 0)
    });
    // Should have SIB byte 0x24
    assert!(
        code.iter().any(|&x| x == 0x24),
        "RSP base should have SIB byte"
    );
}

#[test]
fn modrm_offset_rbp_disp8() {
    // Verify RBP base with offset=0 gets disp8=0
    let code = emit_bytes(|b| {
        emit_modrm_offset(b, OPC_MOVL_GvEv, Reg::Rax, Reg::Rbp, 0)
    });
    // mod=01 with disp8=0
    let last = code[code.len() - 1];
    assert_eq!(last, 0x00, "RBP base with offset=0 should have disp8=0");
}

// ==========================================================
// Extended register addressing tests (R13/R12 special cases)
// ==========================================================

#[test]
fn load_r13_base_zero() {
    // mov rax, [r13] => 49 8B 45 00 (R13 needs disp8=0 like RBP)
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::R13, 0));
    assert_eq!(code, [0x49, 0x8B, 0x45, 0x00]);
}

#[test]
fn load_r13_base_disp8() {
    // mov rax, [r13+0x10] => 49 8B 45 10
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::R13, 0x10));
    assert_eq!(code, [0x49, 0x8B, 0x45, 0x10]);
}

#[test]
fn load_r12_base_zero() {
    // mov rax, [r12] => 49 8B 04 24 (R12 needs SIB like RSP)
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::R12, 0));
    assert_eq!(code, [0x49, 0x8B, 0x04, 0x24]);
}

#[test]
fn load_r12_base_disp8() {
    // mov rax, [r12+0x10] => 49 8B 44 24 10
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::R12, 0x10));
    assert_eq!(code, [0x49, 0x8B, 0x44, 0x24, 0x10]);
}

// ==========================================================
// SIB addressing mode tests
// ==========================================================

#[test]
fn lea_sib_base_index_scale_disp() {
    // lea rax, [rcx+rdx*4+0x10] => 48 8D 44 91 10
    let code = emit_bytes(|b| {
        emit_lea_sib(b, true, Reg::Rax, Reg::Rcx, Reg::Rdx, 2, 0x10)
    });
    assert_eq!(code, [0x48, 0x8D, 0x44, 0x91, 0x10]);
}

#[test]
fn load_sib_base_index_scale_disp() {
    // mov rax, [rcx+rdx*4+0x10] => 48 8B 44 91 10
    let code = emit_bytes(|b| {
        emit_load_sib(b, true, Reg::Rax, Reg::Rcx, Reg::Rdx, 2, 0x10)
    });
    assert_eq!(code, [0x48, 0x8B, 0x44, 0x91, 0x10]);
}

#[test]
fn store_sib_base_index_scale_disp() {
    // mov [rcx+rdx*4+0x10], rax => 48 89 44 91 10
    let code = emit_bytes(|b| {
        emit_store_sib(b, true, Reg::Rax, Reg::Rcx, Reg::Rdx, 2, 0x10)
    });
    assert_eq!(code, [0x48, 0x89, 0x44, 0x91, 0x10]);
}

#[test]
fn load_sib_no_disp() {
    // mov rax, [rcx+rdx*2] => 48 8B 04 51
    let code = emit_bytes(|b| {
        emit_load_sib(b, true, Reg::Rax, Reg::Rcx, Reg::Rdx, 1, 0)
    });
    assert_eq!(code, [0x48, 0x8B, 0x04, 0x51]);
}

#[test]
fn lea_sib_r12_index() {
    // lea rax, [rcx+r12*4+0x10] => 4A 8D 44 A1 10
    let code = emit_bytes(|b| {
        emit_lea_sib(b, true, Reg::Rax, Reg::Rcx, Reg::R12, 2, 0x10)
    });
    assert_eq!(code, [0x4A, 0x8D, 0x44, 0xA1, 0x10]);
}

#[test]
fn lea_sib_r13_base_r12_index_no_disp() {
    // lea rax, [r13+r12*2] => 4B 8D 44 65 00
    let code = emit_bytes(|b| {
        emit_lea_sib(b, true, Reg::Rax, Reg::R13, Reg::R12, 1, 0)
    });
    assert_eq!(code, [0x4B, 0x8D, 0x44, 0x65, 0x00]);
}

#[test]
fn load_sib_r12_index() {
    // mov rax, [rcx+r12*4+0x10] => 4A 8B 44 A1 10
    let code = emit_bytes(|b| {
        emit_load_sib(b, true, Reg::Rax, Reg::Rcx, Reg::R12, 2, 0x10)
    });
    assert_eq!(code, [0x4A, 0x8B, 0x44, 0xA1, 0x10]);
}

#[test]
fn store_sib_r12_index() {
    // mov [rcx+r12*4+0x10], rax => 4A 89 44 A1 10
    let code = emit_bytes(|b| {
        emit_store_sib(b, true, Reg::Rax, Reg::Rcx, Reg::R12, 2, 0x10)
    });
    assert_eq!(code, [0x4A, 0x89, 0x44, 0xA1, 0x10]);
}

#[test]
#[should_panic(expected = "RSP cannot be encoded as a SIB index register")]
fn lea_sib_rsp_index_panics() {
    let _ = emit_bytes(|b| {
        emit_lea_sib(b, true, Reg::Rax, Reg::Rcx, Reg::Rsp, 1, 0x10)
    });
}

#[test]
#[should_panic(expected = "SIB scale shift must be in 0..=3")]
fn lea_sib_shift_out_of_range_panics() {
    let _ = emit_bytes(|b| {
        emit_lea_sib(b, true, Reg::Rax, Reg::Rcx, Reg::Rdx, 4, 0x10)
    });
}

// ==========================================================
// Memory arithmetic tests
// ==========================================================

#[test]
fn arith_add_mr() {
    // add [rcx+0x10], rax => 48 01 41 10
    let code = emit_bytes(|b| {
        emit_arith_mr(b, ArithOp::Add, true, Reg::Rcx, 0x10, Reg::Rax)
    });
    assert_eq!(code, [0x48, 0x01, 0x41, 0x10]);
}

#[test]
fn arith_add_rm() {
    // add rax, [rcx+0x10] => 48 03 41 10
    let code = emit_bytes(|b| {
        emit_arith_rm(b, ArithOp::Add, true, Reg::Rax, Reg::Rcx, 0x10)
    });
    assert_eq!(code, [0x48, 0x03, 0x41, 0x10]);
}

#[test]
fn arith_sub_mr_32() {
    // sub [rdx+0x20], ecx => 29 4A 20
    let code = emit_bytes(|b| {
        emit_arith_mr(b, ArithOp::Sub, false, Reg::Rdx, 0x20, Reg::Rcx)
    });
    assert_eq!(code, [0x29, 0x4A, 0x20]);
}

// ==========================================================
// Byte operation tests
// ==========================================================

#[test]
fn store_byte_base_disp() {
    // mov [rcx+0x10], al => 88 41 10
    let code = emit_bytes(|b| emit_store_byte(b, Reg::Rax, Reg::Rcx, 0x10));
    assert_eq!(code, [0x88, 0x41, 0x10]);
}

#[test]
fn store_byte_extended_base() {
    // mov [r8+0x10], cl => 41 88 48 10
    let code = emit_bytes(|b| emit_store_byte(b, Reg::Rcx, Reg::R8, 0x10));
    assert_eq!(code, [0x41, 0x88, 0x48, 0x10]);
}

#[test]
fn test_bi_sil() {
    // test sil, 0x42 => 40 F6 C6 42
    let code = emit_bytes(|b| emit_test_bi(b, Reg::Rsi, 0x42));
    assert_eq!(code, [0x40, 0xF6, 0xC6, 0x42]);
}

#[test]
fn test_bi_al() {
    // test al, 0xFF => F6 C0 FF (no REX needed)
    let code = emit_bytes(|b| emit_test_bi(b, Reg::Rax, 0xFF));
    assert_eq!(code, [0xF6, 0xC0, 0xFF]);
}

#[test]
fn test_bi_r12b() {
    // test r12b, 0x42 => 41 F6 C4 42
    let code = emit_bytes(|b| emit_test_bi(b, Reg::R12, 0x42));
    assert_eq!(code, [0x41, 0xF6, 0xC4, 0x42]);
}

// ==========================================================
// Memory zero/sign-extend load tests
// ==========================================================

#[test]
fn load_zx_byte() {
    // movzbl eax, [rcx+0x10] => 0F B6 41 10
    let code =
        emit_bytes(|b| emit_load_zx(b, OPC_MOVZBL, Reg::Rax, Reg::Rcx, 0x10));
    assert_eq!(code, [0x0F, 0xB6, 0x41, 0x10]);
}

#[test]
fn load_sx_byte_64() {
    // movsbq rax, [rcx+0x10] => 48 0F BE 41 10
    let code = emit_bytes(|b| {
        emit_load_sx(b, OPC_MOVSBL | P_REXW, Reg::Rax, Reg::Rcx, 0x10)
    });
    assert_eq!(code, [0x48, 0x0F, 0xBE, 0x41, 0x10]);
}

#[test]
fn load_sx_dword_64() {
    // movslq rax, [rcx+0x10] => 48 63 41 10
    let code =
        emit_bytes(|b| emit_load_sx(b, OPC_MOVSLQ, Reg::Rax, Reg::Rcx, 0x10));
    assert_eq!(code, [0x48, 0x63, 0x41, 0x10]);
}

// ==========================================================
// VEX encoding tests (ANDN)
// ==========================================================

#[test]
fn andn_32() {
    // andn eax, ecx, edx => C4 E2 70 F2 C2
    let code =
        emit_bytes(|b| emit_andn(b, false, Reg::Rax, Reg::Rcx, Reg::Rdx));
    assert_eq!(code, [0xC4, 0xE2, 0x70, 0xF2, 0xC2]);
}

#[test]
fn andn_64() {
    // andn rax, rcx, rdx => C4 E2 F0 F2 C2
    let code = emit_bytes(|b| emit_andn(b, true, Reg::Rax, Reg::Rcx, Reg::Rdx));
    assert_eq!(code, [0xC4, 0xE2, 0xF0, 0xF2, 0xC2]);
}

#[test]
fn andn_64_extended_regs() {
    // andn r8, r12, r13 => C4 42 98 F2 C5
    let code = emit_bytes(|b| emit_andn(b, true, Reg::R8, Reg::R12, Reg::R13));
    assert_eq!(code, [0xC4, 0x42, 0x98, 0xF2, 0xC5]);
}

// ==========================================================
// Extended register instruction tests
// ==========================================================

#[test]
fn arith_sub_ri_extended_64() {
    // sub r8, 0x10 => 49 83 E8 10
    let code =
        emit_bytes(|b| emit_arith_ri(b, ArithOp::Sub, true, Reg::R8, 0x10));
    assert_eq!(code, [0x49, 0x83, 0xE8, 0x10]);
}

#[test]
fn arith_add_ri_extended_32() {
    // add r12d, 0x7F => 41 83 C4 7F
    let code =
        emit_bytes(|b| emit_arith_ri(b, ArithOp::Add, false, Reg::R12, 0x7F));
    assert_eq!(code, [0x41, 0x83, 0xC4, 0x7F]);
}

#[test]
fn mov_ri_zero_extended() {
    // xor r8d, r8d => 45 31 C0
    let code = emit_bytes(|b| emit_mov_ri(b, true, Reg::R8, 0));
    assert_eq!(code, [0x45, 0x31, 0xC0]);
}

#[test]
fn shift_shl_extended_64() {
    // shl r9, 4 => 49 C1 E1 04
    let code = emit_bytes(|b| emit_shift_ri(b, ShiftOp::Shl, true, Reg::R9, 4));
    assert_eq!(code, [0x49, 0xC1, 0xE1, 0x04]);
}

#[test]
fn shift_sar_cl_extended_64() {
    // sar r10, cl => 49 D3 FA
    let code = emit_bytes(|b| emit_shift_cl(b, ShiftOp::Sar, true, Reg::R10));
    assert_eq!(code, [0x49, 0xD3, 0xFA]);
}

#[test]
fn imul_rr_extended_64() {
    // imul r8, r9 => 4D 0F AF C1
    let code = emit_bytes(|b| emit_imul_rr(b, true, Reg::R8, Reg::R9));
    assert_eq!(code, [0x4D, 0x0F, 0xAF, 0xC1]);
}

#[test]
fn bsf_extended_64() {
    // bsf r8, r9 => 4D 0F BC C1
    let code = emit_bytes(|b| emit_bsf(b, true, Reg::R8, Reg::R9));
    assert_eq!(code, [0x4D, 0x0F, 0xBC, 0xC1]);
}

#[test]
fn lzcnt_extended_64() {
    // lzcnt r8, r9 => F3 4D 0F BD C1
    let code = emit_bytes(|b| emit_lzcnt(b, true, Reg::R8, Reg::R9));
    assert_eq!(code, [0xF3, 0x4D, 0x0F, 0xBD, 0xC1]);
}

#[test]
fn tzcnt_extended_64() {
    // tzcnt r8, r9 => F3 4D 0F BC C1
    let code = emit_bytes(|b| emit_tzcnt(b, true, Reg::R8, Reg::R9));
    assert_eq!(code, [0xF3, 0x4D, 0x0F, 0xBC, 0xC1]);
}

#[test]
fn popcnt_extended_64() {
    // popcnt r8, r9 => F3 4D 0F B8 C1
    let code = emit_bytes(|b| emit_popcnt(b, true, Reg::R8, Reg::R9));
    assert_eq!(code, [0xF3, 0x4D, 0x0F, 0xB8, 0xC1]);
}

#[test]
fn bt_ri_extended_64() {
    // bt r8, 5 => 49 0F BA E0 05
    let code = emit_bytes(|b| emit_bt_ri(b, true, Reg::R8, 5));
    assert_eq!(code, [0x49, 0x0F, 0xBA, 0xE0, 0x05]);
}

#[test]
fn setcc_sil() {
    // sete sil => 40 0F 94 C6 (needs REX for SIL)
    let code = emit_bytes(|b| emit_setcc(b, X86Cond::Je, Reg::Rsi));
    assert_eq!(code, [0x40, 0x0F, 0x94, 0xC6]);
}

#[test]
fn cmovcc_extended_64() {
    // cmove r8, r9 => 4D 0F 44 C1
    let code =
        emit_bytes(|b| emit_cmovcc(b, X86Cond::Je, true, Reg::R8, Reg::R9));
    assert_eq!(code, [0x4D, 0x0F, 0x44, 0xC1]);
}

#[test]
fn inc_extended_64() {
    // inc r8 => 49 FF C0
    let code = emit_bytes(|b| emit_inc(b, true, Reg::R8));
    assert_eq!(code, [0x49, 0xFF, 0xC0]);
}

#[test]
fn dec_extended_32() {
    // dec r12d => 41 FF CC
    let code = emit_bytes(|b| emit_dec(b, false, Reg::R12));
    assert_eq!(code, [0x41, 0xFF, 0xCC]);
}

#[test]
fn not_extended_64() {
    // not r8 => 49 F7 D0
    let code = emit_bytes(|b| emit_not(b, true, Reg::R8));
    assert_eq!(code, [0x49, 0xF7, 0xD0]);
}

#[test]
fn mul_64() {
    // mul rcx => 48 F7 E1
    let code = emit_bytes(|b| emit_mul(b, true, Reg::Rcx));
    assert_eq!(code, [0x48, 0xF7, 0xE1]);
}

#[test]
fn div_extended_64() {
    // div r8 => 49 F7 F0
    let code = emit_bytes(|b| emit_div(b, true, Reg::R8));
    assert_eq!(code, [0x49, 0xF7, 0xF0]);
}

#[test]
fn push_r15() {
    // push r15 => 41 57
    let code = emit_bytes(|b| emit_push(b, Reg::R15));
    assert_eq!(code, [0x41, 0x57]);
}

#[test]
fn pop_r15() {
    // pop r15 => 41 5F
    let code = emit_bytes(|b| emit_pop(b, Reg::R15));
    assert_eq!(code, [0x41, 0x5F]);
}

#[test]
fn bswap_r9_64() {
    // bswap r9 => 49 0F C9
    let code = emit_bytes(|b| emit_bswap(b, true, Reg::R9));
    assert_eq!(code, [0x49, 0x0F, 0xC9]);
}

// ==========================================================
// 64-bit variants of 32-bit-only tests
// ==========================================================

#[test]
fn shld_ri_64() {
    // shld rax, rcx, 4 => 48 0F A4 C8 04
    let code = emit_bytes(|b| emit_shld_ri(b, true, Reg::Rax, Reg::Rcx, 4));
    assert_eq!(code, [0x48, 0x0F, 0xA4, 0xC8, 0x04]);
}

#[test]
fn shrd_ri_64() {
    // shrd rax, rcx, 4 => 48 0F AC C8 04
    let code = emit_bytes(|b| emit_shrd_ri(b, true, Reg::Rax, Reg::Rcx, 4));
    assert_eq!(code, [0x48, 0x0F, 0xAC, 0xC8, 0x04]);
}

#[test]
fn xchg_64() {
    // xchg rax, rcx => 48 87 C1
    let code = emit_bytes(|b| emit_xchg(b, true, Reg::Rax, Reg::Rcx));
    assert_eq!(code, [0x48, 0x87, 0xC1]);
}

#[test]
fn shift_shl_cl_64() {
    // shl rax, cl => 48 D3 E0
    let code = emit_bytes(|b| emit_shift_cl(b, ShiftOp::Shl, true, Reg::Rax));
    assert_eq!(code, [0x48, 0xD3, 0xE0]);
}

// ==========================================================
// Negative immediate and edge case tests
// ==========================================================

#[test]
fn arith_add_ri_neg1() {
    // add rax, -1 => 48 83 C0 FF
    let code =
        emit_bytes(|b| emit_arith_ri(b, ArithOp::Add, true, Reg::Rax, -1));
    assert_eq!(code, [0x48, 0x83, 0xC0, 0xFF]);
}

#[test]
fn arith_sub_ri_neg128() {
    // sub rcx, -128 => 48 83 E9 80
    let code =
        emit_bytes(|b| emit_arith_ri(b, ArithOp::Sub, true, Reg::Rcx, -128));
    assert_eq!(code, [0x48, 0x83, 0xE9, 0x80]);
}

#[test]
fn arith_add_ri_imm32_boundary() {
    // add rax, 128 => 48 81 C0 80 00 00 00 (128 doesn't fit imm8)
    let code =
        emit_bytes(|b| emit_arith_ri(b, ArithOp::Add, true, Reg::Rax, 128));
    assert_eq!(code, [0x48, 0x81, 0xC0, 0x80, 0x00, 0x00, 0x00]);
}

#[test]
fn mov_ri_sign_ext_extended() {
    // mov r8, -1 => 49 C7 C0 FF FF FF FF
    let code = emit_bytes(|b| emit_mov_ri(b, true, Reg::R8, u64::MAX));
    assert_eq!(code, [0x49, 0xC7, 0xC0, 0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn mov_ri_u32_extended() {
    // mov r8d, 0x1234 => 41 B8 34 12 00 00
    let code = emit_bytes(|b| emit_mov_ri(b, true, Reg::R8, 0x1234));
    assert_eq!(code, [0x41, 0xB8, 0x34, 0x12, 0x00, 0x00]);
}

#[test]
fn mov_ri_imm64_extended() {
    // movabs r8, 0x123456789ABCDEF0
    let code =
        emit_bytes(|b| emit_mov_ri(b, true, Reg::R8, 0x123456789ABCDEF0));
    assert_eq!(code[0], 0x49); // REX.W + REX.B
    assert_eq!(code[1], 0xB8); // MOV r8, imm64
    assert_eq!(code.len(), 10);
}

// ==========================================================
// NOP size tests (3-7 bytes)
// ==========================================================

#[test]
fn nop_3() {
    // 3-byte NOP: 0F 1F 00
    let code = emit_bytes(|b| emit_nops(b, 3));
    assert_eq!(code, [0x0F, 0x1F, 0x00]);
}

#[test]
fn nop_4() {
    // 4-byte NOP: 0F 1F 40 00
    let code = emit_bytes(|b| emit_nops(b, 4));
    assert_eq!(code, [0x0F, 0x1F, 0x40, 0x00]);
}

#[test]
fn nop_5() {
    // 5-byte NOP: 0F 1F 44 00 00
    let code = emit_bytes(|b| emit_nops(b, 5));
    assert_eq!(code, [0x0F, 0x1F, 0x44, 0x00, 0x00]);
}

#[test]
fn nop_6() {
    // 6-byte NOP: 66 0F 1F 44 00 00
    let code = emit_bytes(|b| emit_nops(b, 6));
    assert_eq!(code, [0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00]);
}

#[test]
fn nop_7() {
    // 7-byte NOP: 0F 1F 80 00 00 00 00
    let code = emit_bytes(|b| emit_nops(b, 7));
    assert_eq!(code, [0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00]);
}

#[test]
fn nop_large_uses_8byte_chunks() {
    // 16 bytes should be two 8-byte NOPs
    let code = emit_bytes(|b| emit_nops(b, 16));
    assert_eq!(code.len(), 16);
}

// ==========================================================
// Additional encoding edge cases
// ==========================================================

#[test]
fn store_64_rsp_base() {
    // mov [rsp+0x10], rax => 48 89 44 24 10
    let code = emit_bytes(|b| emit_store(b, true, Reg::Rax, Reg::Rsp, 0x10));
    assert_eq!(code, [0x48, 0x89, 0x44, 0x24, 0x10]);
}

#[test]
fn lea_rbp_zero() {
    // lea rax, [rbp+0] => 48 8D 45 00
    let code = emit_bytes(|b| emit_lea(b, true, Reg::Rax, Reg::Rbp, 0));
    assert_eq!(code, [0x48, 0x8D, 0x45, 0x00]);
}

#[test]
fn load_disp32_rsp() {
    // mov rax, [rsp+0x1000] => 48 8B 84 24 00 10 00 00
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::Rsp, 0x1000));
    assert_eq!(code, [0x48, 0x8B, 0x84, 0x24, 0x00, 0x10, 0x00, 0x00]);
}

#[test]
fn store_imm_r13_base() {
    // mov dword [r13+0], 0x42 => 41 C7 45 00 42 00 00 00
    let code = emit_bytes(|b| emit_store_imm(b, false, Reg::R13, 0, 0x42));
    assert_eq!(code, [0x41, 0xC7, 0x45, 0x00, 0x42, 0x00, 0x00, 0x00]);
}

#[test]
fn push_imm_negative() {
    // push -1 => 6A FF
    let code = emit_bytes(|b| emit_push_imm(b, -1));
    assert_eq!(code, [0x6A, 0xFF]);
}

#[test]
fn mov_rr_mixed_ext() {
    // mov rax, r8 => 4C 89 C0
    let code = emit_bytes(|b| emit_mov_rr(b, true, Reg::Rax, Reg::R8));
    assert_eq!(code, [0x4C, 0x89, 0xC0]);
}

#[test]
fn mov_rr_ext_to_base() {
    // mov r8, rax => 49 89 C0
    let code = emit_bytes(|b| emit_mov_rr(b, true, Reg::R8, Reg::Rax));
    assert_eq!(code, [0x49, 0x89, 0xC0]);
}

#[test]
fn imul_ri_extended_64() {
    // imul r8, r9, 10 => 4D 6B C1 0A
    let code = emit_bytes(|b| emit_imul_ri(b, true, Reg::R8, Reg::R9, 10));
    assert_eq!(code, [0x4D, 0x6B, 0xC1, 0x0A]);
}

#[test]
fn idiv_32() {
    // idiv ecx => F7 F9
    let code = emit_bytes(|b| emit_idiv(b, false, Reg::Rcx));
    assert_eq!(code, [0xF7, 0xF9]);
}

#[test]
fn neg_64() {
    // neg rax => 48 F7 D8
    let code = emit_bytes(|b| emit_neg(b, true, Reg::Rax));
    assert_eq!(code, [0x48, 0xF7, 0xD8]);
}

#[test]
fn not_64() {
    // not rax => 48 F7 D0
    let code = emit_bytes(|b| emit_not(b, true, Reg::Rax));
    assert_eq!(code, [0x48, 0xF7, 0xD0]);
}

#[test]
fn shift_shr_1_64() {
    // shr rax, 1 => 48 D1 E8
    let code =
        emit_bytes(|b| emit_shift_ri(b, ShiftOp::Shr, true, Reg::Rax, 1));
    assert_eq!(code, [0x48, 0xD1, 0xE8]);
}

#[test]
fn load_rsp_zero() {
    // mov rax, [rsp] => 48 8B 04 24
    let code = emit_bytes(|b| emit_load(b, true, Reg::Rax, Reg::Rsp, 0));
    assert_eq!(code, [0x48, 0x8B, 0x04, 0x24]);
}

// ==========================================================
// Additional backend regression and encoding tests
// ==========================================================

fn emit_tcg_op_bytes(
    op: Op,
    oregs: &[u8],
    iregs: &[u8],
    cargs: &[u32],
) -> Vec<u8> {
    let mut buf = CodeBuffer::new(128).unwrap();
    let gen = X86_64CodeGen::new();
    let mut ctx = Context::new();
    gen.init_context(&mut ctx);
    gen.tcg_out_op(&mut buf, &ctx, &op, oregs, iregs, cargs);
    buf.as_slice().to_vec()
}

macro_rules! emit_case {
    ($name:ident, $expected:expr, $body:expr) => {
        #[test]
        fn $name() {
            let code = emit_bytes($body);
            assert_eq!(code, $expected);
        }
    };
}

macro_rules! jcc_case {
    ($name:ident, $cond:expr, $byte:expr) => {
        #[test]
        fn $name() {
            let mut buf = CodeBuffer::new(64).unwrap();
            emit_jcc(&mut buf, $cond, 0);
            let code = buf.as_slice();
            assert_eq!(code[0], 0x0F);
            assert_eq!(code[1], $byte);
        }
    };
}

#[test]
fn codegen_sub_alias_rhs_i64() {
    let op = Op::new(OpIdx(0), Opcode::Sub, Type::I64);
    let code = emit_tcg_op_bytes(
        op,
        &[Reg::Rax as u8],
        &[Reg::Rcx as u8, Reg::Rax as u8],
        &[],
    );
    assert_eq!(code, [0x48, 0xF7, 0xD8, 0x48, 0x03, 0xC1]);
}

#[test]
fn codegen_sub_alias_rhs_i32() {
    let op = Op::new(OpIdx(0), Opcode::Sub, Type::I32);
    let code = emit_tcg_op_bytes(
        op,
        &[Reg::Rax as u8],
        &[Reg::Rcx as u8, Reg::Rax as u8],
        &[],
    );
    assert_eq!(code, [0xF7, 0xD8, 0x03, 0xC1]);
}

#[test]
fn codegen_setcond_movzx_sil() {
    let op = Op::new(OpIdx(0), Opcode::SetCond, Type::I64);
    let code = emit_tcg_op_bytes(
        op,
        &[Reg::Rsi as u8],
        &[Reg::Rax as u8, Reg::Rcx as u8],
        &[tcg_core::Cond::Eq as u32],
    );
    assert_eq!(
        code,
        [
            0x48, 0x3B, 0xC1, 0x40, 0x0F, 0x94, 0xC6, 0x40, 0x0F, 0xB6,
            0xF6
        ]
    );
}

#[test]
fn codegen_setcond_movzx_dil() {
    let op = Op::new(OpIdx(0), Opcode::SetCond, Type::I64);
    let code = emit_tcg_op_bytes(
        op,
        &[Reg::Rdi as u8],
        &[Reg::Rax as u8, Reg::Rcx as u8],
        &[tcg_core::Cond::Eq as u32],
    );
    assert_eq!(
        code,
        [
            0x48, 0x3B, 0xC1, 0x40, 0x0F, 0x94, 0xC7, 0x40, 0x0F, 0xB6,
            0xFF
        ]
    );
}

emit_case!(
    movzx_sil_reg,
    [0x40, 0x0F, 0xB6, 0xC6],
    |b| emit_movzx(b, OPC_MOVZBL | P_REXB_RM, Reg::Rax, Reg::Rsi)
);
emit_case!(
    movzx_dil_reg,
    [0x40, 0x0F, 0xB6, 0xC7],
    |b| emit_movzx(b, OPC_MOVZBL | P_REXB_RM, Reg::Rax, Reg::Rdi)
);
emit_case!(
    movzx_bpl_reg,
    [0x40, 0x0F, 0xB6, 0xC5],
    |b| emit_movzx(b, OPC_MOVZBL | P_REXB_RM, Reg::Rax, Reg::Rbp)
);
emit_case!(
    movzx_spl_reg,
    [0x40, 0x0F, 0xB6, 0xC4],
    |b| emit_movzx(b, OPC_MOVZBL | P_REXB_RM, Reg::Rax, Reg::Rsp)
);
emit_case!(
    movsx_sil_reg,
    [0x40, 0x0F, 0xBE, 0xC6],
    |b| emit_movsx(b, OPC_MOVSBL | P_REXB_RM, Reg::Rax, Reg::Rsi)
);
emit_case!(
    movsx_dil_reg,
    [0x40, 0x0F, 0xBE, 0xC7],
    |b| emit_movsx(b, OPC_MOVSBL | P_REXB_RM, Reg::Rax, Reg::Rdi)
);
emit_case!(
    movsx_bpl_reg,
    [0x40, 0x0F, 0xBE, 0xC5],
    |b| emit_movsx(b, OPC_MOVSBL | P_REXB_RM, Reg::Rax, Reg::Rbp)
);
emit_case!(
    movsx_spl_reg,
    [0x40, 0x0F, 0xBE, 0xC4],
    |b| emit_movsx(b, OPC_MOVSBL | P_REXB_RM, Reg::Rax, Reg::Rsp)
);
emit_case!(
    movzx_r8b_reg,
    [0x41, 0x0F, 0xB6, 0xC0],
    |b| emit_movzx(b, OPC_MOVZBL, Reg::Rax, Reg::R8)
);
emit_case!(
    movzx_r15b_reg,
    [0x41, 0x0F, 0xB6, 0xC7],
    |b| emit_movzx(b, OPC_MOVZBL, Reg::Rax, Reg::R15)
);
emit_case!(
    store_byte_sil_base_rcx,
    [0x40, 0x88, 0x71, 0x10],
    |b| emit_store_byte(b, Reg::Rsi, Reg::Rcx, 0x10)
);
emit_case!(
    store_byte_dil_base_rcx,
    [0x40, 0x88, 0x79, 0x10],
    |b| emit_store_byte(b, Reg::Rdi, Reg::Rcx, 0x10)
);
emit_case!(
    store_byte_sil_base_r12,
    [0x41, 0x88, 0x74, 0x24, 0x20],
    |b| emit_store_byte(b, Reg::Rsi, Reg::R12, 0x20)
);
emit_case!(
    load_rbp_disp32,
    [0x48, 0x8B, 0x85, 0x34, 0x12, 0x00, 0x00],
    |b| emit_load(b, true, Reg::Rax, Reg::Rbp, 0x1234)
);
emit_case!(
    load_rsp_disp32,
    [0x48, 0x8B, 0x84, 0x24, 0x34, 0x12, 0x00, 0x00],
    |b| emit_load(b, true, Reg::Rax, Reg::Rsp, 0x1234)
);
emit_case!(
    load_r12_disp32,
    [0x49, 0x8B, 0x84, 0x24, 0x34, 0x12, 0x00, 0x00],
    |b| emit_load(b, true, Reg::Rax, Reg::R12, 0x1234)
);
emit_case!(
    load_r13_disp32,
    [0x49, 0x8B, 0x85, 0x34, 0x12, 0x00, 0x00],
    |b| emit_load(b, true, Reg::Rax, Reg::R13, 0x1234)
);
emit_case!(
    load_sib_r13_r14_disp8_neg,
    [0x4B, 0x8B, 0x44, 0xF5, 0xE0],
    |b| emit_load_sib(b, true, Reg::Rax, Reg::R13, Reg::R14, 3, -0x20)
);
emit_case!(
    store_sib_r12_r9_disp8,
    [0x4B, 0x89, 0x44, 0x8C, 0x7F],
    |b| emit_store_sib(b, true, Reg::Rax, Reg::R12, Reg::R9, 2, 0x7F)
);
emit_case!(
    lea_sib_rbp_rdx_disp0,
    [0x48, 0x8D, 0x44, 0x15, 0x00],
    |b| emit_lea_sib(b, true, Reg::Rax, Reg::Rbp, Reg::Rdx, 0, 0)
);
emit_case!(
    sib_byte_reg_rex,
    [0x40, 0x88, 0x34, 0x50],
    |b| {
        emit_modrm_sib(
            b,
            OPC_MOVB_EvGv | P_REXB_R,
            Reg::Rsi,
            Reg::Rax,
            Reg::Rdx,
            1,
            0,
        )
    }
);
emit_case!(
    test_rr_extended_64,
    [0x4D, 0x85, 0xC1],
    |b| emit_test_rr(b, true, Reg::R8, Reg::R9)
);
emit_case!(
    test_rr_extended_32,
    [0x45, 0x85, 0xE5],
    |b| emit_test_rr(b, false, Reg::R12, Reg::R13)
);
emit_case!(
    shift_shl_imm0,
    [0xC1, 0xE0, 0x00],
    |b| emit_shift_ri(b, ShiftOp::Shl, false, Reg::Rax, 0)
);
emit_case!(
    shift_shr_imm31,
    [0xC1, 0xE8, 0x1F],
    |b| emit_shift_ri(b, ShiftOp::Shr, false, Reg::Rax, 31)
);
emit_case!(
    shift_sar_imm63,
    [0x48, 0xC1, 0xF8, 0x3F],
    |b| emit_shift_ri(b, ShiftOp::Sar, true, Reg::Rax, 63)
);
emit_case!(
    shift_shl_r9_imm0,
    [0x49, 0xC1, 0xE1, 0x00],
    |b| emit_shift_ri(b, ShiftOp::Shl, true, Reg::R9, 0)
);
emit_case!(
    mov_ri_movabs_u64,
    [0x48, 0xB8, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00],
    |b| emit_mov_ri(b, true, Reg::Rax, 0x1_0000_0000)
);
emit_case!(
    mov_ri_signext_min_i32,
    [0x48, 0xC7, 0xC0, 0x00, 0x00, 0x00, 0x80],
    |b| emit_mov_ri(b, true, Reg::Rax, 0xFFFF_FFFF_8000_0000)
);
emit_case!(
    cmovb_test,
    [0x0F, 0x42, 0xC1],
    |b| emit_cmovcc(b, X86Cond::Jb, false, Reg::Rax, Reg::Rcx)
);
emit_case!(
    cmovae_test,
    [0x48, 0x0F, 0x43, 0xC1],
    |b| emit_cmovcc(b, X86Cond::Jae, true, Reg::Rax, Reg::Rcx)
);
emit_case!(
    cmovl_test,
    [0x0F, 0x4C, 0xC1],
    |b| emit_cmovcc(b, X86Cond::Jl, false, Reg::Rax, Reg::Rcx)
);
emit_case!(
    cmovg_test,
    [0x48, 0x0F, 0x4F, 0xC1],
    |b| emit_cmovcc(b, X86Cond::Jg, true, Reg::Rax, Reg::Rcx)
);

jcc_case!(jcc_jne_opcode, X86Cond::Jne, 0x85);
jcc_case!(jcc_jb_opcode, X86Cond::Jb, 0x82);
jcc_case!(jcc_jae_opcode, X86Cond::Jae, 0x83);
jcc_case!(jcc_jbe_opcode, X86Cond::Jbe, 0x86);
jcc_case!(jcc_ja_opcode, X86Cond::Ja, 0x87);
jcc_case!(jcc_jl_opcode, X86Cond::Jl, 0x8C);
jcc_case!(jcc_jge_opcode, X86Cond::Jge, 0x8D);
jcc_case!(jcc_jg_opcode, X86Cond::Jg, 0x8F);
