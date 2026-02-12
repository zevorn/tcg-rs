use std::collections::BTreeMap;
use std::io::Write;

// ── Data structures ─────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct FieldSegment {
    pub pos: u32,
    pub len: u32,
    pub signed: bool,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub segments: Vec<FieldSegment>,
    pub func: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ArgSet {
    pub name: String,
    pub fields: Vec<String>,
    pub is_extern: bool,
}

#[derive(Clone, Debug)]
pub enum FieldMapping {
    FieldRef(String),
    Inline { pos: u32, len: u32, signed: bool },
    Const(i32),
}

#[derive(Clone, Debug)]
pub struct Format {
    #[allow(dead_code)]
    pub name: String,
    pub fixedbits: u32,
    pub fixedmask: u32,
    pub args_name: String,
    pub field_map: BTreeMap<String, FieldMapping>,
}

#[derive(Clone, Debug)]
pub struct Pattern {
    pub name: String,
    pub fixedbits: u32,
    pub fixedmask: u32,
    pub args_name: String,
    pub field_map: BTreeMap<String, FieldMapping>,
}

pub struct Parsed {
    pub fields: BTreeMap<String, Field>,
    pub argsets: BTreeMap<String, ArgSet>,
    pub patterns: Vec<Pattern>,
}

// ── Bit-pattern parsing ─────────────────────────────────────────

pub fn is_bit_char(c: char) -> bool {
    matches!(c, '0' | '1' | '.' | '-')
}

pub fn is_bit_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_bit_char)
}

