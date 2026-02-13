//! IR dump — human-readable text output for TCG ops.
//!
//! Mirrors QEMU's `tcg_dump_ops()` in `tcg/tcg.c`.

use std::io::Write;

use crate::context::Context;
use crate::op::Op;
use crate::opcode::Opcode;
use crate::temp::TempKind;
use crate::types::Type;

/// Format a condition code as a short name.
fn cond_name(c: u32) -> &'static str {
    match c {
        0 => "never",
        1 => "always",
        8 => "eq",
        9 => "ne",
        10 => "lt",
        11 => "ge",
        12 => "le",
        13 => "gt",
        14 => "ltu",
        15 => "geu",
        16 => "leu",
        17 => "gtu",
        18 => "tsteq",
        19 => "tstne",
        _ => "???",
    }
}

/// Format a temp reference for display.
fn fmt_temp(ctx: &Context, idx: crate::temp::TempIdx, buf: &mut String) {
    use std::fmt::Write as FmtWrite;
    let i = idx.0 as usize;
    if i >= ctx.nb_temps() as usize {
        let v = idx.0;
        write!(buf, "$0x{v:x}").unwrap();
        return;
    }
    let t = ctx.temp(idx);
    match t.kind {
        TempKind::Const => {
            let v = t.val;
            write!(buf, "$0x{v:x}").unwrap();
        }
        TempKind::Global => {
            if let Some(name) = t.name {
                buf.push_str(name);
            } else {
                write!(buf, "g{i}").unwrap();
            }
        }
        TempKind::Fixed => {
            if let Some(name) = t.name {
                buf.push_str(name);
            } else {
                let r = t.reg.unwrap_or(0);
                write!(buf, "fixed({r})").unwrap();
            }
        }
        TempKind::Ebb | TempKind::Tb => {
            let local = i as u32 - ctx.nb_globals();
            write!(buf, "tmp{local}").unwrap();
        }
    }
}

/// Build the opcode name with type suffix for polymorphic ops.
fn op_name(op: &Op) -> String {
    let def = op.opc.def();
    if op.opc.is_int_polymorphic() {
        let suffix = match op.op_type {
            Type::I32 => "_i32",
            Type::I64 => "_i64",
            _ => "",
        };
        let base = def.name;
        format!("{base}{suffix}")
    } else {
        def.name.to_string()
    }
}

/// Dump all IR ops in `ctx` to the given writer.
///
/// Output format mirrors QEMU's `tcg_dump_ops()`.
pub fn dump_ops(ctx: &Context, w: &mut impl Write) -> std::io::Result<()> {
    dump_ops_with(ctx, w, |_, _| Ok(()))
}

/// Dump IR ops with an annotation callback for `InsnStart`.
///
/// `insn_anno` is called at each guest instruction boundary with
/// `(pc, writer)` — use it to print source instruction bytes or
/// disassembly on the `---- 0x...` header line.
pub fn dump_ops_with(
    ctx: &Context,
    w: &mut impl Write,
    insn_anno: impl Fn(u64, &mut dyn Write) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut buf = String::with_capacity(128);

    for op in ctx.ops() {
        buf.clear();
        match op.opc {
            Opcode::InsnStart => {
                let cargs = op.cargs();
                let lo = cargs[0].0 as u64;
                let hi = cargs[1].0 as u64;
                let pc = (hi << 32) | lo;
                write!(w, " ---- 0x{pc:016x}")?;
                insn_anno(pc, w)?;
                writeln!(w)?;
                writeln!(w, " insn_start $0x{pc:x}")?;
                continue;
            }
            Opcode::SetLabel => {
                let label_id = op.cargs()[0].0;
                writeln!(w, " L{label_id}:")?;
                continue;
            }
            _ => {}
        }

        // Generic op formatting
        let name = op_name(op);
        write!(w, " {name}")?;

        // Output args
        let oargs = op.oargs();
        for (i, &a) in oargs.iter().enumerate() {
            if i > 0 {
                write!(w, ",")?;
            }
            write!(w, " ")?;
            buf.clear();
            fmt_temp(ctx, a, &mut buf);
            write!(w, "{buf}")?;
        }

        // Input args
        let iargs = op.iargs();
        let has_oargs = !oargs.is_empty();
        for (i, &a) in iargs.iter().enumerate() {
            if has_oargs || i > 0 {
                write!(w, ",")?;
            }
            write!(w, " ")?;
            buf.clear();
            fmt_temp(ctx, a, &mut buf);
            write!(w, "{buf}")?;
        }

        // Constant args — special handling per opcode
        let cargs = op.cargs();
        match op.opc {
            Opcode::BrCond => {
                let cond = cond_name(cargs[0].0);
                let label = cargs[1].0;
                write!(w, ", {cond}, L{label}")?;
            }
            Opcode::SetCond
            | Opcode::NegSetCond
            | Opcode::MovCond
            | Opcode::CmpVec
            | Opcode::CmpselVec => {
                let cond = cond_name(cargs[0].0);
                write!(w, ", {cond}")?;
            }
            Opcode::Br => {
                let label = cargs[0].0;
                write!(w, " L{label}")?;
            }
            Opcode::Call => {
                let lo = cargs[0].0 as u64;
                let hi = cargs[1].0 as u64;
                let addr = (hi << 32) | lo;
                write!(w, ", $0x{addr:x}")?;
            }
            _ => {
                let has_prev = !oargs.is_empty() || !iargs.is_empty();
                for (i, &c) in cargs.iter().enumerate() {
                    if has_prev || i > 0 {
                        write!(w, ",")?;
                    }
                    let v = c.0;
                    write!(w, " $0x{v:x}")?;
                }
            }
        }

        writeln!(w)?;
    }
    Ok(())
}
