use std::mem;

use tcg_linux_user::elf::{
    Elf64Ehdr, Elf64Phdr, ElfError, EM_RISCV, ET_EXEC, PT_LOAD,
};

fn make_valid_ehdr() -> Vec<u8> {
    let mut buf = vec![0u8; mem::size_of::<Elf64Ehdr>()];
    // e_ident
    buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    buf[4] = 2; // ELFCLASS64
    buf[5] = 1; // ELFDATA2LSB
    buf[6] = 1; // EV_CURRENT
                // e_type = ET_EXEC (offset 16, u16 LE)
    buf[16] = ET_EXEC as u8;
    buf[17] = (ET_EXEC >> 8) as u8;
    // e_machine = EM_RISCV (offset 18, u16 LE)
    buf[18] = EM_RISCV as u8;
    buf[19] = (EM_RISCV >> 8) as u8;
    // e_version = 1 (offset 20, u32 LE)
    buf[20] = 1;
    // e_ehsize (offset 52, u16 LE)
    let sz = mem::size_of::<Elf64Ehdr>() as u16;
    buf[52] = sz as u8;
    buf[53] = (sz >> 8) as u8;
    // e_phentsize (offset 54, u16 LE)
    let phsz = mem::size_of::<Elf64Phdr>() as u16;
    buf[54] = phsz as u8;
    buf[55] = (phsz >> 8) as u8;
    buf
}

#[test]
fn test_parse_valid_ehdr() {
    let buf = make_valid_ehdr();
    let ehdr = Elf64Ehdr::from_bytes(&buf).unwrap();
    ehdr.validate_riscv64().unwrap();
    assert_eq!(ehdr.e_machine, EM_RISCV);
    assert_eq!(ehdr.e_type, ET_EXEC);
}

#[test]
fn test_too_small() {
    let buf = [0u8; 4];
    assert!(Elf64Ehdr::from_bytes(&buf).is_err());
}

#[test]
fn test_invalid_magic() {
    let mut buf = make_valid_ehdr();
    buf[0] = 0;
    let ehdr = Elf64Ehdr::from_bytes(&buf).unwrap();
    assert!(matches!(
        ehdr.validate_riscv64(),
        Err(ElfError::InvalidMagic)
    ));
}

#[test]
fn test_wrong_class() {
    let mut buf = make_valid_ehdr();
    buf[4] = 1; // ELFCLASS32
    let ehdr = Elf64Ehdr::from_bytes(&buf).unwrap();
    assert!(matches!(
        ehdr.validate_riscv64(),
        Err(ElfError::UnsupportedClass)
    ));
}

#[test]
fn test_wrong_machine() {
    let mut buf = make_valid_ehdr();
    buf[18] = 0x3e; // EM_X86_64
    buf[19] = 0;
    let ehdr = Elf64Ehdr::from_bytes(&buf).unwrap();
    assert!(matches!(
        ehdr.validate_riscv64(),
        Err(ElfError::UnsupportedMachine)
    ));
}

#[test]
fn test_program_headers() {
    let phdr_size = mem::size_of::<Elf64Phdr>();
    let ehdr_size = mem::size_of::<Elf64Ehdr>();
    let mut buf = make_valid_ehdr();

    // Set e_phoff = ehdr_size, e_phnum = 1
    let off = ehdr_size as u64;
    buf[32..40].copy_from_slice(&off.to_le_bytes());
    buf[56] = 1; // e_phnum
    buf[57] = 0;

    // Append one Elf64Phdr
    buf.resize(ehdr_size + phdr_size, 0);
    // p_type = PT_LOAD at offset ehdr_size
    buf[ehdr_size] = PT_LOAD as u8;

    let ehdr = Elf64Ehdr::from_bytes(&buf).unwrap();
    let phdrs = ehdr.program_headers(&buf).unwrap();
    assert_eq!(phdrs.len(), 1);
    assert_eq!(phdrs[0].p_type, PT_LOAD);
}
