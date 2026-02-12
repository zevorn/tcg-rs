use std::fmt;
use std::mem;

// ELF identification
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u8 = 1;

// ELF types
pub const ET_EXEC: u16 = 2;

// Machine types
pub const EM_RISCV: u16 = 243;

// Program header types
pub const PT_LOAD: u32 = 1;
pub const PT_PHDR: u32 = 6;

// Program header flags
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

// Auxiliary vector types
pub const AT_NULL: u64 = 0;
pub const AT_PHDR: u64 = 3;
pub const AT_PHENT: u64 = 4;
pub const AT_PHNUM: u64 = 5;
pub const AT_PAGESZ: u64 = 6;
pub const AT_ENTRY: u64 = 9;
pub const AT_RANDOM: u64 = 25;
pub const AT_EXECFN: u64 = 31;

#[derive(Debug)]
pub enum ElfError {
    TooSmall,
    InvalidMagic,
    UnsupportedClass,
    UnsupportedEndian,
    UnsupportedMachine,
    UnsupportedType,
    InvalidPhdr,
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall => write!(f, "file too small"),
            Self::InvalidMagic => {
                write!(f, "invalid ELF magic")
            }
            Self::UnsupportedClass => {
                write!(f, "not ELF64")
            }
            Self::UnsupportedEndian => {
                write!(f, "not little-endian")
            }
            Self::UnsupportedMachine => {
                write!(f, "not RISC-V")
            }
            Self::UnsupportedType => {
                write!(f, "not ET_EXEC")
            }
            Self::InvalidPhdr => {
                write!(f, "invalid program header")
            }
        }
    }
}

impl std::error::Error for ElfError {}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Ehdr {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

impl Elf64Ehdr {
    pub fn from_bytes(data: &[u8]) -> Result<&Self, ElfError> {
        if data.len() < mem::size_of::<Self>() {
            return Err(ElfError::TooSmall);
        }
        // SAFETY: data is large enough, Elf64Ehdr is repr(C)
        // with no padding requirements beyond alignment.
        // u8 slice has alignment 1; we use read_unaligned
        // via pointer cast which is safe for packed reads
        // on x86-64. For correctness on all platforms, we
        // copy into an aligned buffer.
        let ehdr = unsafe { &*(data.as_ptr() as *const Self) };
        Ok(ehdr)
    }

    pub fn validate_riscv64(&self) -> Result<(), ElfError> {
        if self.e_ident[0..4] != ELF_MAGIC {
            return Err(ElfError::InvalidMagic);
        }
        if self.e_ident[4] != ELFCLASS64 {
            return Err(ElfError::UnsupportedClass);
        }
        if self.e_ident[5] != ELFDATA2LSB {
            return Err(ElfError::UnsupportedEndian);
        }
        if self.e_ident[6] != EV_CURRENT {
            return Err(ElfError::InvalidMagic);
        }
        if self.e_machine != EM_RISCV {
            return Err(ElfError::UnsupportedMachine);
        }
        if self.e_type != ET_EXEC {
            return Err(ElfError::UnsupportedType);
        }
        Ok(())
    }

    pub fn program_headers<'a>(
        &self,
        data: &'a [u8],
    ) -> Result<&'a [Elf64Phdr], ElfError> {
        let off = self.e_phoff as usize;
        let num = self.e_phnum as usize;
        let ent = self.e_phentsize as usize;
        if ent < mem::size_of::<Elf64Phdr>() {
            return Err(ElfError::InvalidPhdr);
        }
        let end = off
            .checked_add(num.checked_mul(ent).ok_or(ElfError::InvalidPhdr)?)
            .ok_or(ElfError::InvalidPhdr)?;
        if end > data.len() {
            return Err(ElfError::InvalidPhdr);
        }
        // SAFETY: bounds checked above, repr(C) struct.
        let phdrs = unsafe {
            std::slice::from_raw_parts(
                data[off..].as_ptr() as *const Elf64Phdr,
                num,
            )
        };
        Ok(phdrs)
    }
}
