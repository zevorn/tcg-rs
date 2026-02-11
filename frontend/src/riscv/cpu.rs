//! RISC-V CPU state for user-mode emulation.

/// Number of general-purpose registers (x0-x31).
pub const NUM_GPRS: usize = 32;

/// RISC-V CPU architectural state (RV64, user-mode).
///
/// Layout must be `#[repr(C)]` so that TCG global temps can
/// reference fields at fixed offsets from the env pointer.
#[repr(C)]
pub struct RiscvCpu {
    /// General-purpose registers x0-x31.
    /// x0 is hardwired to zero (enforced by the frontend,
    /// not by this struct).
    pub gpr: [u64; NUM_GPRS],
    /// Program counter.
    pub pc: u64,
    /// Guest memory base pointer (host address).
    /// Used by generated code to translate guest addresses.
    pub guest_base: u64,
}

// Field offsets (bytes) from the start of RiscvCpu.
// Used by `Context::new_global()` to bind IR temps.

/// Byte offset of `gpr[i]`: `i * 8`.
pub const fn gpr_offset(i: usize) -> i64 {
    (i * 8) as i64
}

/// Byte offset of the `pc` field.
pub const PC_OFFSET: i64 = (NUM_GPRS * 8) as i64; // 256

/// Byte offset of the `guest_base` field.
pub const GUEST_BASE_OFFSET: i64 = PC_OFFSET + 8; // 264

impl RiscvCpu {
    pub fn new() -> Self {
        Self {
            gpr: [0u64; NUM_GPRS],
            pc: 0,
            guest_base: 0,
        }
    }
}

impl Default for RiscvCpu {
    fn default() -> Self {
        Self::new()
    }
}
