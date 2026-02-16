//! Minimal ELF64 parser — extracts entry point and PT_LOAD segments.

use std::mem;

const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Ehdr {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

/// A loaded ELF segment.
pub struct Segment {
    pub vaddr: u64,
    pub data: Vec<u8>,
    pub executable: bool,
}

/// Parsed ELF information.
pub struct ElfInfo {
    pub entry: u64,
    pub e_machine: u16,
    pub segments: Vec<Segment>,
}

/// Parse an ELF64 binary from raw bytes.
pub fn parse(data: &[u8]) -> Result<ElfInfo, String> {
    if data.len() < mem::size_of::<Elf64Ehdr>() {
        return Err("file too small for ELF header".into());
    }
    let ehdr: Elf64Ehdr =
        unsafe { std::ptr::read_unaligned(data.as_ptr() as *const _) };

    if ehdr.e_ident[..4] != ELF_MAGIC {
        return Err("not an ELF file".into());
    }
    if ehdr.e_ident[4] != ELFCLASS64 {
        return Err("not a 64-bit ELF".into());
    }

    let ph_off = ehdr.e_phoff as usize;
    let ph_ent = ehdr.e_phentsize as usize;
    let ph_num = ehdr.e_phnum as usize;

    let mut segments = Vec::new();
    for i in 0..ph_num {
        let off = ph_off + i * ph_ent;
        if off + mem::size_of::<Elf64Phdr>() > data.len() {
            return Err("phdr out of bounds".into());
        }
        let phdr: Elf64Phdr = unsafe {
            std::ptr::read_unaligned(data.as_ptr().add(off) as *const _)
        };
        if phdr.p_type != PT_LOAD {
            continue;
        }
        let foff = phdr.p_offset as usize;
        let fsz = phdr.p_filesz as usize;
        let msz = phdr.p_memsz as usize;
        if foff + fsz > data.len() {
            return Err("segment data out of bounds".into());
        }
        let mut seg_data = vec![0u8; msz];
        seg_data[..fsz].copy_from_slice(&data[foff..foff + fsz]);
        segments.push(Segment {
            vaddr: phdr.p_vaddr,
            data: seg_data,
            executable: (phdr.p_flags & PF_X) != 0,
        });
    }

    Ok(ElfInfo {
        entry: ehdr.e_entry,
        e_machine: ehdr.e_machine,
        segments,
    })
}
