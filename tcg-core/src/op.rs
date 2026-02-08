use crate::opcode::Opcode;
use crate::temp::TempIdx;
use crate::types::{RegSet, Type};

/// Maximum number of arguments per IR operation.
pub const MAX_OP_ARGS: usize = 10;

/// Index into the Context's op list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpIdx(pub u32);

/// Liveness data for an op's arguments — tracks which args are dead
/// after this op and which need to be synced to memory.
///
/// Maps to QEMU's `TCGLifeData`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LifeData(pub u32);

impl LifeData {
    pub const fn dead_arg(n: u32) -> u32 {
        1 << (n * 2)
    }

    pub const fn sync_arg(n: u32) -> u32 {
        1 << (n * 2 + 1)
    }

    pub fn is_dead(&self, n: u32) -> bool {
        self.0 & Self::dead_arg(n) != 0
    }

    pub fn is_sync(&self, n: u32) -> bool {
        self.0 & Self::sync_arg(n) != 0
    }

    pub fn set_dead(&mut self, n: u32) {
        self.0 |= Self::dead_arg(n);
    }

    pub fn set_sync(&mut self, n: u32) {
        self.0 |= Self::sync_arg(n);
    }
}

/// A single TCG IR operation.
///
/// Maps to QEMU's `TCGOp`. Each op has an opcode, a type (for polymorphic ops),
/// opcode-specific parameters, liveness info, and up to MAX_OP_ARGS arguments.
#[derive(Debug, Clone)]
pub struct Op {
    pub idx: OpIdx,
    pub opc: Opcode,
    /// Operand type for type-polymorphic ops (I32 or I64).
    pub op_type: Type,
    /// Opcode-specific parameter 1 (CALLI / TYPE / VECE).
    pub param1: u8,
    /// Opcode-specific parameter 2 (CALLO / FLAGS / VECE).
    pub param2: u8,
    /// Liveness analysis results.
    pub life: LifeData,
    /// Preferred output registers (hints for register allocator).
    pub output_pref: [RegSet; 2],
    /// Arguments: temp indices, label ids, or encoded immediates.
    pub args: [TempIdx; MAX_OP_ARGS],
    pub nargs: u8,
}

impl Op {
    pub fn new(idx: OpIdx, opc: Opcode, op_type: Type) -> Self {
        Self {
            idx,
            opc,
            op_type,
            param1: 0,
            param2: 0,
            life: LifeData::default(),
            output_pref: [RegSet::EMPTY; 2],
            args: [TempIdx(0); MAX_OP_ARGS],
            nargs: 0,
        }
    }

    pub fn with_args(idx: OpIdx, opc: Opcode, op_type: Type, args: &[TempIdx]) -> Self {
        let mut op = Self::new(idx, opc, op_type);
        let n = args.len().min(MAX_OP_ARGS);
        op.args[..n].copy_from_slice(&args[..n]);
        op.nargs = n as u8;
        op
    }

    /// Get the output arguments slice (based on opcode definition).
    pub fn oargs(&self) -> &[TempIdx] {
        let n = self.opc.def().nb_oargs as usize;
        &self.args[..n]
    }

    /// Get the input arguments slice.
    pub fn iargs(&self) -> &[TempIdx] {
        let def = self.opc.def();
        let start = def.nb_oargs as usize;
        let end = start + def.nb_iargs as usize;
        &self.args[start..end]
    }

    /// Get the constant arguments slice.
    pub fn cargs(&self) -> &[TempIdx] {
        let def = self.opc.def();
        let start = (def.nb_oargs + def.nb_iargs) as usize;
        let end = start + def.nb_cargs as usize;
        &self.args[start..end]
    }
}
