use tcg_backend::code_buffer::CodeBuffer;
use tcg_backend::x86_64::regs::*;
use tcg_backend::x86_64::X86_64CodeGen;
use tcg_backend::HostCodeGen;

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
    let has_add = epilogue
        .windows(3)
        .any(|w| w[0] == 0x48 && (w[1] == 0x81 || w[1] == 0x83) && w[2] == 0xC4);
    assert!(has_add, "epilogue should contain add rsp, imm");
}

#[test]
fn epilogue_pop_count_matches_push() {
    let (buf, gen) = gen_prologue_epilogue();
    let epilogue = &buf.as_slice()[gen.tb_ret_offset..];
    // Count pop instructions (0x58-0x5F for base regs, 0x41 0x58-0x5F for extended)
    let mut pop_count = 0;
    let mut i = 0;
    while i < epilogue.len() {
        if epilogue[i] == 0x41 && i + 1 < epilogue.len() && (0x58..=0x5F).contains(&epilogue[i + 1])
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