pub fn is_inline_field(s: &str) -> bool {
    if let Some(idx) = s.find(':') {
        let name = &s[..idx];
        let rest = &s[idx + 1..];
        !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !rest.is_empty()
            && rest.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

pub struct BitPatternResult {
    pub fixedbits: u32,
    pub fixedmask: u32,
    pub inline_fields: BTreeMap<String, (u32, u32)>,
}

pub fn parse_bit_tokens(
    tokens: &[&str],
    width: u32,
) -> Result<BitPatternResult, String> {
    let mut fixedbits: u32 = 0;
    let mut fixedmask: u32 = 0;
    let mut inline_fields = BTreeMap::new();
    let mut bit_pos: i32 = width as i32 - 1;

    for &tok in tokens {
        if is_bit_token(tok) {
            for c in tok.chars() {
                if bit_pos < 0 {
                    return Err(format!("bit pattern exceeds {width} bits"));
                }
                match c {
                    '1' => {
                        fixedbits |= 1 << bit_pos;
                        fixedmask |= 1 << bit_pos;
                    }
                    '0' => {
                        fixedmask |= 1 << bit_pos;
                    }
                    '.' | '-' => {}
                    _ => unreachable!(),
                }
                bit_pos -= 1;
            }
        } else if is_inline_field(tok) {
            let idx = tok.find(':').unwrap();
            let name = &tok[..idx];
            let len: u32 = tok[idx + 1..]
                .parse()
                .map_err(|e| format!("bad inline field len: {e}"))?;
            let pos = (bit_pos - len as i32 + 1) as u32;
            inline_fields.insert(name.to_string(), (pos, len));
            bit_pos -= len as i32;
        } else {
            break;
        }
    }
    Ok(BitPatternResult {
        fixedbits,
        fixedmask,
        inline_fields,
    })
}

pub fn count_bit_tokens(tokens: &[&str]) -> usize {
    tokens
        .iter()
        .take_while(|t| is_bit_token(t) || is_inline_field(t))
        .count()
}

// ── Field segment parsing ──────────────────────────────────────

pub fn parse_field_segment(s: &str) -> Result<FieldSegment, String> {
    let (pos_str, rest) = s
        .split_once(':')
        .ok_or_else(|| format!("bad segment: {s}"))?;
    let signed = rest.starts_with('s');
    let len_str = if signed { &rest[1..] } else { rest };
    let pos: u32 =
        pos_str.parse().map_err(|_| format!("bad pos: {pos_str}"))?;
    let len: u32 =
        len_str.parse().map_err(|_| format!("bad len: {len_str}"))?;
    Ok(FieldSegment { pos, len, signed })
}

pub fn parse_field(line: &str) -> Result<Field, String> {
    // %name seg1 seg2 ... [!function=func]
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let name = tokens[0][1..].to_string(); // skip %
    let mut segments = Vec::new();
    let mut func = None;
    for &tok in &tokens[1..] {
        if let Some(f) = tok.strip_prefix("!function=") {
            func = Some(f.to_string());
        } else {
            segments.push(parse_field_segment(tok)?);
        }
    }
    Ok(Field {
        name,
        segments,
        func,
    })
}

pub fn parse_argset(line: &str) -> Result<ArgSet, String> {
    // &name field1 field2 ... [!extern]
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let name = tokens[0][1..].to_string(); // skip &
    let is_extern = tokens.last() == Some(&"!extern");
    let end = if is_extern {
        tokens.len() - 1
    } else {
        tokens.len()
    };
    let fields = tokens[1..end].iter().map(|s| s.to_string()).collect();
    Ok(ArgSet {
        name,
        fields,
        is_extern,
    })
}

/// Parse trailing attributes after bit tokens.
/// Returns (args_name, field_map).
fn parse_attrs(
    tokens: &[&str],
    fields: &BTreeMap<String, Field>,
) -> Result<(String, BTreeMap<String, FieldMapping>), String> {
    let mut args_name = String::new();
    let mut field_map = BTreeMap::new();
    for &tok in tokens {
        if let Some(a) = tok.strip_prefix('&') {
            args_name = a.to_string();
        } else if let Some(f) = tok.strip_prefix('%') {
            // %field_ref → field_name = FieldRef(field_name)
            field_map
                .insert(f.to_string(), FieldMapping::FieldRef(f.to_string()));
        } else if let Some(idx) = tok.find('=') {
            let key = &tok[..idx];
            let val = &tok[idx + 1..];
            if let Some(fref) = val.strip_prefix('%') {
                field_map.insert(
                    key.to_string(),
                    FieldMapping::FieldRef(fref.to_string()),
                );
            } else if let Ok(c) = val.parse::<i32>() {
                field_map.insert(key.to_string(), FieldMapping::Const(c));
            } else {
                return Err(format!("bad attr: {tok}"));
            }
        } else if tok.starts_with('!') {
            // !function= etc, skip (handled in field)
        } else if !tok.starts_with('@') {
            // Unknown token in attrs
            if fields.contains_key(tok) {
                field_map.insert(
                    tok.to_string(),
                    FieldMapping::FieldRef(tok.to_string()),
                );
            }
        }
    }
    Ok((args_name, field_map))
}

fn parse_format(
    line: &str,
    fields: &BTreeMap<String, Field>,
    width: u32,
) -> Result<(String, Format), String> {
    // @name bit_tokens... &argset [mappings...]
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let name = tokens[0][1..].to_string(); // skip @
    let bit_count = count_bit_tokens(&tokens[1..]);
    let bp = parse_bit_tokens(&tokens[1..1 + bit_count], width)?;
    let rest = &tokens[1 + bit_count..];
    let (args_name, mut field_map) = parse_attrs(rest, fields)?;
    // Merge inline fields from bit pattern
    for (fname, (pos, len)) in &bp.inline_fields {
        field_map
            .entry(fname.clone())
            .or_insert(FieldMapping::Inline {
                pos: *pos,
                len: *len,
                signed: false,
            });
    }
    Ok((
        name.clone(),
        Format {
            name,
            fixedbits: bp.fixedbits,
            fixedmask: bp.fixedmask,
            args_name,
            field_map,
        },
    ))
}

fn parse_pattern(
    line: &str,
    formats: &BTreeMap<String, Format>,
    fields: &BTreeMap<String, Field>,
    auto_args: &mut BTreeMap<String, ArgSet>,
    width: u32,
) -> Result<Pattern, String> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let name = tokens[0].to_string();
    let bit_count = count_bit_tokens(&tokens[1..]);
    let bp = parse_bit_tokens(&tokens[1..1 + bit_count], width)?;
    let rest = &tokens[1 + bit_count..];

    // Find @format reference
    let fmt_ref = rest
        .iter()
        .find_map(|t| t.strip_prefix('@').map(|s| s.to_string()));

    let (args_name, field_map, fmt_bits, fmt_mask);
    if let Some(ref fname) = fmt_ref {
        let fmt = formats
            .get(fname)
            .ok_or_else(|| format!("unknown format @{fname}"))?;
        fmt_bits = fmt.fixedbits;
        fmt_mask = fmt.fixedmask;
        let (_, extra_map) = parse_attrs(rest, fields)?;
        let mut fm = fmt.field_map.clone();
        fm.extend(extra_map);
        args_name = fmt.args_name.clone();
        field_map = fm;
    } else {
        fmt_bits = 0;
        fmt_mask = 0;
        let (an, mut fm) = parse_attrs(rest, fields)?;
        // Add inline fields from bit pattern
        for (fname, (pos, len)) in &bp.inline_fields {
            fm.entry(fname.clone()).or_insert(FieldMapping::Inline {
                pos: *pos,
                len: *len,
                signed: false,
            });
        }
        if an.is_empty() && !fm.is_empty() {
            // Auto-generate argset
            let aname = format!("_auto_{name}");
            let afields: Vec<String> = fm.keys().cloned().collect();
            auto_args.insert(
                aname.clone(),
                ArgSet {
                    name: aname.clone(),
                    fields: afields,
                    is_extern: false,
                },
            );
            args_name = aname;
        } else {
            args_name = an;
        }
        field_map = fm;
    }

    Ok(Pattern {
        name,
        fixedbits: bp.fixedbits | fmt_bits,
        fixedmask: bp.fixedmask | fmt_mask,
        args_name,
        field_map,
    })
}

/// Merge backslash-continuation lines into single logical
/// lines.  A trailing `\` joins the next line.
pub fn merge_continuations(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cont = false;
    for line in input.lines() {
        if cont {
            // Append to previous logical line (space-separated).
            out.push(' ');
            out.push_str(line.trim());
        } else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
        }
        cont = out.ends_with('\\');
        if cont {
            out.pop(); // remove trailing backslash
                       // Trim trailing whitespace before the backslash
            while out.ends_with(' ') {
                out.pop();
            }
        }
    }
    out
}

