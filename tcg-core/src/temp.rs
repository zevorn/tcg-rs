use crate::types::{TempVal, Type};

/// Lifetime/scope of a TCG temporary.
///
/// Maps to QEMU's `TCGTempKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TempKind {
    /// Live within a single extended basic block.
    Ebb,
    /// Live across the entire translation block.
    Tb,
    /// Global — persists across TBs, backed by CPUState field.
    Global,
    /// Fixed to a specific host register.
    Fixed,
    /// Compile-time constant.
    Const,
}

/// Index into the Context's temp pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TempIdx(pub u32);

/// A TCG temporary variable.
///
/// Maps to QEMU's `TCGTemp`. Tracks both the IR-level properties (kind, type)
/// and the register allocator state (val_type, reg).
#[derive(Debug, Clone)]
pub struct Temp {
    pub idx: TempIdx,
    /// The actual operand type for this temp.
    pub ty: Type,
    /// Base type (may differ for sub-parts of multi-register values).
    pub base_type: Type,
    pub kind: TempKind,

    // -- Register allocator state --
    /// Where the value currently lives.
    pub val_type: TempVal,
    /// Allocated host register (valid when val_type == Reg).
    pub reg: Option<u8>,
    /// Whether the in-memory copy is up-to-date with the register.
    pub mem_coherent: bool,
    /// Whether a memory slot has been allocated for spilling.
    pub mem_allocated: bool,

    // -- Constant / memory info --
    /// For `Const` temps, the immediate value.
    pub val: u64,
    /// For `Global` temps, the base temp (env pointer) index.
    pub mem_base: Option<TempIdx>,
    /// For `Global` temps, the offset from mem_base into CPUState.
    pub mem_offset: i64,

    /// Debug name (e.g. "pc", "sp").
    pub name: Option<&'static str>,
}

impl Temp {
    pub fn new_ebb(idx: TempIdx, ty: Type) -> Self {
        Self {
            idx,
            ty,
            base_type: ty,
            kind: TempKind::Ebb,
            val_type: TempVal::Dead,
            reg: None,
            mem_coherent: false,
            mem_allocated: false,
            val: 0,
            mem_base: None,
            mem_offset: 0,
            name: None,
        }
    }

    pub fn new_tb(idx: TempIdx, ty: Type) -> Self {
        let mut t = Self::new_ebb(idx, ty);
        t.kind = TempKind::Tb;
        t
    }

    pub fn new_const(idx: TempIdx, ty: Type, val: u64) -> Self {
        Self {
            idx,
            ty,
            base_type: ty,
            kind: TempKind::Const,
            val_type: TempVal::Const,
            reg: None,
            mem_coherent: false,
            mem_allocated: false,
            val,
            mem_base: None,
            mem_offset: 0,
            name: None,
        }
    }

    pub fn new_global(
        idx: TempIdx,
        ty: Type,
        base: TempIdx,
        offset: i64,
        name: &'static str,
    ) -> Self {
        Self {
            idx,
            ty,
            base_type: ty,
            kind: TempKind::Global,
            val_type: TempVal::Mem,
            reg: None,
            mem_coherent: true,
            mem_allocated: true,
            val: 0,
            mem_base: Some(base),
            mem_offset: offset,
            name: Some(name),
        }
    }

    pub fn new_fixed(idx: TempIdx, ty: Type, reg: u8, name: &'static str) -> Self {
        Self {
            idx,
            ty,
            base_type: ty,
            kind: TempKind::Fixed,
            val_type: TempVal::Reg,
            reg: Some(reg),
            mem_coherent: false,
            mem_allocated: false,
            val: 0,
            mem_base: None,
            mem_offset: 0,
            name: Some(name),
        }
    }

    pub fn is_const(&self) -> bool {
        self.kind == TempKind::Const
    }

    pub fn is_global(&self) -> bool {
        self.kind == TempKind::Global
    }

    pub fn is_fixed(&self) -> bool {
        self.kind == TempKind::Fixed
    }

    /// Whether this temp needs to be saved back to memory at BB boundaries.
    pub fn is_global_or_fixed(&self) -> bool {
        matches!(self.kind, TempKind::Global | TempKind::Fixed)
    }
}
