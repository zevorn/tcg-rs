/// TCG IR value types.
///
/// Maps to QEMU's `TCGType` — represents the width of IR operands.
/// Integer types (I32/I64) are used for scalar ops; vector types for SIMD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Type {
    I32 = 0,
    I64 = 1,
    I128 = 2,
    V64 = 3,
    V128 = 4,
    V256 = 5,
}

pub const TYPE_COUNT: usize = 6;

impl Type {
    pub const fn size_bits(self) -> u32 {
        match self {
            Type::I32 => 32,
            Type::I64 => 64,
            Type::I128 => 128,
            Type::V64 => 64,
            Type::V128 => 128,
            Type::V256 => 256,
        }
    }

    pub const fn size_bytes(self) -> u32 {
        self.size_bits() / 8
    }

    pub const fn is_vector(self) -> bool {
        matches!(self, Type::V64 | Type::V128 | Type::V256)
    }

    pub const fn is_integer(self) -> bool {
        matches!(self, Type::I32 | Type::I64 | Type::I128)
    }
}

/// Runtime location of a TCG temporary's value during register allocation.
///
/// Maps to QEMU's `TCGTempVal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TempVal {
    Dead = 0,
    Reg = 1,
    Mem = 2,
    Const = 3,
}

/// Comparison conditions for branch/setcond operations.
///
/// Maps to QEMU's `TCGCond`. Encoding matches QEMU for direct translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Cond {
    Never = 0,
    Always = 1,
    Eq = 8,
    Ne = 9,
    // Signed
    Lt = 10,
    Ge = 11,
    Le = 12,
    Gt = 13,
    // Unsigned
    Ltu = 14,
    Geu = 15,
    Leu = 16,
    Gtu = 17,
    // Test (AND then compare vs 0)
    TstEq = 18,
    TstNe = 19,
}

impl Cond {
    /// Return the inverted condition.
    pub const fn invert(self) -> Cond {
        match self {
            Cond::Never => Cond::Always,
            Cond::Always => Cond::Never,
            Cond::Eq => Cond::Ne,
            Cond::Ne => Cond::Eq,
            Cond::Lt => Cond::Ge,
            Cond::Ge => Cond::Lt,
            Cond::Le => Cond::Gt,
            Cond::Gt => Cond::Le,
            Cond::Ltu => Cond::Geu,
            Cond::Geu => Cond::Ltu,
            Cond::Leu => Cond::Gtu,
            Cond::Gtu => Cond::Leu,
            Cond::TstEq => Cond::TstNe,
            Cond::TstNe => Cond::TstEq,
        }
    }

    /// Swap operand order (e.g. Lt becomes Gt).
    pub const fn swap(self) -> Cond {
        match self {
            Cond::Eq
            | Cond::Ne
            | Cond::Never
            | Cond::Always
            | Cond::TstEq
            | Cond::TstNe => self,
            Cond::Lt => Cond::Gt,
            Cond::Ge => Cond::Le,
            Cond::Le => Cond::Ge,
            Cond::Gt => Cond::Lt,
            Cond::Ltu => Cond::Gtu,
            Cond::Geu => Cond::Leu,
            Cond::Leu => Cond::Geu,
            Cond::Gtu => Cond::Ltu,
        }
    }

    pub const fn is_signed(self) -> bool {
        matches!(self, Cond::Lt | Cond::Ge | Cond::Le | Cond::Gt)
    }

    pub const fn is_unsigned(self) -> bool {
        matches!(self, Cond::Ltu | Cond::Geu | Cond::Leu | Cond::Gtu)
    }

    pub const fn is_tst(self) -> bool {
        matches!(self, Cond::TstEq | Cond::TstNe)
    }
}

/// Memory operation descriptor — encodes size, signedness,
/// endianness, alignment.
///
/// Maps to QEMU's `MemOp`. Bit-packed for compact storage in IR ops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemOp(u16);

impl MemOp {
    pub const SIZE_8: u16 = 0;
    pub const SIZE_16: u16 = 1;
    pub const SIZE_32: u16 = 2;
    pub const SIZE_64: u16 = 3;

    pub const SIGN: u16 = 1 << 2;
    pub const BSWAP: u16 = 1 << 3;
    pub const ALIGN_2: u16 = 1 << 4;
    pub const ALIGN_4: u16 = 2 << 4;
    pub const ALIGN_8: u16 = 3 << 4;
    pub const ALIGN_16: u16 = 4 << 4;
    pub const ALIGN_32: u16 = 5 << 4;
    pub const ALIGN_64: u16 = 6 << 4;

    pub const fn new(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn ub() -> Self {
        Self(Self::SIZE_8)
    }
    pub const fn sb() -> Self {
        Self(Self::SIZE_8 | Self::SIGN)
    }
    pub const fn uw() -> Self {
        Self(Self::SIZE_16)
    }
    pub const fn sw() -> Self {
        Self(Self::SIZE_16 | Self::SIGN)
    }
    pub const fn ul() -> Self {
        Self(Self::SIZE_32)
    }
    pub const fn sl() -> Self {
        Self(Self::SIZE_32 | Self::SIGN)
    }
    pub const fn uq() -> Self {
        Self(Self::SIZE_64)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }
    pub const fn size(self) -> u16 {
        self.0 & 0x3
    }
    pub const fn is_signed(self) -> bool {
        self.0 & Self::SIGN != 0
    }
    pub const fn is_bswap(self) -> bool {
        self.0 & Self::BSWAP != 0
    }
    pub const fn size_bytes(self) -> u32 {
        1 << self.size()
    }
}

/// Bitmap of host registers, used for register allocation constraints.
///
/// Maps to QEMU's `TCGRegSet`. Supports up to 64 registers.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegSet(u64);

impl RegSet {
    pub const EMPTY: RegSet = RegSet(0);

    pub const fn new() -> Self {
        Self(0)
    }

    pub const fn from_raw(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn set(self, reg: u8) -> Self {
        Self(self.0 | (1u64 << reg))
    }

    pub const fn clear(self, reg: u8) -> Self {
        Self(self.0 & !(1u64 << reg))
    }

    pub const fn contains(self, reg: u8) -> bool {
        self.0 & (1u64 << reg) != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn union(self, other: RegSet) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersect(self, other: RegSet) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn subtract(self, other: RegSet) -> Self {
        Self(self.0 & !other.0)
    }

    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }

    /// Return the lowest set register, or None.
    pub const fn first(self) -> Option<u8> {
        if self.0 == 0 {
            None
        } else {
            Some(self.0.trailing_zeros() as u8)
        }
    }
}

impl Default for RegSet {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl std::fmt::Debug for RegSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RegSet(0x{:016x})", self.0)
    }
}
