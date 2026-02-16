//! Binary IR serialization/deserialization (.tcgir format).
//!
//! Format (little-endian):
//!   HEADER: magic[4] + version[2] + flags[2] + nb_globals[4]
//!           + nb_labels[4] + tb_count[4]
//!   Per TB: STRING TABLE + TEMP SECTION + OP SECTION

use std::io::{self, Read, Write};

use crate::context::Context;
use crate::label::Label;
use crate::op::{Op, OpIdx, MAX_OP_ARGS};
use crate::opcode::Opcode;
use crate::temp::{Temp, TempIdx, TempKind};
use crate::types::Type;

const MAGIC: &[u8; 4] = b"TCIR";
const VERSION: u16 = 1;

// -- Write helpers --

fn write_u8(w: &mut impl Write, v: u8) -> io::Result<()> {
    w.write_all(&[v])
}

fn write_u16(w: &mut impl Write, v: u16) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn write_u32(w: &mut impl Write, v: u32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn write_u64(w: &mut impl Write, v: u64) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn write_i64(w: &mut impl Write, v: i64) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

// -- Read helpers --

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_i64(r: &mut impl Read) -> io::Result<i64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(i64::from_le_bytes(buf))
}

fn err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn u8_to_kind(v: u8) -> io::Result<TempKind> {
    match v {
        0 => Ok(TempKind::Ebb),
        1 => Ok(TempKind::Tb),
        2 => Ok(TempKind::Global),
        3 => Ok(TempKind::Fixed),
        4 => Ok(TempKind::Const),
        _ => Err(err("invalid TempKind")),
    }
}

fn u8_to_type(v: u8) -> io::Result<Type> {
    match v {
        0 => Ok(Type::I32),
        1 => Ok(Type::I64),
        2 => Ok(Type::I128),
        3 => Ok(Type::V64),
        4 => Ok(Type::V128),
        5 => Ok(Type::V256),
        _ => Err(err("invalid Type")),
    }
}

fn u8_to_opcode(v: u8) -> io::Result<Opcode> {
    if (v as usize) < Opcode::Count as usize {
        // SAFETY: Opcode is repr(u8) and v < Count.
        Ok(unsafe { std::mem::transmute::<u8, Opcode>(v) })
    } else {
        Err(err("invalid Opcode"))
    }
}

// -- String table --

struct StringTable {
    strings: Vec<String>,
    map: std::collections::HashMap<String, u32>,
}

impl StringTable {
    fn new() -> Self {
        Self {
            strings: Vec::new(),
            map: std::collections::HashMap::new(),
        }
    }

    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&idx) = self.map.get(s) {
            return idx;
        }
        let idx = self.strings.len() as u32;
        self.strings.push(s.to_owned());
        self.map.insert(s.to_owned(), idx);
        idx
    }

    fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        write_u32(w, self.strings.len() as u32)?;
        for s in &self.strings {
            let bytes = s.as_bytes();
            write_u16(w, bytes.len() as u16)?;
            w.write_all(bytes)?;
        }
        Ok(())
    }
}

fn read_string_table(r: &mut impl Read) -> io::Result<Vec<&'static str>> {
    let count = read_u32(r)? as usize;
    let mut table = Vec::with_capacity(count);
    for _ in 0..count {
        let len = read_u16(r)? as usize;
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf)?;
        let s = String::from_utf8(buf)
            .map_err(|e| err(&format!("invalid UTF-8: {e}")))?;
        // Leak to get &'static str — CLI tool, short-lived.
        let leaked: &'static str = Box::leak(s.into_boxed_str());
        table.push(leaked);
    }
    Ok(table)
}

/// Serialize a single TB's Context to binary .tcgir format.
pub fn serialize(ctx: &Context, w: &mut impl Write) -> io::Result<()> {
    // -- Header --
    w.write_all(MAGIC)?;
    write_u16(w, VERSION)?;
    write_u16(w, 0)?; // flags
    write_u32(w, ctx.nb_globals())?;
    write_u32(w, ctx.labels().len() as u32)?;
    write_u32(w, 1)?; // tb_count = 1

    // -- Build string table --
    let mut strtab = StringTable::new();
    let mut name_indices: Vec<u32> = Vec::with_capacity(ctx.temps().len());
    for t in ctx.temps() {
        if let Some(name) = t.name {
            name_indices.push(strtab.intern(name));
        } else {
            name_indices.push(0xFFFF_FFFF);
        }
    }
    strtab.write_to(w)?;

    // -- Temps --
    write_u32(w, ctx.temps().len() as u32)?;
    for (i, t) in ctx.temps().iter().enumerate() {
        write_u8(w, t.kind as u8)?;
        write_u8(w, t.ty as u8)?;
        write_u8(w, t.base_type as u8)?;
        write_u8(w, t.reg.unwrap_or(0xFF))?;
        write_u64(w, t.val)?;
        write_u32(w, t.mem_base.map_or(0xFFFF_FFFF, |b| b.0))?;
        write_i64(w, t.mem_offset)?;
        write_u32(w, name_indices[i])?;
    }

    // -- Ops --
    write_u32(w, ctx.ops().len() as u32)?;
    for op in ctx.ops() {
        write_u8(w, op.opc as u8)?;
        write_u8(w, op.op_type as u8)?;
        write_u8(w, op.param1)?;
        write_u8(w, op.param2)?;
        write_u8(w, op.nargs)?;
        w.write_all(&[0u8; 3])?; // padding
        for i in 0..op.nargs as usize {
            write_u32(w, op.args[i].0)?;
        }
    }

    Ok(())
}

