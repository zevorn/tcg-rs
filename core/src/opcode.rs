use crate::types::Type;

/// TCG IR opcodes — unified (type-polymorphic for integer ops).
///
/// Maps to QEMU's `TCGOpcode` defined via DEF() macros in `tcg-opc.h`.
/// Integer ops (marked with `OPF_INT`) work on both I32 and I64;
/// the actual type is carried in `Op::op_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Opcode {
    // -- Data movement --
    Mov = 0,
    SetCond,
    NegSetCond,
    MovCond,

    // -- Arithmetic --
    Add,
    Sub,
    Mul,
    Neg,
    DivS,
    DivU,
    RemS,
    RemU,
    DivS2, // signed double-width division
    DivU2, // unsigned double-width division

    // -- Widening multiply --
    MulSH, // signed multiply high
    MulUH, // unsigned multiply high
    MulS2, // signed multiply -> double width
    MulU2, // unsigned multiply -> double width

    // -- Carry arithmetic --
    AddCO, // add with carry out
    AddCI, // add with carry in
    AddCIO,
    AddC1O,
    SubBO, // sub with borrow out
    SubBI, // sub with borrow in
    SubBIO,
    SubB1O,

    // -- Logic --
    And,
    Or,
    Xor,
    Not,
    AndC, // a & ~b
    OrC,  // a | ~b
    Eqv,  // ~(a ^ b)
    Nand,
    Nor,

    // -- Shift/rotate --
    Shl,
    Shr,
    Sar,
    RotL,
    RotR,

    // -- Bit field --
    Extract,  // unsigned bit-field extract
    SExtract, // signed bit-field extract
    Deposit,  // bit-field deposit
    Extract2, // extract from concatenation of two regs

    // -- Byte swap --
    Bswap16,
    Bswap32,
    Bswap64,

    // -- Bit counting --
    Clz,   // count leading zeros
    Ctz,   // count trailing zeros
    CtPop, // population count

    // -- 32-bit host: 64-bit ops on paired regs --
    BrCond2I32,  // 64-bit conditional branch (32-bit host)
    SetCond2I32, // 64-bit setcond (32-bit host)

    // -- Type conversion --
    ExtI32I64,   // sign-extend i32 -> i64
    ExtUI32I64,  // zero-extend i32 -> i64
    ExtrlI64I32, // truncate i64 -> i32 (low)
    ExtrhI64I32, // extract i64 -> i32 (high)

    // -- Host memory load/store (direct, for accessing CPUState fields) --
    Ld8U,
    Ld8S,
    Ld16U,
    Ld16S,
    Ld32U,
    Ld32S,
    Ld, // native-width load
    St8,
    St16,
    St32,
    St, // native-width store

    // -- Guest memory access (through software TLB) --
    QemuLd,
    QemuSt,
    QemuLd2, // 128-bit guest load (two regs)
    QemuSt2, // 128-bit guest store (two regs)

    // -- Control flow --
    Br,       // unconditional branch to label
    BrCond,   // conditional branch
    SetLabel, // define label position
    GotoTb,   // direct jump to another TB (patchable)
    ExitTb,   // return from TB to execution loop
    GotoPtr,  // indirect jump through register
    Mb,       // memory barrier

    // -- Call --
    Call,

    // -- Plugin --
    PluginCb,
    PluginMemCb,

    // -- Misc --
    Nop,
    Discard,
    InsnStart, // marks guest instruction boundary

    // -- Vector data movement --
    MovVec,
    DupVec,  // duplicate scalar to all lanes
    Dup2Vec, // duplicate two i32 to vector
    LdVec,   // vector load
    StVec,   // vector store
    DupmVec, // duplicate from memory to vector

    // -- Vector arithmetic --
    AddVec,
    SubVec,
    MulVec,
    NegVec,
    AbsVec,
    SsaddVec, // signed saturating add
    UsaddVec, // unsigned saturating add
    SssubVec, // signed saturating sub
    UssubVec, // unsigned saturating sub
    SminVec,
    UminVec,
    SmaxVec,
    UmaxVec,

    // -- Vector logic --
    AndVec,
    OrVec,
    XorVec,
    AndcVec,
    OrcVec,
    NandVec,
    NorVec,
    EqvVec,
    NotVec,

    // -- Vector shift by immediate --
    ShliVec,
    ShriVec,
    SariVec,
    RotliVec,

    // -- Vector shift by scalar --
    ShlsVec,
    ShrsVec,
    SarsVec,
    RotlsVec,

    // -- Vector shift by vector --
    ShlvVec,
    ShrvVec,
    SarvVec,
    RotlvVec,
    RotrvVec,

    // -- Vector compare/select --
    CmpVec,
    BitselVec, // bitwise select
    CmpselVec, // compare and select

    // Sentinel — must be last
    Count,
}

