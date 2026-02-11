use std::fmt;
use std::fs;
use std::path::Path;

use crate::elf::*;
use crate::guest_space::*;

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Elf(ElfError),
    NoLoadSegment,
    SegmentOutOfRange,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O: {e}"),
            Self::Elf(e) => write!(f, "ELF: {e}"),
            Self::NoLoadSegment => {
                write!(f, "no PT_LOAD segment")
            }
            Self::SegmentOutOfRange => {
                write!(f, "segment out of range")
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ElfError> for LoadError {
    fn from(e: ElfError) -> Self {
        Self::Elf(e)
    }
}

/// Result of loading an ELF binary.
pub struct ElfInfo {
    pub entry: u64,
    pub phdr_addr: u64,
    pub phnum: u16,
    pub sp: u64,
    pub brk: u64,
}

/// Convert ELF p_flags to mmap prot flags.
fn elf_to_prot(flags: u32) -> i32 {
    let mut prot = 0;
    if flags & PF_R != 0 {
        prot |= libc::PROT_READ;
    }
    if flags & PF_W != 0 {
        prot |= libc::PROT_WRITE;
    }
    if flags & PF_X != 0 {
        prot |= libc::PROT_EXEC;
    }
    prot
}

/// Load a static RISC-V 64-bit ELF executable.
pub fn load_elf(
    path: &Path,
    space: &mut GuestSpace,
    argv: &[&str],
    envp: &[&str],
) -> Result<ElfInfo, LoadError> {
    let data = fs::read(path)?;
    let ehdr = Elf64Ehdr::from_bytes(&data)?;
    ehdr.validate_riscv64()?;
    let phdrs = ehdr.program_headers(&data)?;

    let mut brk: u64 = 0;
    let mut has_load = false;
    let mut phdr_addr: u64 = 0;

    // Find phdr_addr from PT_PHDR or first PT_LOAD
    let mut first_load_vaddr: Option<u64> = None;
    for ph in phdrs {
        if ph.p_type == PT_PHDR {
            phdr_addr = ph.p_vaddr;
        }
        if ph.p_type == PT_LOAD && first_load_vaddr.is_none() {
            first_load_vaddr = Some(ph.p_vaddr);
        }
    }
    if phdr_addr == 0 {
        if let Some(base) = first_load_vaddr {
            phdr_addr = base + ehdr.e_phoff;
        }
    }

    // Load PT_LOAD segments
    for ph in phdrs {
        if ph.p_type != PT_LOAD {
            continue;
        }
        has_load = true;

        let aligned_start = page_align_down(ph.p_vaddr);
        let aligned_end = page_align_up(ph.p_vaddr + ph.p_memsz);
        let aligned_size = (aligned_end - aligned_start) as usize;

        if aligned_end as usize > GUEST_STACK_TOP as usize {
            return Err(LoadError::SegmentOutOfRange);
        }

        // Map RW first for data copy
        space.mmap_fixed(
            aligned_start,
            aligned_size,
            libc::PROT_READ | libc::PROT_WRITE,
        )?;

        // Copy file data
        if ph.p_filesz > 0 {
            let src_off = ph.p_offset as usize;
            let src_end = src_off + ph.p_filesz as usize;
            if src_end > data.len() {
                return Err(LoadError::Elf(ElfError::InvalidPhdr));
            }
            unsafe {
                space.write_bytes(ph.p_vaddr, &data[src_off..src_end]);
            }
        }

        // Set final permissions
        let prot = elf_to_prot(ph.p_flags);
        if prot != (libc::PROT_READ | libc::PROT_WRITE) {
            space.mprotect(aligned_start, aligned_size, prot)?;
        }

        // Track brk
        let seg_end = page_align_up(ph.p_vaddr + ph.p_memsz);
        if seg_end > brk {
            brk = seg_end;
        }
    }

    if !has_load {
        return Err(LoadError::NoLoadSegment);
    }

    space.set_brk(brk);

    let sp =
        setup_stack(space, ehdr.e_entry, phdr_addr, ehdr.e_phnum, argv, envp)?;

    Ok(ElfInfo {
        entry: ehdr.e_entry,
        phdr_addr,
        phnum: ehdr.e_phnum,
        sp,
        brk,
    })
}

/// Build initial stack per Linux ABI.
fn setup_stack(
    space: &GuestSpace,
    entry: u64,
    phdr_addr: u64,
    phnum: u16,
    argv: &[&str],
    envp: &[&str],
) -> Result<u64, LoadError> {
    let stack_top = GUEST_STACK_TOP;
    let stack_base = stack_top - GUEST_STACK_SIZE as u64;

    // Map stack
    space.mmap_fixed(
        stack_base,
        GUEST_STACK_SIZE,
        libc::PROT_READ | libc::PROT_WRITE,
    )?;

    // Build from top down
    let mut pos = stack_top;

    // 16 bytes random data for AT_RANDOM
    pos -= 16;
    let random_addr = pos;
    // Fill with pseudo-random (deterministic seed)
    let random_data: [u8; 16] = [
        0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe, 0x01, 0x23, 0x45, 0x67,
        0x89, 0xab, 0xcd, 0xef,
    ];
    unsafe {
        space.write_bytes(random_addr, &random_data);
    }

    // Write env strings, collect guest addrs
    let mut envp_addrs = Vec::with_capacity(envp.len());
    for &s in envp.iter().rev() {
        let bytes = s.as_bytes();
        pos -= (bytes.len() + 1) as u64; // +1 NUL
        envp_addrs.push(pos);
        unsafe {
            space.write_bytes(pos, bytes);
            // NUL terminator (mmap zero-init)
        }
    }
    envp_addrs.reverse();

    // Write argv strings, collect guest addrs
    let mut argv_addrs = Vec::with_capacity(argv.len());
    for &s in argv.iter().rev() {
        let bytes = s.as_bytes();
        pos -= (bytes.len() + 1) as u64;
        argv_addrs.push(pos);
        unsafe {
            space.write_bytes(pos, bytes);
        }
    }
    argv_addrs.reverse();

    // Align to 16 bytes
    pos &= !15;

    // Calculate total size of the stack frame:
    // argc + argv ptrs + NULL + envp ptrs + NULL
    // + auxv entries (7 pairs) + AT_NULL
    let argc = argv.len();
    let envc = envp.len();
    let auxv_count = 7; // 6 entries + AT_NULL
    let frame_u64s = 1 + argc + 1 + envc + 1 + auxv_count * 2;
    pos -= (frame_u64s * 8) as u64;
    // Align SP to 16
    pos &= !15;

    let sp = pos;
    let mut cur = sp;

    // argc
    unsafe { space.write_u64(cur, argc as u64) };
    cur += 8;

    // argv pointers
    for &addr in &argv_addrs {
        unsafe { space.write_u64(cur, addr) };
        cur += 8;
    }
    // argv NULL terminator
    unsafe { space.write_u64(cur, 0) };
    cur += 8;

    // envp pointers
    for &addr in &envp_addrs {
        unsafe { space.write_u64(cur, addr) };
        cur += 8;
    }
    // envp NULL terminator
    unsafe { space.write_u64(cur, 0) };
    cur += 8;

    // Auxiliary vector
    let auxv: [(u64, u64); 7] = [
        (AT_PHDR, phdr_addr),
        (AT_PHENT, 56), // sizeof(Elf64Phdr)
        (AT_PHNUM, phnum as u64),
        (AT_PAGESZ, page_size() as u64),
        (AT_ENTRY, entry),
        (AT_RANDOM, random_addr),
        (AT_NULL, 0),
    ];
    for (typ, val) in auxv {
        unsafe {
            space.write_u64(cur, typ);
            space.write_u64(cur + 8, val);
        }
        cur += 16;
    }

    Ok(sp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::mem;
    use std::sync::atomic::{AtomicU32, Ordering};

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
        buf[16..18].copy_from_slice(&2u16.to_le_bytes());
        // e_machine = EM_RISCV
        buf[18..20].copy_from_slice(&243u16.to_le_bytes());
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
        buf[ph_off..ph_off + 4].copy_from_slice(&1u32.to_le_bytes());
        // p_flags = PF_R | PF_X
        buf[ph_off + 4..ph_off + 8].copy_from_slice(&5u32.to_le_bytes());
        // p_offset
        buf[ph_off + 8..ph_off + 16]
            .copy_from_slice(&(code_offset as u64).to_le_bytes());
        // p_vaddr
        buf[ph_off + 16..ph_off + 24]
            .copy_from_slice(&load_vaddr.to_le_bytes());
        // p_paddr
        buf[ph_off + 24..ph_off + 32]
            .copy_from_slice(&load_vaddr.to_le_bytes());
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
        let path = std::path::PathBuf::from(format!(
            "/tmp/tcg_test_elf_{pid}_{n}.bin"
        ));
        let file = fs::File::create(&path)?;
        Ok(TempFile { path, file })
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
        }
    }
}