pub fn parse_with_width(input: &str, width: u32) -> Result<Parsed, String> {
    let merged = merge_continuations(input);
    let mut fields = BTreeMap::new();
    let mut argsets = BTreeMap::new();
    let mut formats = BTreeMap::new();
    let mut patterns = Vec::new();
    let mut auto_args = BTreeMap::new();

    for (lineno, raw) in merged.lines().enumerate() {
        let line = match raw.find('#') {
            Some(i) => &raw[..i],
            None => raw,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let first = line.chars().next().unwrap();
        let result: Result<(), String> = match first {
            '%' => {
                let f = parse_field(line)?;
                fields.insert(f.name.clone(), f);
                Ok(())
            }
            '&' => {
                let a = parse_argset(line)?;
                argsets.insert(a.name.clone(), a);
                Ok(())
            }
            '@' => {
                let (n, f) = parse_format(line, &fields, width)?;
                formats.insert(n, f);
                Ok(())
            }
            '{' | '}' | '[' | ']' => Ok(()),
            _ => {
                let p = parse_pattern(
                    line,
                    &formats,
                    &fields,
                    &mut auto_args,
                    width,
                )?;
                patterns.push(p);
                Ok(())
            }
        };
        result.map_err(|e: String| format!("line {}: {e}", lineno + 1))?;
    }
    argsets.extend(auto_args);
    Ok(Parsed {
        fields,
        argsets,
        patterns,
    })
}

// ── Code generation ────────────────────────────────────────────

pub fn format_hex(val: u32, width: u32) -> String {
    if width <= 16 {
        format!("{val:#06x}")
    } else {
        format!("{val:#010x}")
    }
}

pub fn to_camel(s: &str) -> String {
    let mut result = String::new();
    let mut upper = true;
    for c in s.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            result.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            result.push(c);
        }
    }
    result
}

fn emit_arg_structs(
    w: &mut dyn Write,
    argsets: &BTreeMap<String, ArgSet>,
) -> std::io::Result<()> {
    for a in argsets.values() {
        if a.is_extern {
            continue;
        }
        let sname = format!("Args{}", to_camel(&a.name));
        writeln!(w, "#[derive(Debug, Clone, Copy, Default)]")?;
        writeln!(w, "pub struct {sname} {{")?;
        for f in &a.fields {
            writeln!(w, "    pub {f}: i64,")?;
        }
        writeln!(w, "}}\n")?;
    }
    Ok(())
}

