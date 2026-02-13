//! RISC-V ISA extension configuration.
//!
//! Provides MISA-style letter-extension bitmask (`MisaExt`) and a
//! per-CPU configuration struct (`RiscvCfg`) that mirrors QEMU's
//! `RISCVCPUConfig`.  Zero external dependencies — all bit ops are
//! `const`.

// ── MISA letter-extension bitmask ────────────────────────────────

/// Bitmask of single-letter RISC-V extensions (MISA bits).
///
/// Bit layout follows the RISC-V spec: bit N = extension whose
/// letter is `'A' + N`.  This matches QEMU's `RV('X')` macro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MisaExt(u32);

#[allow(non_upper_case_globals)]
impl MisaExt {
    pub const EMPTY: Self = Self(0);
    pub const I: Self = Self(1 << (b'I' - b'A'));
    pub const M: Self = Self(1 << (b'M' - b'A'));
    pub const A: Self = Self(1 << 0); // bit 0 = 'A'
    pub const F: Self = Self(1 << (b'F' - b'A'));
    pub const D: Self = Self(1 << (b'D' - b'A'));
    pub const C: Self = Self(1 << (b'C' - b'A'));

    /// G = IMAFD (general-purpose).
    pub const G: Self =
        Self(Self::I.0 | Self::M.0 | Self::A.0 | Self::F.0 | Self::D.0);

    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[inline]
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & ((1 << 26) - 1))
    }

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

// ── Extension configuration ──────────────────────────────────────

/// Per-CPU RISC-V extension configuration.
///
/// `misa` covers single-letter extensions; boolean fields cover
/// Z-extensions.  Only extensions that tcg-rs already implements
/// (or will implement soon) are listed.
#[derive(Clone, Copy, Debug)]
pub struct RiscvCfg {
    pub misa: MisaExt,
    // Z-extensions (user-mode relevant)
    pub ext_zicsr: bool,
    pub ext_zifencei: bool,
    pub ext_zba: bool,
    pub ext_zbb: bool,
    pub ext_zbc: bool,
    pub ext_zbs: bool,
}

// ── Predefined profiles ──────────────────────────────────────────

impl RiscvCfg {
    /// RV64GC = RV64IMAFDC + Zicsr + Zifencei.
    pub const RV64IMAFDC: Self = Self {
        misa: MisaExt::from_bits_truncate(
            MisaExt::I.0
                | MisaExt::M.0
                | MisaExt::A.0
                | MisaExt::F.0
                | MisaExt::D.0
                | MisaExt::C.0,
        ),
        ext_zicsr: true,
        ext_zifencei: true,
        ext_zba: false,
        ext_zbb: false,
        ext_zbc: false,
        ext_zbs: false,
    };
}

impl Default for RiscvCfg {
    fn default() -> Self {
        Self::RV64IMAFDC
    }
}
