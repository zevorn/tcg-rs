use std::fs;
use std::io::Write;
use std::mem;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use tcg_linux_user::elf::{
    Elf64Ehdr, Elf64Phdr, AT_EXECFN, AT_NULL, AT_PHDR, EM_RISCV, ET_EXEC, PF_R,
    PF_X, PT_LOAD,
};
use tcg_linux_user::guest_space::{
    GuestSpace, GUEST_STACK_SIZE, GUEST_STACK_TOP,
};
use tcg_linux_user::loader::load_elf;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Build a minimal valid RISC-V ELF in memory.
fn make_minimal_elf() -> Vec<u8> {
    let ehdr_sz = mem::size_of::<Elf64Ehdr>();
    let phdr_sz = mem::size_of::<Elf64Phdr>();
    let code_offset = ehdr_sz + phdr_sz;
    // Minimal code: RISC-V NOP (addi x0,x0,0)
    let code: [u8; 4] = [0x13, 0x00, 0x00, 0x00];
    let file_size = code_offset + code.len();
    let load_vaddr: u64 = 0x10000;

    let mut buf = vec![0u8; file_size];

    // ELF header
    buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    buf[4] = 2; // ELFCLASS64
    buf[5] = 1; // ELFDATA2LSB
    buf[6] = 1; // EV_CURRENT
                // e_type = ET_EXEC
    buf[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
    // e_machine = EM_RISCV
    buf[18..20].copy_from_slice(&EM_RISCV.to_le_bytes());
    // e_version
    buf[20..24].copy_from_slice(&1u32.to_le_bytes());
    // e_entry
    buf[24..32].copy_from_slice(&load_vaddr.to_le_bytes());
    // e_phoff
    buf[32..40].copy_from_slice(&(ehdr_sz as u64).to_le_bytes());
    // e_ehsize
    buf[52..54].copy_from_slice(&(ehdr_sz as u16).to_le_bytes());
    // e_phentsize
    buf[54..56].copy_from_slice(&(phdr_sz as u16).to_le_bytes());
    // e_phnum = 1
    buf[56..58].copy_from_slice(&1u16.to_le_bytes());

    // Program header (PT_LOAD)
    let ph_off = ehdr_sz;
    // p_type = PT_LOAD
    buf[ph_off..ph_off + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
    // p_flags = PF_R | PF_X
    buf[ph_off + 4..ph_off + 8].copy_from_slice(&(PF_R | PF_X).to_le_bytes());
    // p_offset
    buf[ph_off + 8..ph_off + 16]
        .copy_from_slice(&(code_offset as u64).to_le_bytes());
    // p_vaddr
    buf[ph_off + 16..ph_off + 24].copy_from_slice(&load_vaddr.to_le_bytes());
    // p_paddr
    buf[ph_off + 24..ph_off + 32].copy_from_slice(&load_vaddr.to_le_bytes());
    // p_filesz
    buf[ph_off + 32..ph_off + 40]
        .copy_from_slice(&(code.len() as u64).to_le_bytes());
    // p_memsz
    buf[ph_off + 40..ph_off + 48]
        .copy_from_slice(&(code.len() as u64).to_le_bytes());
    // p_align
    buf[ph_off + 48..ph_off + 56].copy_from_slice(&4096u64.to_le_bytes());

    // Code
    buf[code_offset..code_offset + code.len()].copy_from_slice(&code);

    buf
}

/// Simple temp file helper.
struct TempFile {
    path: std::path::PathBuf,
    file: fs::File,
}

impl TempFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.file.write_all(data)?;
        self.file.flush()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn tempfile() -> std::io::Result<TempFile> {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::path::PathBuf::from(format!("/tmp/tcg_test_elf_{pid}_{n}.bin"));
    let file = fs::File::create(&path)?;
    Ok(TempFile { path, file })
}

unsafe fn read_cstr(space: &GuestSpace, addr: u64) -> String {
    let mut out = Vec::new();
    let mut off = 0u64;
    loop {
        let ch = *space.g2h(addr + off);
        if ch == 0 {
            break;
        }
        out.push(ch);
        off += 1;
    }
    String::from_utf8(out).expect("guest cstr is utf8")
}

#[test]
fn test_load_minimal_elf() {
    let elf_data = make_minimal_elf();

    let mut tmpfile = tempfile().expect("create tmpfile");
    tmpfile.write_all(&elf_data).expect("write elf");
    let path = tmpfile.path();

    let mut space = GuestSpace::new().expect("guest space");
    let info = load_elf(path, &mut space, &["./test"], &["HOME=/tmp"])
        .expect("load_elf");

    assert_eq!(info.entry, 0x10000);
    assert_eq!(info.phnum, 1);
    assert!(info.sp < GUEST_STACK_TOP);
    assert!(info.sp > GUEST_STACK_TOP - GUEST_STACK_SIZE as u64);
    assert!(info.brk > 0);

    // Verify argc on stack
    let argc = unsafe { space.read_u64(info.sp) };
    assert_eq!(argc, 1); // one argv entry
}

#[test]
fn test_stack_layout() {
    let elf_data = make_minimal_elf();
    let mut tmpfile = tempfile().expect("create tmpfile");
    tmpfile.write_all(&elf_data).expect("write elf");
    let path = tmpfile.path();

    let mut space = GuestSpace::new().expect("guest space");
    let info = load_elf(path, &mut space, &["./prog", "arg1"], &["K=V"])
        .expect("load_elf");

    let sp = info.sp;
    unsafe {
        // argc = 2
        assert_eq!(space.read_u64(sp), 2);
        // argv[0] pointer (non-null)
        let argv0 = space.read_u64(sp + 8);
        assert_ne!(argv0, 0);
        // argv[1] pointer (non-null)
        let argv1 = space.read_u64(sp + 16);
        assert_ne!(argv1, 0);
        // argv NULL terminator
        assert_eq!(space.read_u64(sp + 24), 0);
        // envp[0] pointer (non-null)
        let envp0 = space.read_u64(sp + 32);
        assert_ne!(envp0, 0);
        // envp NULL terminator
        assert_eq!(space.read_u64(sp + 40), 0);
        // First auxv: AT_PHDR
        assert_eq!(space.read_u64(sp + 48), AT_PHDR);

        // Find AT_EXECFN in auxv.
        let mut auxp = sp + 48;
        let mut execfn_ptr = 0u64;
        loop {
            let typ = space.read_u64(auxp);
            let val = space.read_u64(auxp + 8);
            auxp += 16;
            if typ == AT_EXECFN {
                execfn_ptr = val;
            }
            if typ == AT_NULL {
                break;
            }
        }
        assert_ne!(execfn_ptr, 0);
        let execfn = read_cstr(&space, execfn_ptr);
        assert!(execfn.ends_with(".bin"));
    }
}
