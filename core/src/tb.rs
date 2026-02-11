/// A cached translated code block.
///
/// Maps to QEMU's `TranslationBlock`. Represents the mapping from a
/// guest code region to generated host machine code.
#[derive(Debug)]
pub struct TranslationBlock {
    /// Guest virtual PC where this TB starts.
    pub pc: u64,
    /// CS base (x86) or 0 for other architectures.
    pub cs_base: u64,
    /// CPU state flags that affect translation (e.g. privilege level, ISA mode).
    pub flags: u32,
    /// Compile flags (instruction count limit, single-step, etc.).
    pub cflags: u32,
    /// Size of guest code covered by this TB, in bytes.
    pub size: u32,
    /// Number of guest instructions in this TB.
    pub icount: u16,

    /// Offset into the global code buffer where host code starts.
    pub host_offset: usize,
    /// Size of generated host code in bytes.
    pub host_size: usize,

    /// Offset of the `goto_tb` jump instruction for each exit (up to 2).
    /// Used by TB chaining to atomically patch the jump target.
    /// `None` means the slot is unused.
    pub jmp_insn_offset: [Option<u32>; 2],

    /// Offset right after the `goto_tb` instruction for each exit.
    /// Used to reset the jump when unlinking.
    pub jmp_reset_offset: [Option<u32>; 2],

    /// Physical page address for TB invalidation tracking.
    pub phys_pc: u64,

    /// Index of the next TB in the same hash bucket, or `None`.
    pub hash_next: Option<usize>,

    /// Whether this TB has been invalidated.
    pub invalid: bool,
}

/// Compile flags for TranslationBlock.cflags.
pub mod cflags {
    /// Mask for the instruction count limit (0 = no limit).
    pub const CF_COUNT_MASK: u32 = 0x0000_FFFF;
    /// Last I/O instruction in the TB.
    pub const CF_LAST_IO: u32 = 0x0001_0000;
    /// TB is being single-stepped.
    pub const CF_SINGLE_STEP: u32 = 0x0002_0000;
    /// Use icount (deterministic execution).
    pub const CF_USE_ICOUNT: u32 = 0x0004_0000;
}

impl TranslationBlock {
    pub fn new(pc: u64, flags: u32, cflags: u32) -> Self {
        Self {
            pc,
            cs_base: 0,
            flags,
            cflags,
            size: 0,
            icount: 0,
            host_offset: 0,
            host_size: 0,
            jmp_insn_offset: [None; 2],
            jmp_reset_offset: [None; 2],
            phys_pc: 0,
            hash_next: None,
            invalid: false,
        }
    }

    /// Compute hash bucket index for TB lookup.
    pub fn hash(pc: u64, flags: u32) -> usize {
        let h = pc.wrapping_mul(0x9e3779b97f4a7c15) ^ (flags as u64);
        (h as usize) & (TB_HASH_SIZE - 1)
    }

    /// Record the offset of a `goto_tb` jump instruction for exit slot `n`.
    pub fn set_jmp_insn_offset(&mut self, n: usize, offset: u32) {
        assert!(n < 2);
        self.jmp_insn_offset[n] = Some(offset);
    }

    /// Record the reset offset for exit slot `n`.
    pub fn set_jmp_reset_offset(&mut self, n: usize, offset: u32) {
        assert!(n < 2);
        self.jmp_reset_offset[n] = Some(offset);
    }

    /// Maximum number of guest instructions per TB.
    pub fn max_insns(cflags: u32) -> u32 {
        let count = cflags & cflags::CF_COUNT_MASK;
        if count == 0 {
            512
        } else {
            count
        }
    }
}

/// Number of buckets in the global TB hash table.
pub const TB_HASH_SIZE: usize = 1 << 15; // 32768

/// Number of entries in the per-CPU jump cache.
pub const TB_JMP_CACHE_SIZE: usize = 1 << 12; // 4096

/// Per-CPU direct-mapped TB jump cache.
///
/// Indexed by `(pc >> 2) & (TB_JMP_CACHE_SIZE - 1)`.
/// Provides O(1) lookup for the common case of re-executing the same PC.
pub struct JumpCache {
    entries: Box<[Option<usize>; TB_JMP_CACHE_SIZE]>,
}

impl JumpCache {
    pub fn new() -> Self {
        Self {
            entries: Box::new([None; TB_JMP_CACHE_SIZE]),
        }
    }

    fn index(pc: u64) -> usize {
        (pc as usize >> 2) & (TB_JMP_CACHE_SIZE - 1)
    }

    pub fn lookup(&self, pc: u64) -> Option<usize> {
        self.entries[Self::index(pc)]
    }

    pub fn insert(&mut self, pc: u64, tb_idx: usize) {
        self.entries[Self::index(pc)] = Some(tb_idx);
    }

    pub fn remove(&mut self, pc: u64) {
        self.entries[Self::index(pc)] = None;
    }

    pub fn invalidate(&mut self) {
        self.entries.fill(None);
    }
}

impl Default for JumpCache {
    fn default() -> Self {
        Self::new()
    }
}