/// Flags describing properties of an opcode.
///
/// Maps to QEMU's `TCG_OPF_*` flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpFlags(u16);

impl OpFlags {
    pub const NONE: OpFlags = OpFlags(0);
    /// Exits the translation block.
    pub const BB_EXIT: OpFlags = OpFlags(0x01);
    /// Ends a basic block (next op starts a new BB).
    pub const BB_END: OpFlags = OpFlags(0x02);
    /// Clobbers caller-saved registers (like a function call).
    pub const CALL_CLOBBER: OpFlags = OpFlags(0x04);
    /// Has side effects — cannot be eliminated by DCE.
    pub const SIDE_EFFECTS: OpFlags = OpFlags(0x08);
    /// Operands may be I32 or I64 (type-polymorphic).
    pub const INT: OpFlags = OpFlags(0x10);
    /// Not directly emitted to host code (lowered earlier).
    pub const NOT_PRESENT: OpFlags = OpFlags(0x20);
    /// Vector operation.
    pub const VECTOR: OpFlags = OpFlags(0x40);
    /// Conditional branch (may or may not be taken).
    pub const COND_BRANCH: OpFlags = OpFlags(0x80);
    /// Produces carry/borrow output.
    pub const CARRY_OUT: OpFlags = OpFlags(0x100);
    /// Consumes carry/borrow input.
    pub const CARRY_IN: OpFlags = OpFlags(0x200);

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, other: OpFlags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: OpFlags) -> Self {
        Self(self.0 | other.0)
    }
}

/// Static definition of an opcode — argument counts and flags.
///
/// Maps to QEMU's `TCGOpDef`.
#[derive(Debug, Clone, Copy)]
pub struct OpDef {
    pub name: &'static str,
    pub nb_oargs: u8,
    pub nb_iargs: u8,
    pub nb_cargs: u8,
    pub flags: OpFlags,
}

impl OpDef {
    pub const fn nb_args(&self) -> u8 {
        self.nb_oargs + self.nb_iargs + self.nb_cargs
    }
}

// Helper to combine flags in const context.
const fn f(a: OpFlags, b: OpFlags) -> OpFlags {
    OpFlags(a.0 | b.0)
}

const INT: OpFlags = OpFlags::INT;
const NP: OpFlags = OpFlags::NOT_PRESENT;
const SE: OpFlags = OpFlags::SIDE_EFFECTS;
const CC: OpFlags = OpFlags::CALL_CLOBBER;
const BE: OpFlags = OpFlags::BB_END;
const BX: OpFlags = OpFlags::BB_EXIT;
const CB: OpFlags = OpFlags::COND_BRANCH;
const CO: OpFlags = OpFlags::CARRY_OUT;
const CI: OpFlags = OpFlags::CARRY_IN;
const VC: OpFlags = OpFlags::VECTOR;
const N: OpFlags = OpFlags::NONE;

