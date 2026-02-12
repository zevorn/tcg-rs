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

    let execfn = path.to_string_lossy();
    let sp = setup_stack(
        space,
        ehdr.e_entry,
        phdr_addr,
        ehdr.e_phnum,
        argv,
        envp,
        execfn.as_ref(),
    )?;

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
    execfn: &str,
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

    // Keep original executable name for AT_EXECFN.
    let execfn_bytes = execfn.as_bytes();
    pos -= (execfn_bytes.len() + 1) as u64;
    let execfn_addr = pos;
    unsafe {
        space.write_bytes(execfn_addr, execfn_bytes);
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

    let auxv: [(u64, u64); 8] = [
        (AT_PHDR, phdr_addr),
        (AT_PHENT, 56), // sizeof(Elf64Phdr)
        (AT_PHNUM, phnum as u64),
        (AT_PAGESZ, page_size() as u64),
        (AT_ENTRY, entry),
        (AT_RANDOM, random_addr),
        (AT_EXECFN, execfn_addr),
        (AT_NULL, 0),
    ];

    // Calculate total size of the stack frame:
    // argc + argv ptrs + NULL + envp ptrs + NULL + auxv pairs
    let argc = argv.len();
    let envc = envp.len();
    let frame_u64s = 1 + argc + 1 + envc + 1 + auxv.len() * 2;
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
    for (typ, val) in auxv {
        unsafe {
            space.write_u64(cur, typ);
            space.write_u64(cur + 8, val);
        }
        cur += 16;
    }

    Ok(sp)
}
