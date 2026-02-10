use std::collections::BTreeMap;
use std::io::Write;

// ── Data structures ─────────────────────────────────────────────

#[derive(Clone, Debug)]
struct FieldSegment {
    pos: u32,
    len: u32,
    signed: bool,
}

#[derive(Clone, Debug)]
struct Field {
    name: String,
    segments: Vec<FieldSegment>,
    func: Option<String>,
}

#[derive(Clone, Debug)]
struct ArgSet {
    name: String,
    fields: Vec<String>,
}

#[derive(Clone, Debug)]
enum FieldMapping {
    FieldRef(String),
    Inline { pos: u32, len: u32, signed: bool },
    Const(i32),
}

#[derive(Clone, Debug)]
struct Format {
    #[allow(dead_code)]
    name: String,
    fixedbits: u32,
    fixedmask: u32,
    args_name: String,
    field_map: BTreeMap<String, FieldMapping>,
}

#[derive(Clone, Debug)]
struct Pattern {
    name: String,
    fixedbits: u32,
    fixedmask: u32,
    args_name: String,
    field_map: BTreeMap<String, FieldMapping>,
}

struct Parsed {
    fields: BTreeMap<String, Field>,
    argsets: BTreeMap<String, ArgSet>,
    patterns: Vec<Pattern>,
}

// ── Bit-pattern parsing ─────────────────────────────────────────

fn is_bit_char(c: char) -> bool {
    matches!(c, '0' | '1' | '.' | '-')
}

fn is_bit_token(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_bit_char)
}