fn emit_extract_field(
    w: &mut dyn Write,
    field: &Field,
    width: u32,
) -> std::io::Result<()> {
    let insn_ty = if width <= 16 { "u16" } else { "u32" };
    let signed_ty = if width <= 16 { "i16" } else { "i32" };
    writeln!(w, "fn extract_{}(insn: {insn_ty}) -> i64 {{", field.name)?;
    let segs = &field.segments;
    if segs.len() == 1 {
        let s = &segs[0];
        if s.signed {
            let lshift = width - s.pos - s.len;
            let rshift = width - s.len;
            if lshift == 0 {
                writeln!(
                    w,
                    "    let val = \
                     (insn as {signed_ty}) >> {rshift};",
                )?;
            } else {
                writeln!(
                    w,
                    "    let val = \
                     ((insn as {signed_ty}) \
                     << {lshift}) >> {rshift};",
                )?;
            }
        } else {
            let mask = (1u32 << s.len) - 1;
            writeln!(w, "    let val = (insn >> {}) & {:#x};", s.pos, mask)?;
        }
    } else {
        // Multi-segment: first may be signed
        let s0 = &segs[0];
        if s0.signed {
            let lshift = width - s0.pos - s0.len;
            let rshift = width - s0.len;
            if lshift == 0 {
                writeln!(
                    w,
                    "    let mut val: i64 = \
                     ((insn as {signed_ty}) \
                     >> {rshift}) as i64;",
                )?;
            } else {
                writeln!(
                    w,
                    "    let mut val: i64 = \
                     (((insn as {signed_ty}) \
                     << {lshift}) \
                     >> {rshift}) as i64;",
                )?;
            }
        } else {
            let mask = (1u32 << s0.len) - 1;
            writeln!(
                w,
                "    let mut val: i64 = \
                 ((insn >> {}) & {:#x}) as i64;",
                s0.pos, mask
            )?;
        }
        for s in &segs[1..] {
            let mask = (1u32 << s.len) - 1;
            writeln!(
                w,
                "    val = (val << {}) \
                 | ((insn >> {}) & {:#x}) as i64;",
                s.len, s.pos, mask
            )?;
        }
    }
    let cast = if segs.len() == 1 { "val as i64" } else { "val" };
    if let Some(ref func) = field.func {
        emit_func_transform(w, func, cast)?;
    } else {
        writeln!(w, "    {cast}")?;
    }
    writeln!(w, "}}\n")
}

/// Emit the transform expression for a `!function=` handler.
fn emit_func_transform(
    w: &mut dyn Write,
    func: &str,
    cast: &str,
) -> std::io::Result<()> {
    match func {
        "ex_shift_1" => {
            writeln!(w, "    ({cast}) << 1")
        }
        "ex_shift_2" => {
            writeln!(w, "    ({cast}) << 2")
        }
        "ex_shift_3" => {
            writeln!(w, "    ({cast}) << 3")
        }
        "ex_shift_4" => {
            writeln!(w, "    ({cast}) << 4")
        }
        "ex_shift_12" => {
            writeln!(w, "    ({cast}) << 12")
        }
        "ex_rvc_register" => {
            writeln!(w, "    ({cast}) + 8")
        }
        "ex_sreg_register" => {
            writeln!(
                w,
                "    [8,9,18,19,20,21,22,23]\
                 [({cast}) as usize & 7]"
            )
        }
        "ex_rvc_shiftli" | "ex_rvc_shiftri" => {
            // Identity for RV64
            writeln!(w, "    {cast}")
        }
        _ => {
            writeln!(w, "    // unknown func: {func}\n    {cast}")
        }
    }
}

fn emit_field_expr(
    w: &mut dyn Write,
    _fname: &str,
    mapping: &FieldMapping,
    width: u32,
) -> std::io::Result<()> {
    let signed_ty = if width <= 16 { "i16" } else { "i32" };
    match mapping {
        FieldMapping::FieldRef(r) => {
            write!(w, "extract_{r}(insn)")?;
        }
        FieldMapping::Inline { pos, len, signed } => {
            if *signed {
                let shift = width - pos - len;
                write!(
                    w,
                    "(((insn as {signed_ty}) \
                     << {shift}) >> {}) as i64",
                    width - len
                )?;
            } else {
                let mask = (1u32 << len) - 1;
                write!(w, "((insn >> {pos}) & {mask:#x}) as i64")?;
            }
        }
        FieldMapping::Const(c) => {
            write!(w, "{c}_i64")?;
        }
    }
    Ok(())
}