/// Static opcode definition table, indexed by `Opcode as usize`.
pub static OPCODE_DEFS: [OpDef; Opcode::Count as usize] = [
    // Mov
    OpDef {
        name: "mov",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: f(INT, NP),
    },
    // SetCond
    OpDef {
        name: "setcond",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: INT,
    },
    // NegSetCond
    OpDef {
        name: "negsetcond",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: INT,
    },
    // MovCond
    OpDef {
        name: "movcond",
        nb_oargs: 1,
        nb_iargs: 4,
        nb_cargs: 1,
        flags: INT,
    },
    // Add
    OpDef {
        name: "add",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Sub
    OpDef {
        name: "sub",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Mul
    OpDef {
        name: "mul",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Neg
    OpDef {
        name: "neg",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: INT,
    },
    // DivS
    OpDef {
        name: "divs",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // DivU
    OpDef {
        name: "divu",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // RemS
    OpDef {
        name: "rems",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // RemU
    OpDef {
        name: "remu",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // DivS2
    OpDef {
        name: "divs2",
        nb_oargs: 2,
        nb_iargs: 3,
        nb_cargs: 0,
        flags: INT,
    },
    // DivU2
    OpDef {
        name: "divu2",
        nb_oargs: 2,
        nb_iargs: 3,
        nb_cargs: 0,
        flags: INT,
    },
    // MulSH
    OpDef {
        name: "mulsh",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // MulUH
    OpDef {
        name: "muluh",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // MulS2
    OpDef {
        name: "muls2",
        nb_oargs: 2,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // MulU2
    OpDef {
        name: "mulu2",
        nb_oargs: 2,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // AddCO
    OpDef {
        name: "addco",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: f(INT, CO),
    },
    // AddCI
    OpDef {
        name: "addci",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: f(INT, CI),
    },
    // AddCIO
    OpDef {
        name: "addcio",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: OpFlags(INT.0 | CI.0 | CO.0),
    },
    // AddC1O
    OpDef {
        name: "addc1o",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: f(INT, CO),
    },
    // SubBO
    OpDef {
        name: "subbo",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: f(INT, CO),
    },
    // SubBI
    OpDef {
        name: "subbi",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: f(INT, CI),
    },
    // SubBIO
    OpDef {
        name: "subbio",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: OpFlags(INT.0 | CI.0 | CO.0),
    },
    // SubB1O
    OpDef {
        name: "subb1o",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: f(INT, CO),
    },
    // And
    OpDef {
        name: "and",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Or
    OpDef {
        name: "or",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Xor
    OpDef {
        name: "xor",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Not
    OpDef {
        name: "not",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: INT,
    },
    // AndC
    OpDef {
        name: "andc",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // OrC
    OpDef {
        name: "orc",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Eqv
    OpDef {
        name: "eqv",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Nand
    OpDef {
        name: "nand",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Nor
    OpDef {
        name: "nor",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Shl
    OpDef {
        name: "shl",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Shr
    OpDef {
        name: "shr",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Sar
    OpDef {
        name: "sar",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // RotL
    OpDef {
        name: "rotl",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // RotR
    OpDef {
        name: "rotr",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Extract
    OpDef {
        name: "extract",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 2,
        flags: INT,
    },
    // SExtract
    OpDef {
        name: "sextract",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 2,
        flags: INT,
    },
    // Deposit
    OpDef {
        name: "deposit",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 2,
        flags: INT,
    },
    // Extract2
    OpDef {
        name: "extract2",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: INT,
    },
    // Bswap16
    OpDef {
        name: "bswap16",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // Bswap32
    OpDef {
        name: "bswap32",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // Bswap64
    OpDef {
        name: "bswap64",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // Clz
    OpDef {
        name: "clz",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // Ctz
    OpDef {
        name: "ctz",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: INT,
    },
    // CtPop
    OpDef {
        name: "ctpop",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: INT,
    },
    // BrCond2I32
    OpDef {
        name: "brcond2_i32",
        nb_oargs: 0,
        nb_iargs: 4,
        nb_cargs: 2,
        flags: OpFlags(BE.0 | CB.0),
    },
    // SetCond2I32
    OpDef {
        name: "setcond2_i32",
        nb_oargs: 1,
        nb_iargs: 4,
        nb_cargs: 1,
        flags: N,
    },
    // ExtI32I64
    OpDef {
        name: "ext_i32_i64",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: N,
    },
    // ExtUI32I64
    OpDef {
        name: "extu_i32_i64",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: N,
    },
    // ExtrlI64I32
    OpDef {
        name: "extrl_i64_i32",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: N,
    },
    // ExtrhI64I32
    OpDef {
        name: "extrh_i64_i32",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: N,
    },
    // Ld8U
    OpDef {
        name: "ld8u",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // Ld8S
    OpDef {
        name: "ld8s",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // Ld16U
    OpDef {
        name: "ld16u",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // Ld16S
    OpDef {
        name: "ld16s",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // Ld32U
    OpDef {
        name: "ld32u",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // Ld32S
    OpDef {
        name: "ld32s",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // Ld
    OpDef {
        name: "ld",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: INT,
    },
    // St8
    OpDef {
        name: "st8",
        nb_oargs: 0,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: INT,
    },
    // St16
    OpDef {
        name: "st16",
        nb_oargs: 0,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: INT,
    },
    // St32
    OpDef {
        name: "st32",
        nb_oargs: 0,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: INT,
    },
    // St
    OpDef {
        name: "st",
        nb_oargs: 0,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: INT,
    },
    // QemuLd
    OpDef {
        name: "qemu_ld",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: OpFlags(CC.0 | SE.0 | INT.0),
    },
    // QemuSt
    OpDef {
        name: "qemu_st",
        nb_oargs: 0,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: OpFlags(CC.0 | SE.0 | INT.0),
    },
    // QemuLd2
    OpDef {
        name: "qemu_ld2",
        nb_oargs: 2,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: OpFlags(CC.0 | SE.0 | INT.0),
    },
    // QemuSt2
    OpDef {
        name: "qemu_st2",
        nb_oargs: 0,
        nb_iargs: 3,
        nb_cargs: 1,
        flags: OpFlags(CC.0 | SE.0 | INT.0),
    },
    // Br
    OpDef {
        name: "br",
        nb_oargs: 0,
        nb_iargs: 0,
        nb_cargs: 1,
        flags: f(BE, NP),
    },
    // BrCond
    OpDef {
        name: "brcond",
        nb_oargs: 0,
        nb_iargs: 2,
        nb_cargs: 2,
        flags: OpFlags(BE.0 | CB.0 | INT.0),
    },
    // SetLabel
    OpDef {
        name: "set_label",
        nb_oargs: 0,
        nb_iargs: 0,
        nb_cargs: 1,
        flags: f(BE, NP),
    },
    // GotoTb
    OpDef {
        name: "goto_tb",
        nb_oargs: 0,
        nb_iargs: 0,
        nb_cargs: 1,
        flags: OpFlags(BX.0 | BE.0 | NP.0),
    },
    // ExitTb
    OpDef {
        name: "exit_tb",
        nb_oargs: 0,
        nb_iargs: 0,
        nb_cargs: 1,
        flags: OpFlags(BX.0 | BE.0 | NP.0),
    },
    // GotoPtr
    OpDef {
        name: "goto_ptr",
        nb_oargs: 0,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: f(BX, BE),
    },
    // Mb
    OpDef {
        name: "mb",
        nb_oargs: 0,
        nb_iargs: 0,
        nb_cargs: 1,
        flags: NP,
    },
    // Call
    OpDef {
        name: "call",
        nb_oargs: 1,
        nb_iargs: 6,
        nb_cargs: 2,
        flags: f(CC, NP),
    },
    // PluginCb
    OpDef {
        name: "plugin_cb",
        nb_oargs: 0,
        nb_iargs: 0,
        nb_cargs: 1,
        flags: NP,
    },
    // PluginMemCb
    OpDef {
        name: "plugin_mem_cb",
        nb_oargs: 0,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: NP,
    },
    // Nop
    OpDef {
        name: "nop",
        nb_oargs: 0,
        nb_iargs: 0,
        nb_cargs: 0,
        flags: NP,
    },
    // Discard
    OpDef {
        name: "discard",
        nb_oargs: 1,
        nb_iargs: 0,
        nb_cargs: 0,
        flags: NP,
    },
    // InsnStart
    OpDef {
        name: "insn_start",
        nb_oargs: 0,
        nb_iargs: 0,
        nb_cargs: 2,
        flags: NP,
    },
    // -- Vector ops --
    // MovVec
    OpDef {
        name: "mov_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: f(VC, NP),
    },
    // DupVec
    OpDef {
        name: "dup_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: VC,
    },
    // Dup2Vec
    OpDef {
        name: "dup2_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // LdVec
    OpDef {
        name: "ld_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: VC,
    },
    // StVec
    OpDef {
        name: "st_vec",
        nb_oargs: 0,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: VC,
    },
    // DupmVec
    OpDef {
        name: "dupm_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: VC,
    },
    // AddVec
    OpDef {
        name: "add_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // SubVec
    OpDef {
        name: "sub_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // MulVec
    OpDef {
        name: "mul_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // NegVec
    OpDef {
        name: "neg_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: VC,
    },
    // AbsVec
    OpDef {
        name: "abs_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: VC,
    },
    // SsaddVec
    OpDef {
        name: "ssadd_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // UsaddVec
    OpDef {
        name: "usadd_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // SssubVec
    OpDef {
        name: "sssub_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // UssubVec
    OpDef {
        name: "ussub_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // SminVec
    OpDef {
        name: "smin_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // UminVec
    OpDef {
        name: "umin_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // SmaxVec
    OpDef {
        name: "smax_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // UmaxVec
    OpDef {
        name: "umax_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // AndVec
    OpDef {
        name: "and_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // OrVec
    OpDef {
        name: "or_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // XorVec
    OpDef {
        name: "xor_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // AndcVec
    OpDef {
        name: "andc_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // OrcVec
    OpDef {
        name: "orc_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // NandVec
    OpDef {
        name: "nand_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // NorVec
    OpDef {
        name: "nor_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // EqvVec
    OpDef {
        name: "eqv_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // NotVec
    OpDef {
        name: "not_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 0,
        flags: VC,
    },
    // ShliVec
    OpDef {
        name: "shli_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: VC,
    },
    // ShriVec
    OpDef {
        name: "shri_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: VC,
    },
    // SariVec
    OpDef {
        name: "sari_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: VC,
    },
    // RotliVec
    OpDef {
        name: "rotli_vec",
        nb_oargs: 1,
        nb_iargs: 1,
        nb_cargs: 1,
        flags: VC,
    },
    // ShlsVec
    OpDef {
        name: "shls_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // ShrsVec
    OpDef {
        name: "shrs_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // SarsVec
    OpDef {
        name: "sars_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // RotlsVec
    OpDef {
        name: "rotls_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // ShlvVec
    OpDef {
        name: "shlv_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // ShrvVec
    OpDef {
        name: "shrv_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // SarvVec
    OpDef {
        name: "sarv_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // RotlvVec
    OpDef {
        name: "rotlv_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // RotrvVec
    OpDef {
        name: "rotrv_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 0,
        flags: VC,
    },
    // CmpVec
    OpDef {
        name: "cmp_vec",
        nb_oargs: 1,
        nb_iargs: 2,
        nb_cargs: 1,
        flags: VC,
    },
    // BitselVec
    OpDef {
        name: "bitsel_vec",
        nb_oargs: 1,
        nb_iargs: 3,
        nb_cargs: 0,
        flags: VC,
    },
    // CmpselVec
    OpDef {
        name: "cmpsel_vec",
        nb_oargs: 1,
        nb_iargs: 4,
        nb_cargs: 1,
        flags: VC,
    },
];

impl Opcode {
    /// Look up the static definition for this opcode.
    pub fn def(self) -> &'static OpDef {
        &OPCODE_DEFS[self as usize]
    }

    /// Return the fixed IR type this opcode operates on, if not type-polymorphic.
    pub fn fixed_type(self) -> Option<Type> {
        match self {
            Opcode::ExtI32I64 | Opcode::ExtUI32I64 => Some(Type::I64),
            Opcode::ExtrlI64I32
            | Opcode::ExtrhI64I32
            | Opcode::BrCond2I32
            | Opcode::SetCond2I32 => Some(Type::I32),
            _ => None,
        }
    }

    /// Whether this opcode is type-polymorphic (works on I32 or I64).
    pub fn is_int_polymorphic(self) -> bool {
        self.def().flags.contains(OpFlags::INT)
    }

    /// Whether this is a vector operation.
    pub fn is_vector(self) -> bool {
        self.def().flags.contains(OpFlags::VECTOR)
    }
}
