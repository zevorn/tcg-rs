/// A branch target label within a translation block.
///
/// Maps to QEMU's `TCGLabel`. Labels support forward references:
/// branches can reference a label before it is placed, and the
/// code generator back-patches them when `set_label` is emitted.
#[derive(Debug, Clone)]
pub struct Label {
    pub id: u32,
    /// Whether this label has been placed (set_label emitted).
    pub present: bool,
    /// Whether the target address is known (resolved after codegen).
    pub has_value: bool,
    /// Resolved offset in the host code buffer.
    pub value: usize,
    /// Forward references that need back-patching when the label is resolved.
    pub uses: Vec<LabelUse>,
}

/// A forward reference to a label — records where a branch instruction
/// was emitted so it can be patched once the label's address is known.
#[derive(Debug, Clone, Copy)]
pub struct LabelUse {
    /// Offset in the code buffer where the branch was emitted.
    pub offset: usize,
    /// Type of relocation needed.
    pub kind: RelocKind,
}

/// Relocation types for label back-patching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocKind {
    /// x86-64 RIP-relative 32-bit displacement (at offset+1 from jmp/jcc opcode).
    Rel32,
}

impl Label {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            present: false,
            has_value: false,
            value: 0,
            uses: Vec::new(),
        }
    }

    /// Record a forward reference to this label.
    pub fn add_use(&mut self, offset: usize, kind: RelocKind) {
        self.uses.push(LabelUse { offset, kind });
    }

    /// Mark this label as placed at the given code buffer offset.
    pub fn set_value(&mut self, offset: usize) {
        self.present = true;
        self.has_value = true;
        self.value = offset;
    }

    /// Whether there are unresolved forward references.
    pub fn has_pending_uses(&self) -> bool {
        !self.uses.is_empty() && !self.has_value
    }
}