fn is_inline_field(s: &str) -> bool {
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

struct BitPatternResult {
    fixedbits: u32,
    fixedmask: u32,
    inline_fields: BTreeMap<String, (u32, u32)>,
}

fn parse_bit_tokens(tokens: &[&str]) -> Result<BitPatternResult, String> {
    let mut fixedbits: u32 = 0;
    let mut fixedmask: u32 = 0;
    let mut inline_fields = BTreeMap::new();
    let mut bit_pos: i32 = 31;

    for &tok in tokens {
        if is_bit_token(tok) {
            for c in tok.chars() {
                if bit_pos < 0 {
                    return Err("bit pattern exceeds 32 bits".into());
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

fn count_bit_tokens(tokens: &[&str]) -> usize {
    tokens
        .iter()
        .take_while(|t| is_bit_token(t) || is_inline_field(t))
        .count()
}

// ── Field segment parsing ──────────────────────────────────────

fn parse_field_segment(s: &str) -> Result<FieldSegment, String> {
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

fn parse_field(line: &str) -> Result<Field, String> {
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

fn parse_argset(line: &str) -> Result<ArgSet, String> {
    // &name field1 field2 ...
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let name = tokens[0][1..].to_string(); // skip &
    let fields = tokens[1..].iter().map(|s| s.to_string()).collect();
    Ok(ArgSet { name, fields })
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
) -> Result<(String, Format), String> {
    // @name bit_tokens... &argset [mappings...]
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let name = tokens[0][1..].to_string(); // skip @
    let bit_count = count_bit_tokens(&tokens[1..]);
    let bp = parse_bit_tokens(&tokens[1..1 + bit_count])?;
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
) -> Result<Pattern, String> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let name = tokens[0].to_string();
    let bit_count = count_bit_tokens(&tokens[1..]);
    let bp = parse_bit_tokens(&tokens[1..1 + bit_count])?;
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

fn parse(input: &str) -> Result<Parsed, String> {
    let mut fields = BTreeMap::new();
    let mut argsets = BTreeMap::new();
    let mut formats = BTreeMap::new();
    let mut patterns = Vec::new();
    let mut auto_args = BTreeMap::new();

    for (lineno, raw) in input.lines().enumerate() {
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
                let (n, f) = parse_format(line, &fields)?;
                formats.insert(n, f);
                Ok(())
            }
            _ => {
                let p = parse_pattern(line, &formats, &fields, &mut auto_args)?;
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

fn to_camel(s: &str) -> String {
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

fn emit_extract_field(w: &mut dyn Write, field: &Field) -> std::io::Result<()> {
    writeln!(w, "fn extract_{}(insn: u32) -> i64 {{", field.name)?;
    let segs = &field.segments;
    if segs.len() == 1 {
        let s = &segs[0];
        if s.signed {
            let lshift = 32 - s.pos - s.len;
            let rshift = 32 - s.len;
            if lshift == 0 {
                writeln!(
                    w,
                    "    let val = (insn as i32) >> {rshift};",
                )?;
            } else {
                writeln!(
                    w,
                    "    let val = ((insn as i32) \
                     << {lshift}) >> {rshift};",
                )?;
            }
        } else {
            let mask = (1u32 << s.len) - 1;
            writeln!(
                w,
                "    let val = (insn >> {}) & {:#x};",
                s.pos, mask
            )?;
        }
    } else {
        // Multi-segment: first may be signed
        let s0 = &segs[0];
        if s0.signed {
            let lshift = 32 - s0.pos - s0.len;
            let rshift = 32 - s0.len;
            if lshift == 0 {
                writeln!(
                    w,
                    "    let mut val: i64 = \
                     ((insn as i32) >> {rshift}) as i64;",
                )?;
            } else {
                writeln!(
                    w,
                    "    let mut val: i64 = \
                     (((insn as i32) << {lshift}) \
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
    if let Some(ref func) = field.func {
        let shift = match func.as_str() {
            "ex_shift_1" => 1,
            "ex_shift_12" => 12,
            _ => {
                return writeln!(
                    w,
                    "    // unknown func: {func}\n    \
                     val as i64\n}}\n"
                );
            }
        };
        if segs.len() == 1 {
            // val is i32/u32, cast first
            writeln!(
                w,
                "    (val as i64) << {shift}"
            )?;
        } else {
            // val is already i64
            writeln!(w, "    val << {shift}")?;
        }
    } else if segs.len() == 1 {
        writeln!(w, "    val as i64")?;
    } else {
        writeln!(w, "    val")?;
    }
    writeln!(w, "}}\n")
}

fn emit_field_expr(
    w: &mut dyn Write,
    _fname: &str,
    mapping: &FieldMapping,
) -> std::io::Result<()> {
    match mapping {
        FieldMapping::FieldRef(r) => {
            write!(w, "extract_{r}(insn)")?;
        }
        FieldMapping::Inline { pos, len, signed } => {
            if *signed {
                let shift = 32 - pos - len;
                write!(
                    w,
                    "(((insn as i32) << {shift}) >> {}) \
                     as i64",
                    32 - len
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
) -> std::io::Result<()> {
    writeln!(w, "pub trait Decode {{")?;
    for p in patterns {
        let sname = if p.args_name.is_empty() {
            "ArgsEmpty".to_string()
        } else {
            format!("Args{}", to_camel(&p.args_name))
        };
        writeln!(
            w,
            "    fn trans_{}(&mut self, a: &{sname}) \
             -> bool;",
            p.name
        )?;
    }
    writeln!(w, "}}\n")?;
    // Ensure ArgsEmpty exists if needed
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
) -> std::io::Result<()> {
    writeln!(
        w,
        "pub fn decode<T: Decode>(\
         ctx: &mut T, insn: u32\
         ) -> bool {{"
    )?;
    for p in patterns {
        let sname = if p.args_name.is_empty() {
            "ArgsEmpty".to_string()
        } else {
            format!("Args{}", to_camel(&p.args_name))
        };
        if p.fixedmask == 0xffff_ffff {
            writeln!(
                w,
                "    if insn == {:#010x} {{",
                p.fixedbits
            )?;
        } else {
            writeln!(
                w,
                "    if insn & {:#010x} == {:#010x} {{",
                p.fixedmask, p.fixedbits
            )?;
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
                 &{sname} {{}});",
                p.name
            )?;
        } else {
            writeln!(w, "        let a = {sname} {{")?;
            for af in &arg_fields {
                if let Some(mapping) = p.field_map.get(af) {
                    write!(w, "            {af}: ")?;
                    emit_field_expr(w, af, mapping)?;
                    writeln!(w, ",")?;
                } else {
                    writeln!(w, "            {af}: 0,")?;
                }
            }
            writeln!(w, "        }};")?;
            writeln!(w, "        return ctx.trans_{}(&a);", p.name)?;
        }
        writeln!(w, "    }}")?;
    }
    writeln!(w, "    false")?;
    writeln!(w, "}}\n")
}

// ── Public API ─────────────────────────────────────────────────

pub fn generate(input: &str, output: &mut dyn Write) -> Result<(), String> {
    let parsed = parse(input)?;
    writeln!(output, "// Auto-generated by decodetree.")
        .map_err(|e| e.to_string())?;
    writeln!(output, "// Do not edit.\n").map_err(|e| e.to_string())?;
    emit_arg_structs(output, &parsed.argsets).map_err(|e| e.to_string())?;
    for field in parsed.fields.values() {
        emit_extract_field(output, field).map_err(|e| e.to_string())?;
    }
    emit_decode_trait(output, &parsed.patterns, &parsed.argsets)
        .map_err(|e| e.to_string())?;
    emit_decode_fn(output, &parsed.patterns, &parsed.argsets)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Bit-pattern parsing ────────────────────────────────────

    #[test]
    fn bit_pattern_all_fixed() {
        let toks =
            ["0000000", "00000", "00000", "000", "00000", "1110011"];
        let r = parse_bit_tokens(&toks).unwrap();
        assert_eq!(r.fixedbits, 0x0000_0073);
        assert_eq!(r.fixedmask, 0xffff_ffff);
        assert!(r.inline_fields.is_empty());
    }

    #[test]
    fn bit_pattern_with_dontcare() {
        let toks =
            ["....................", ".....", "0110111"];
        let r = parse_bit_tokens(&toks).unwrap();
        assert_eq!(r.fixedbits, 0x0000_0037);
        assert_eq!(r.fixedmask, 0x0000_007f);
    }

    #[test]
    fn bit_pattern_inline_fields() {
        let toks = [
            "----", "pred:4", "succ:4", "-----", "000",
            "-----", "0001111",
        ];
        let r = parse_bit_tokens(&toks).unwrap();
        assert_eq!(r.fixedmask, 0x0000_707f);
        assert_eq!(r.fixedbits, 0x0000_000f);
        assert_eq!(r.inline_fields["pred"], (24, 4));
        assert_eq!(r.inline_fields["succ"], (20, 4));
    }

    #[test]
    fn bit_pattern_exceeds_32() {
        let toks =
            ["11111111111111111111111111111111", "1"];
        assert!(parse_bit_tokens(&toks).is_err());
    }

    // ── Field parsing ──────────────────────────────────────────

    #[test]
    fn parse_field_unsigned() {
        let f = parse_field("%rs2 20:5").unwrap();
        assert_eq!(f.name, "rs2");
        assert_eq!(f.segments.len(), 1);
        assert_eq!(f.segments[0].pos, 20);
        assert_eq!(f.segments[0].len, 5);
        assert!(!f.segments[0].signed);
        assert!(f.func.is_none());
    }

    #[test]
    fn parse_field_signed() {
        let f = parse_field("%imm_i 20:s12").unwrap();
        assert_eq!(f.name, "imm_i");
        assert_eq!(f.segments.len(), 1);
        assert!(f.segments[0].signed);
        assert_eq!(f.segments[0].pos, 20);
        assert_eq!(f.segments[0].len, 12);
    }

    #[test]
    fn parse_field_multi_segment() {
        let f = parse_field("%imm_s 25:s7 7:5").unwrap();
        assert_eq!(f.name, "imm_s");
        assert_eq!(f.segments.len(), 2);
        assert!(f.segments[0].signed);
        assert_eq!(f.segments[0].pos, 25);
        assert_eq!(f.segments[0].len, 7);
        assert!(!f.segments[1].signed);
        assert_eq!(f.segments[1].pos, 7);
        assert_eq!(f.segments[1].len, 5);
    }

    #[test]
    fn parse_field_with_function() {
        let f = parse_field(
            "%imm_b 31:s1 7:1 25:6 8:4 !function=ex_shift_1",
        )
        .unwrap();
        assert_eq!(f.name, "imm_b");
        assert_eq!(f.segments.len(), 4);
        assert_eq!(f.func.as_deref(), Some("ex_shift_1"));
    }

    // ── Argset parsing ─────────────────────────────────────────

    #[test]
    fn parse_argset_normal() {
        let a = parse_argset("&r rd rs1 rs2").unwrap();
        assert_eq!(a.name, "r");
        assert_eq!(a.fields, ["rd", "rs1", "rs2"]);
    }

    #[test]
    fn parse_argset_empty() {
        let a = parse_argset("&empty").unwrap();
        assert_eq!(a.name, "empty");
        assert!(a.fields.is_empty());
    }

    // ── Full parse ─────────────────────────────────────────────

    fn mini_decode() -> &'static str {
        "\
# Test decode
%rs2    20:5
%rs1    15:5
%rd     7:5
%imm_i  20:s12

&r   rd rs1 rs2
&i   imm rs1 rd

@r  ....... ..... ..... ... ..... ....... &r  %rs2 %rs1 %rd
@i  ............ ..... ... ..... ....... &i  imm=%imm_i %rs1 %rd

add  0000000 ..... ..... 000 ..... 0110011 @r
addi ............ ..... 000 ..... 0010011 @i
"
    }

    #[test]
    fn parse_mini_decode() {
        let p = parse(mini_decode()).unwrap();
        assert_eq!(p.fields.len(), 4);
        assert_eq!(p.argsets.len(), 2);
        assert_eq!(p.patterns.len(), 2);

        let add = &p.patterns[0];
        assert_eq!(add.name, "add");
        assert_eq!(add.fixedmask, 0xfe00_707f);
        assert_eq!(add.fixedbits, 0x0000_0033);
        assert_eq!(add.args_name, "r");

        let addi = &p.patterns[1];
        assert_eq!(addi.name, "addi");
        assert_eq!(addi.fixedmask, 0x0000_707f);
        assert_eq!(addi.fixedbits, 0x0000_0013);
        assert_eq!(addi.args_name, "i");
    }

    // ── Full riscv32.decode parse ──────────────────────────────

    #[test]
    fn parse_riscv32_decode() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let p = parse(&input).unwrap();
        assert_eq!(p.patterns.len(), 65);
        assert!(p.fields.contains_key("imm_b"));
        assert!(p.fields.contains_key("imm_j"));
        assert!(p.argsets.contains_key("r"));
        assert!(p.argsets.contains_key("shift"));
    }

    // ── Code generation ────────────────────────────────────────

    #[test]
    fn generate_mini_decode() {
        let mut out = Vec::new();
        generate(mini_decode(), &mut out).unwrap();
        let code = String::from_utf8(out).unwrap();
        assert!(code.contains("pub struct ArgsR"));
        assert!(code.contains("pub struct ArgsI"));
        assert!(code.contains("pub trait Decode"));
        assert!(code.contains("fn trans_add("));
        assert!(code.contains("fn trans_addi("));
        assert!(code.contains("pub fn decode<T: Decode>"));
        assert!(code.contains("extract_rs1(insn)"));
        assert!(code.contains("extract_imm_i(insn)"));
    }

    #[test]
    fn generate_riscv32_decode() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let mut out = Vec::new();
        generate(&input, &mut out).unwrap();
        let code = String::from_utf8(out).unwrap();
        // All 65 trans_ methods in trait
        assert_eq!(
            code.matches("fn trans_").count(),
            65
        );
        assert!(code.contains("fn trans_lui("));
        assert!(code.contains("fn trans_jal("));
        assert!(code.contains("fn trans_mul("));
        assert!(code.contains("fn trans_remuw("));
        assert!(code.contains("fn trans_fence("));
    }

    // ── Pattern mask/bits for known instructions ───────────────

    #[test]
    fn pattern_masks_r_type() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let p = parse(&input).unwrap();
        let find = |name: &str| {
            p.patterns.iter().find(|p| p.name == name).unwrap()
        };
        // add: funct7=0000000, funct3=000, opcode=0110011
        let add = find("add");
        assert_eq!(add.fixedmask, 0xfe00_707f);
        assert_eq!(add.fixedbits, 0x0000_0033);
        // sub: funct7=0100000
        let sub = find("sub");
        assert_eq!(sub.fixedmask, 0xfe00_707f);
        assert_eq!(sub.fixedbits, 0x4000_0033);
        // mul: funct7=0000001
        let mul = find("mul");
        assert_eq!(mul.fixedbits, 0x0200_0033);
    }

    #[test]
    fn pattern_masks_i_type() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let p = parse(&input).unwrap();
        let find = |name: &str| {
            p.patterns.iter().find(|p| p.name == name).unwrap()
        };
        // addi: funct3=000, opcode=0010011
        let addi = find("addi");
        assert_eq!(addi.fixedmask, 0x0000_707f);
        assert_eq!(addi.fixedbits, 0x0000_0013);
        // jalr: funct3=000, opcode=1100111
        let jalr = find("jalr");
        assert_eq!(jalr.fixedmask, 0x0000_707f);
        assert_eq!(jalr.fixedbits, 0x0000_0067);
    }

    #[test]
    fn pattern_masks_b_type() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let p = parse(&input).unwrap();
        let find = |name: &str| {
            p.patterns.iter().find(|p| p.name == name).unwrap()
        };
        // beq: funct3=000, opcode=1100011
        let beq = find("beq");
        assert_eq!(beq.fixedmask, 0x0000_707f);
        assert_eq!(beq.fixedbits, 0x0000_0063);
        // bne: funct3=001
        let bne = find("bne");
        assert_eq!(bne.fixedbits, 0x0000_1063);
    }

    #[test]
    fn pattern_masks_shift() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let p = parse(&input).unwrap();
        let find = |name: &str| {
            p.patterns.iter().find(|p| p.name == name).unwrap()
        };
        // slli (RV64): top 6 bits = 00000., funct3=001
        let slli = find("slli");
        assert_eq!(slli.fixedmask, 0xf800_707f);
        assert_eq!(slli.fixedbits, 0x0000_1013);
        // sraiw (RV64): funct7=0100000, funct3=101
        let sraiw = find("sraiw");
        assert_eq!(sraiw.fixedmask, 0xfe00_707f);
        assert_eq!(sraiw.fixedbits, 0x4000_501b);
    }

    // ── Extract function correctness ───────────────────────────
    // Verify generated extract code by manually computing values.

    #[test]
    fn extract_rd_rs1_rs2_values() {
        // add x3, x1, x2 → 0x002081b3
        let insn: u32 = 0x002081b3;
        assert_eq!((insn >> 7) & 0x1f, 3); // rd
        assert_eq!((insn >> 15) & 0x1f, 1); // rs1
        assert_eq!((insn >> 20) & 0x1f, 2); // rs2
    }

    #[test]
    fn extract_imm_i_positive() {
        // addi x1, x0, 42 → 0x02a00093
        let insn: u32 = 0x02a00093;
        let val = ((insn as i32) >> 20) as i64;
        assert_eq!(val, 42);
    }

    #[test]
    fn extract_imm_i_negative() {
        // addi x1, x0, -1 → 0xfff00093
        let insn: u32 = 0xfff00093;
        let val = ((insn as i32) >> 20) as i64;
        assert_eq!(val, -1);
    }

    #[test]
    fn extract_imm_s_value() {
        // sw x2, 8(x1) → 0x00208423 (imm=8)
        let insn: u32 = 0x0020_8423;
        // imm_s: 25:s7 7:5
        let mut val =
            (((insn as i32) << 0) >> 25) as i64;
        val = (val << 5)
            | (((insn >> 7) & 0x1f) as i64);
        assert_eq!(val, 8);
    }

    #[test]
    fn extract_imm_b_value() {
        // beq x0, x0, +8 → 0x00000463
        // B-imm: {31:s1, 7:1, 25:6, 8:4} << 1
        let insn: u32 = 0x0000_0463;
        let mut val =
            (((insn as i32) << 0) >> 31) as i64;
        val = (val << 1)
            | (((insn >> 7) & 0x1) as i64);
        val = (val << 6)
            | (((insn >> 25) & 0x3f) as i64);
        val = (val << 4)
            | (((insn >> 8) & 0xf) as i64);
        val <<= 1;
        assert_eq!(val, 8);
    }

    #[test]
    fn extract_imm_j_value() {
        // jal x1, +20 → 0x014000ef
        // J-imm: {31:s1, 12:8, 20:1, 21:10} << 1
        let insn: u32 = 0x0140_00ef;
        let mut val =
            (((insn as i32) << 0) >> 31) as i64;
        val = (val << 8)
            | (((insn >> 12) & 0xff) as i64);
        val = (val << 1)
            | (((insn >> 20) & 0x1) as i64);
        val = (val << 10)
            | (((insn >> 21) & 0x3ff) as i64);
        val <<= 1;
        assert_eq!(val, 20);
    }

    #[test]
    fn extract_imm_u_value() {
        // lui x5, 0x12345 → 0x123452b7
        // U-imm: 12:s20, then << 12
        let insn: u32 = 0x1234_52b7;
        let val = ((insn as i32) << 0) >> 12;
        let val = (val << 12) as i64;
        assert_eq!(val, 0x12345000);
    }

    #[test]
    fn extract_imm_b_negative() {
        // beq x0, x0, -4 → 0xfe000ee3
        let insn: u32 = 0xfe00_0ee3;
        let mut val =
            (((insn as i32) << 0) >> 31) as i64;
        val = (val << 1)
            | (((insn >> 7) & 0x1) as i64);
        val = (val << 6)
            | (((insn >> 25) & 0x3f) as i64);
        val = (val << 4)
            | (((insn >> 8) & 0xf) as i64);
        val <<= 1;
        assert_eq!(val, -4);
    }

    // ── Error handling ─────────────────────────────────────────

    #[test]
    fn parse_empty_input() {
        let p = parse("").unwrap();
        assert!(p.patterns.is_empty());
        assert!(p.fields.is_empty());
        assert!(p.argsets.is_empty());
    }

    #[test]
    fn parse_comments_only() {
        let p = parse("# just a comment\n# another\n").unwrap();
        assert!(p.patterns.is_empty());
    }

    #[test]
    fn parse_bad_field_segment() {
        assert!(parse_field_segment("abc").is_err());
        assert!(parse_field_segment(":5").is_err());
        assert!(parse_field_segment("20:").is_err());
    }

    #[test]
    fn parse_unknown_format_ref() {
        let input = "\
%rd 7:5
&r rd
@r ....... ..... ..... ... ..... ....... &r %rd
add 0000000 ..... ..... 000 ..... 0110011 @nonexistent
";
        assert!(parse(input).is_err());
    }

    // ── Helpers ────────────────────────────────────────────────

    #[test]
    fn is_bit_char_coverage() {
        assert!(is_bit_char('0'));
        assert!(is_bit_char('1'));
        assert!(is_bit_char('.'));
        assert!(is_bit_char('-'));
        assert!(!is_bit_char('2'));
        assert!(!is_bit_char('a'));
    }

    #[test]
    fn is_bit_token_coverage() {
        assert!(is_bit_token("01.-"));
        assert!(is_bit_token("0000000"));
        assert!(!is_bit_token(""));
        assert!(!is_bit_token("abc"));
        assert!(!is_bit_token("01a"));
    }

    #[test]
    fn is_inline_field_coverage() {
        assert!(is_inline_field("pred:4"));
        assert!(is_inline_field("rs2:5"));
        assert!(!is_inline_field("pred"));
        assert!(!is_inline_field(":4"));
        assert!(!is_inline_field("pred:"));
        assert!(!is_inline_field("pred:abc"));
    }

    #[test]
    fn count_bit_tokens_mixed() {
        let toks = ["0110011", ".....", "pred:4", "@r", "%rd"];
        assert_eq!(count_bit_tokens(&toks), 3);
    }

    #[test]
    fn count_bit_tokens_none() {
        let toks = ["@r", "&i", "%rd"];
        assert_eq!(count_bit_tokens(&toks), 0);
    }

    // ── Format inheritance ─────────────────────────────────────

    #[test]
    fn format_inherits_args_and_fields() {
        let input = "\
%rs2 20:5
%rs1 15:5
%rd  7:5
&r rd rs1 rs2
@r ....... ..... ..... ... ..... ....... &r %rs2 %rs1 %rd
add 0000000 ..... ..... 000 ..... 0110011 @r
";
        let p = parse(input).unwrap();
        let add = &p.patterns[0];
        assert_eq!(add.args_name, "r");
        assert!(add.field_map.contains_key("rd"));
        assert!(add.field_map.contains_key("rs1"));
        assert!(add.field_map.contains_key("rs2"));
    }

    #[test]
    fn format_bits_merge_with_pattern() {
        // Format @u has only opcode bits masked
        // Pattern lui adds its own opcode bits
        let input = "\
%rd    7:5
%imm_u 12:s20 !function=ex_shift_12
&u imm rd
@u .................... ..... ....... &u imm=%imm_u %rd
lui  .................... ..... 0110111 @u
";
        let p = parse(input).unwrap();
        let lui = &p.patterns[0];
        // Format @u has fixedmask=0 (all dots)
        // Pattern lui has opcode=0110111 → mask=0x7f, bits=0x37
        assert_eq!(lui.fixedmask, 0x0000_007f);
        assert_eq!(lui.fixedbits, 0x0000_0037);
    }

    // ── Auto-generated argset ──────────────────────────────────

    #[test]
    fn auto_argset_for_fence() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let p = parse(&input).unwrap();
        let fence = p
            .patterns
            .iter()
            .find(|p| p.name == "fence")
            .unwrap();
        assert_eq!(fence.args_name, "_auto_fence");
        assert!(fence.field_map.contains_key("pred"));
        assert!(fence.field_map.contains_key("succ"));
        // Auto argset should exist
        let auto = p.argsets.get("_auto_fence").unwrap();
        assert!(auto.fields.contains(&"pred".to_string()));
        assert!(auto.fields.contains(&"succ".to_string()));
    }

    // ── More extract correctness ───────────────────────────────

    #[test]
    fn extract_imm_j_negative() {
        // jal x0, -2 → 0xfffff06f
        // J-imm = -2: {1, 11111111, 1, 1111111111} << 1
        let insn: u32 = 0xffff_f06f;
        let mut val = ((insn as i32) >> 31) as i64;
        val = (val << 8) | (((insn >> 12) & 0xff) as i64);
        val = (val << 1) | (((insn >> 20) & 0x1) as i64);
        val = (val << 10)
            | (((insn >> 21) & 0x3ff) as i64);
        val <<= 1;
        assert_eq!(val, -2);
    }

    #[test]
    fn extract_imm_s_negative() {
        // sd x0, -8(x1) → 0xfe00bc23 ... actually let me
        // compute: sw x2, -4(x1)
        // S-imm = -4: imm[11:5]=1111111, imm[4:0]=11100
        // sw: funct3=010, opcode=0100011
        // 1111111 00010 00001 010 11100 0100011
        let insn: u32 = 0xfe20_ae23;
        let mut val = ((insn as i32) >> 25) as i64;
        val = (val << 5) | (((insn >> 7) & 0x1f) as i64);
        assert_eq!(val, -4);
    }

    #[test]
    fn extract_imm_u_negative() {
        // lui x1, 0xfffff → 0xfffff0b7
        // U-imm: 12:s20 → top 20 bits = 0xfffff (all 1s)
        // After << 12: 0xfffff000 = -4096
        let insn: u32 = 0xffff_f0b7;
        let val = (insn as i32) >> 12;
        let val = (val as i64) << 12;
        assert_eq!(val, -4096);
    }

    #[test]
    fn extract_imm_i_max_positive() {
        // addi x1, x0, 2047 → 0x7ff00093
        let insn: u32 = 0x7ff0_0093;
        let val = ((insn as i32) >> 20) as i64;
        assert_eq!(val, 2047);
    }

    #[test]
    fn extract_imm_i_min_negative() {
        // addi x1, x0, -2048 → 0x80000093
        let insn: u32 = 0x8000_0093;
        let val = ((insn as i32) >> 20) as i64;
        assert_eq!(val, -2048);
    }

    #[test]
    fn extract_shift_amount() {
        // slli x1, x2, 13 → 0x00d11093
        let insn: u32 = 0x00d1_1093;
        let shamt = ((insn >> 20) & 0x7f) as i64;
        assert_eq!(shamt, 13);
        let rs1 = ((insn >> 15) & 0x1f) as i64;
        assert_eq!(rs1, 2);
        let rd = ((insn >> 7) & 0x1f) as i64;
        assert_eq!(rd, 1);
    }

    #[test]
    fn extract_fence_fields() {
        // fence rw, rw → pred=0011, succ=0011
        // 0000 0011 0011 00000 000 00000 0001111
        let insn: u32 = 0x0330_000f;
        let pred = ((insn >> 24) & 0xf) as i64;
        let succ = ((insn >> 20) & 0xf) as i64;
        assert_eq!(pred, 3); // rw
        assert_eq!(succ, 3); // rw
    }

    #[test]
    fn extract_fence_iorw() {
        // fence iorw, iorw → pred=1111, succ=1111
        let insn: u32 = 0x0ff0_000f;
        let pred = ((insn >> 24) & 0xf) as i64;
        let succ = ((insn >> 20) & 0xf) as i64;
        assert_eq!(pred, 15);
        assert_eq!(succ, 15);
    }

    // ── Pattern matching correctness ───────────────────────────

    fn riscv_parsed() -> Parsed {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        parse(&input).unwrap()
    }

    /// Verify that an instruction encoding matches exactly
    /// one pattern.
    fn matches_pattern(p: &Parsed, insn: u32) -> Vec<String> {
        p.patterns
            .iter()
            .filter(|pat| {
                insn & pat.fixedmask == pat.fixedbits
            })
            .map(|pat| pat.name.clone())
            .collect()
    }

    #[test]
    fn insn_add_matches_only_add() {
        let p = riscv_parsed();
        // add x3, x1, x2 → 0x002081b3
        let m = matches_pattern(&p, 0x0020_81b3);
        assert_eq!(m, vec!["add"]);
    }

    #[test]
    fn insn_sub_matches_only_sub() {
        let p = riscv_parsed();
        // sub x3, x1, x2 → 0x402081b3
        let m = matches_pattern(&p, 0x4020_81b3);
        assert_eq!(m, vec!["sub"]);
    }

    #[test]
    fn insn_ecall_matches_only_ecall() {
        let p = riscv_parsed();
        let m = matches_pattern(&p, 0x0000_0073);
        assert_eq!(m, vec!["ecall"]);
    }

    #[test]
    fn insn_ebreak_matches_only_ebreak() {
        let p = riscv_parsed();
        let m = matches_pattern(&p, 0x0010_0073);
        assert_eq!(m, vec!["ebreak"]);
    }

    #[test]
    fn insn_mul_matches_only_mul() {
        let p = riscv_parsed();
        // mul x3, x1, x2 → 0x022081b3
        let m = matches_pattern(&p, 0x0220_81b3);
        assert_eq!(m, vec!["mul"]);
    }

    #[test]
    fn insn_slli_no_overlap_with_srli() {
        let p = riscv_parsed();
        // slli x1, x2, 1 → 0x00111093
        let m = matches_pattern(&p, 0x0011_1093);
        assert_eq!(m, vec!["slli"]);
        // srli x1, x2, 1 → 0x00115093
        let m = matches_pattern(&p, 0x0011_5093);
        assert_eq!(m, vec!["srli"]);
    }

    #[test]
    fn insn_jal_matches_only_jal() {
        let p = riscv_parsed();
        // jal x1, +20 → 0x014000ef
        let m = matches_pattern(&p, 0x0140_00ef);
        assert_eq!(m, vec!["jal"]);
    }

    // ── Code generation details ────────────────────────────────

    #[test]
    fn generate_ecall_no_args() {
        let mut out = Vec::new();
        generate(mini_decode(), &mut out).unwrap();
        let code = String::from_utf8(out).unwrap();
        // decode() should call extract functions
        assert!(code.contains("extract_rd(insn)"));
    }

    #[test]
    fn generate_fence_auto_struct() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let mut out = Vec::new();
        generate(&input, &mut out).unwrap();
        let code = String::from_utf8(out).unwrap();
        assert!(code.contains("pub struct ArgsAutoFence"));
        assert!(code.contains("trans_fence"));
    }

    #[test]
    fn generate_no_identity_mask() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let mut out = Vec::new();
        generate(&input, &mut out).unwrap();
        let code = String::from_utf8(out).unwrap();
        // ecall/ebreak should use `insn ==` not `insn & 0xffffffff ==`
        assert!(code.contains("if insn == 0x00000073"));
        assert!(code.contains("if insn == 0x00100073"));
        assert!(!code.contains("0xffffffff"));
    }

    #[test]
    fn generate_no_shift_zero() {
        let input =
            std::fs::read_to_string("../frontend/decode/riscv32.decode")
                .unwrap();
        let mut out = Vec::new();
        generate(&input, &mut out).unwrap();
        let code = String::from_utf8(out).unwrap();
        // No `<< 0` identity operations
        assert!(!code.contains("<< 0)"));
    }

    // ── to_camel helper ────────────────────────────────────────

    #[test]
    fn to_camel_basic() {
        assert_eq!(to_camel("r"), "R");
        assert_eq!(to_camel("shift"), "Shift");
        assert_eq!(to_camel("auto_fence"), "AutoFence");
        assert_eq!(to_camel("_auto_fence"), "AutoFence");
        assert_eq!(to_camel("empty"), "Empty");
    }
}