/// Deserialize a .tcgir file into a Vec of Contexts (one per TB).
/// Handles concatenated .tcgir files (each with its own header).
pub fn deserialize(r: &mut impl Read) -> io::Result<Vec<Context>> {
    let mut contexts = Vec::new();
    loop {
        // Try to read magic; EOF here is normal termination.
        let mut magic = [0u8; 4];
        match r.read_exact(&mut magic) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(e),
        }
        if &magic != MAGIC {
            return Err(err("bad magic"));
        }
        let version = read_u16(r)?;
        if version != VERSION {
            return Err(err("unsupported version"));
        }
        let _flags = read_u16(r)?;
        let nb_globals = read_u32(r)?;
        let _nb_labels = read_u32(r)?;
        let tb_count = read_u32(r)? as usize;

        for _ in 0..tb_count {
            let ctx = deserialize_one_tb(r, nb_globals)?;
            contexts.push(ctx);
        }
    }
    Ok(contexts)
}

fn deserialize_one_tb(
    r: &mut impl Read,
    nb_globals: u32,
) -> io::Result<Context> {
    // -- String table --
    let strtab = read_string_table(r)?;

    // -- Temps --
    let temp_count = read_u32(r)? as usize;
    let mut temps = Vec::with_capacity(temp_count);
    for i in 0..temp_count {
        let kind = u8_to_kind(read_u8(r)?)?;
        let ty = u8_to_type(read_u8(r)?)?;
        let base_type = u8_to_type(read_u8(r)?)?;
        let reg_byte = read_u8(r)?;
        let reg = if reg_byte == 0xFF {
            None
        } else {
            Some(reg_byte)
        };
        let val = read_u64(r)?;
        let mem_base_raw = read_u32(r)?;
        let mem_base = if mem_base_raw == 0xFFFF_FFFF {
            None
        } else {
            Some(TempIdx(mem_base_raw))
        };
        let mem_offset = read_i64(r)?;
        let name_idx = read_u32(r)?;
        let name = if name_idx == 0xFFFF_FFFF {
            None
        } else {
            Some(strtab[name_idx as usize])
        };

        temps.push(Temp {
            idx: TempIdx(i as u32),
            ty,
            base_type,
            kind,
            val_type: match kind {
                TempKind::Const => crate::types::TempVal::Const,
                TempKind::Fixed => crate::types::TempVal::Reg,
                TempKind::Global => crate::types::TempVal::Mem,
                _ => crate::types::TempVal::Dead,
            },
            reg,
            mem_coherent: matches!(kind, TempKind::Global),
            mem_allocated: matches!(kind, TempKind::Global),
            val,
            mem_base,
            mem_offset,
            name,
        });
    }

    // -- Ops --
    let op_count = read_u32(r)? as usize;
    let mut ops = Vec::with_capacity(op_count);
    for i in 0..op_count {
        let opc = u8_to_opcode(read_u8(r)?)?;
        let op_type = u8_to_type(read_u8(r)?)?;
        let param1 = read_u8(r)?;
        let param2 = read_u8(r)?;
        let nargs = read_u8(r)?;
        let mut pad = [0u8; 3];
        r.read_exact(&mut pad)?;
        let mut args = [TempIdx(0); MAX_OP_ARGS];
        for slot in args.iter_mut().take(nargs as usize) {
            *slot = TempIdx(read_u32(r)?);
        }
        let mut op = Op::new(OpIdx(i as u32), opc, op_type);
        op.param1 = param1;
        op.param2 = param2;
        op.nargs = nargs;
        op.args = args;
        ops.push(op);
    }

    // -- Labels: create fresh labels based on ops --
    let mut labels = Vec::new();
    for op in &ops {
        if op.opc == Opcode::SetLabel {
            let id = op.args[0].0;
            while labels.len() <= id as usize {
                labels.push(Label::new(labels.len() as u32));
            }
        }
        if op.opc == Opcode::Br || op.opc == Opcode::BrCond {
            let def = op.opc.def();
            let label_pos =
                (def.nb_oargs + def.nb_iargs + def.nb_cargs - 1) as usize;
            let id = op.args[label_pos].0;
            while labels.len() <= id as usize {
                labels.push(Label::new(labels.len() as u32));
            }
        }
    }

    Ok(Context::from_raw_parts(temps, ops, labels, nb_globals))
}
