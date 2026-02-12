//! RISC-V CPU state for user-mode emulation.

/// Number of general-purpose registers (x0-x31).
pub const NUM_GPRS: usize = 32;
/// Number of floating-point registers (f0-f31).
pub const NUM_FPRS: usize = 32;

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
    /// Floating-point registers f0-f31 (raw bits).
    pub fpr: [u64; NUM_FPRS],
    /// Program counter.
    pub pc: u64,
    /// Guest memory base pointer (host address).
    /// Used by generated code to translate guest addresses.
    pub guest_base: u64,
    /// LR reservation address (-1 = no reservation).
    pub load_res: u64,
    /// LR loaded value (for SC comparison).
    pub load_val: u64,
    /// Floating-point accrued exception flags (fflags).
    pub fflags: u64,
    /// Floating-point rounding mode (frm).
    pub frm: u64,
    /// User status register (ustatus).
    pub ustatus: u64,
    /// User interrupt-enable register (uie).
    pub uie: u64,
    /// User trap vector base address (utvec).
    pub utvec: u64,
    /// User scratch register (uscratch).
    pub uscratch: u64,
    /// User exception program counter (uepc).
    pub uepc: u64,
    /// User exception cause (ucause).
    pub ucause: u64,
    /// User trap value (utval).
    pub utval: u64,
    /// User interrupt pending (uip).
    pub uip: u64,
}

// Field offsets (bytes) from the start of RiscvCpu.
// Used by `Context::new_global()` to bind IR temps.

/// Byte offset of `gpr[i]`: `i * 8`.
pub const fn gpr_offset(i: usize) -> i64 {
    (i * 8) as i64
}

/// Byte offset of `fpr[i]`: `NUM_GPRS*8 + i*8`.
pub const fn fpr_offset(i: usize) -> i64 {
    ((NUM_GPRS + i) * 8) as i64
}

/// Byte offset of the `pc` field.
pub const PC_OFFSET: i64 = ((NUM_GPRS + NUM_FPRS) * 8) as i64; // 512

/// Byte offset of the `guest_base` field.
pub const GUEST_BASE_OFFSET: i64 = PC_OFFSET + 8; // 520

/// Byte offset of the `load_res` field.
pub const LOAD_RES_OFFSET: i64 = GUEST_BASE_OFFSET + 8; // 528

/// Byte offset of the `load_val` field.
pub const LOAD_VAL_OFFSET: i64 = LOAD_RES_OFFSET + 8; // 536

/// Byte offset of `fflags`.
pub const FFLAGS_OFFSET: i64 = LOAD_VAL_OFFSET + 8; // 544
/// Byte offset of `frm`.
pub const FRM_OFFSET: i64 = FFLAGS_OFFSET + 8; // 552
/// Byte offset of `ustatus`.
pub const USTATUS_OFFSET: i64 = FRM_OFFSET + 8; // 560
/// Byte offset of `uie`.
pub const UIE_OFFSET: i64 = USTATUS_OFFSET + 8; // 568
/// Byte offset of `utvec`.
pub const UTVEC_OFFSET: i64 = UIE_OFFSET + 8; // 576
/// Byte offset of `uscratch`.
pub const USCRATCH_OFFSET: i64 = UTVEC_OFFSET + 8; // 584
/// Byte offset of `uepc`.
pub const UEPC_OFFSET: i64 = USCRATCH_OFFSET + 8; // 592
/// Byte offset of `ucause`.
pub const UCAUSE_OFFSET: i64 = UEPC_OFFSET + 8; // 600
/// Byte offset of `utval`.
pub const UTVAL_OFFSET: i64 = UCAUSE_OFFSET + 8; // 608
/// Byte offset of `uip`.
pub const UIP_OFFSET: i64 = UTVAL_OFFSET + 8; // 616

/// USTATUS FS bits mask.
pub const USTATUS_FS_MASK: u64 = 0x0000_6000;
/// USTATUS FS = Dirty.
pub const USTATUS_FS_DIRTY: u64 = 0x0000_6000;

impl RiscvCpu {
    pub fn new() -> Self {
        Self {
            gpr: [0u64; NUM_GPRS],
            fpr: [0u64; NUM_FPRS],
            pc: 0,
            guest_base: 0,
            load_res: u64::MAX,
            load_val: 0,
            fflags: 0,
            frm: 0,
            ustatus: USTATUS_FS_DIRTY,
            uie: 0,
            utvec: 0,
            uscratch: 0,
            uepc: 0,
            ucause: 0,
            utval: 0,
            uip: 0,
        }
    }
}

impl Default for RiscvCpu {
    fn default() -> Self {
        Self::new()
    }
}