fn emit_decode_trait(
    w: &mut dyn Write,
    patterns: &[Pattern],
    argsets: &BTreeMap<String, ArgSet>,
    width: u32,
) -> std::io::Result<()> {
    let trait_name = if width <= 16 { "Decode16" } else { "Decode" };
    writeln!(w, "pub trait {trait_name}<Ir> {{")?;
    let mut seen = std::collections::HashSet::new();
    for p in patterns {
        if !seen.insert(&p.name) {
            continue; // skip duplicate trait methods
        }
        let sname = if p.args_name.is_empty() {
            "ArgsEmpty".to_string()
        } else {
            format!("Args{}", to_camel(&p.args_name))
        };
        writeln!(
            w,
            "    fn trans_{}(\
             &mut self, ir: &mut Ir, a: &{sname}\
             ) -> bool;",
            p.name
        )?;
    }
    writeln!(w, "}}\n")?;
    let needs_empty = patterns.iter().any(|p| p.args_name.is_empty());
    if needs_empty && !argsets.contains_key("empty") {
        // Already emitted by argsets if &empty exists
    }
    Ok(())
}

fn emit_decode_fn(
    w: &mut dyn Write,
    patterns: &[Pattern],
    argsets: &BTreeMap<String, ArgSet>,
    width: u32,
) -> std::io::Result<()> {
    let insn_ty = if width <= 16 { "u16" } else { "u32" };
    let trait_name = if width <= 16 { "Decode16" } else { "Decode" };
    let fn_name = if width <= 16 { "decode16" } else { "decode" };
    let full_mask: u32 = if width <= 16 { 0xffff } else { 0xffff_ffff };
    writeln!(
        w,
        "pub fn {fn_name}<Ir, T: {trait_name}<Ir>>(\
         ctx: &mut T, ir: &mut Ir, insn: {insn_ty}\
         ) -> bool {{"
    )?;
    for p in patterns {
        let sname = if p.args_name.is_empty() {
            "ArgsEmpty".to_string()
        } else {
            format!("Args{}", to_camel(&p.args_name))
        };
        if p.fixedmask == full_mask {
            let bits = format_hex(p.fixedbits, width);
            writeln!(w, "    if insn == {bits} {{")?;
        } else {
            let mask = format_hex(p.fixedmask, width);
            let bits = format_hex(p.fixedbits, width);
            writeln!(w, "    if insn & {mask} == {bits} {{")?;
        }
        // Build args struct
        let arg_fields = if p.args_name.is_empty() {
            Vec::new()
        } else if let Some(a) = argsets.get(&p.args_name) {
            a.fields.clone()
        } else {
            Vec::new()
        };
        if arg_fields.is_empty() {
            writeln!(
                w,
                "        return ctx.trans_{}(\
                 ir, &{sname} {{}});",
                p.name
            )?;
        } else {
            writeln!(w, "        let a = {sname} {{")?;
            for af in &arg_fields {
                if let Some(mapping) = p.field_map.get(af) {
                    write!(w, "            {af}: ")?;
                    emit_field_expr(w, af, mapping, width)?;
                    writeln!(w, ",")?;
                } else {
                    writeln!(w, "            {af}: 0,")?;
                }
            }
            writeln!(w, "        }};")?;
            writeln!(w, "        return ctx.trans_{}(ir, &a);", p.name)?;
        }
        writeln!(w, "    }}")?;
    }
    writeln!(w, "    false")?;
    writeln!(w, "}}\n")
}

// ── Public API ─────────────────────────────────────────────────

pub fn generate_with_width(
    input: &str,
    output: &mut dyn Write,
    width: u32,
) -> Result<(), String> {
    let parsed = parse_with_width(input, width)?;
    writeln!(output, "// Auto-generated by decodetree.")
        .map_err(|e| e.to_string())?;
    writeln!(output, "// Do not edit.\n").map_err(|e| e.to_string())?;
    emit_arg_structs(output, &parsed.argsets).map_err(|e| e.to_string())?;
    for field in parsed.fields.values() {
        emit_extract_field(output, field, width).map_err(|e| e.to_string())?;
    }
    emit_decode_trait(output, &parsed.patterns, &parsed.argsets, width)
        .map_err(|e| e.to_string())?;
    emit_decode_fn(output, &parsed.patterns, &parsed.argsets, width)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn generate(input: &str, output: &mut dyn Write) -> Result<(), String> {
    generate_with_width(input, output, 32)
}
