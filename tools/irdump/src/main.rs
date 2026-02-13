//! tcg-irdump — static ELF → IR dump tool.
//!
//! Reads a RISC-V ELF binary, translates it TB-by-TB into TCG IR,
//! and prints the IR in a human-readable format.

mod elf;

use std::env;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::process;

use tcg_core::context::Context;
use tcg_core::dump::dump_ops_with;
use tcg_core::TempIdx;
use tcg_frontend::riscv::cpu::NUM_GPRS;
use tcg_frontend::riscv::ext::RiscvCfg;
use tcg_frontend::riscv::{RiscvDisasContext, RiscvTranslator};
use tcg_frontend::{translator_loop, DisasJumpType, TranslatorOps};

struct Args {
    elf_path: String,
    output: Option<String>,
    start: Option<u64>,
    count: Option<usize>,
    max_insns: u32,
}

fn parse_args() -> Args {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: tcg-irdump <elf> [options]\n\
             \n\
             Options:\n  \
               -o <file>       Output to file\n  \
               --start <hex>   Start address\n  \
               --count <n>     Max TBs to translate\n  \
               --max-insns <n> Max insns per TB (default: 512)"
        );
        process::exit(1);
    }

    let mut a = Args {
        elf_path: args[1].clone(),
        output: None,
        start: None,
        count: None,
        max_insns: 512,
    };

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                a.output = Some(args[i].clone());
            }
            "--start" => {
                i += 1;
                let s = args[i].trim_start_matches("0x");
                a.start = Some(
                    u64::from_str_radix(s, 16).expect("invalid hex address"),
                );
            }
            "--count" => {
                i += 1;
                a.count = Some(args[i].parse().expect("invalid count"));
            }
            "--max-insns" => {
                i += 1;
                a.max_insns = args[i].parse().expect("invalid max-insns");
            }
            other => {
                eprintln!("unknown option: {other}");
                process::exit(1);
            }
        }
        i += 1;
    }
    a
}

/// Build a flat guest memory image from ELF segments.
/// Returns (base_addr, image_buffer).
fn build_image(info: &elf::ElfInfo) -> (u64, Vec<u8>) {
    let exec_segs: Vec<&elf::Segment> =
        info.segments.iter().filter(|s| s.executable).collect();
    if exec_segs.is_empty() {
        eprintln!("no executable segments found");
        process::exit(1);
    }

    let lo = exec_segs.iter().map(|s| s.vaddr).min().unwrap();
    let hi = exec_segs
        .iter()
        .map(|s| s.vaddr + s.data.len() as u64)
        .max()
        .unwrap();

    let size = (hi - lo) as usize;
    let mut image = vec![0u8; size];
    for seg in &exec_segs {
        let off = (seg.vaddr - lo) as usize;
        let len = seg.data.len();
        image[off..off + len].copy_from_slice(&seg.data);
    }
    (lo, image)
}

/// Annotate an InsnStart line with raw encoding and disassembly.
///
/// # Safety
/// `guest_base + pc` must point to valid, readable guest memory.
fn insn_annotation(
    pc: u64,
    guest_base: *const u8,
    w: &mut dyn Write,
) -> io::Result<()> {
    unsafe {
        let ptr = guest_base.add(pc as usize);
        let half = (ptr as *const u16).read_unaligned();
        let len = if half & 0x3 != 0x3 { 2 } else { 4 };
        let data = std::slice::from_raw_parts(ptr, len);
        let (asm, _) = tcg_disas::riscv::print_insn_riscv64(pc, data);
        if len == 2 {
            write!(w, "  {half:04x}      {asm}")
        } else {
            let insn = (ptr as *const u32).read_unaligned();
            write!(w, "  {insn:08x}  {asm}")
        }
    }
}

/// Translate one TB starting at `pc` and dump its IR.
fn translate_tb(
    ir: &mut Context,
    pc: u64,
    guest_base: *const u8,
    cfg: RiscvCfg,
    max_insns: u32,
    w: &mut impl Write,
) -> (u64, DisasJumpType) {
    if ir.nb_globals() == 0 {
        // First TB — register globals via translator_loop.
        let mut d = RiscvDisasContext::new(pc, guest_base, cfg);
        d.base.max_insns = max_insns;
        translator_loop::<RiscvTranslator>(&mut d, ir);
        let gb = guest_base;
        dump_ops_with(ir, w, |pc, w| insn_annotation(pc, gb, w))
            .expect("write failed");
        (d.base.pc_next, d.base.is_jmp)
    } else {
        // Subsequent TBs — globals already registered.
        ir.reset();
        let mut d = RiscvDisasContext::new(pc, guest_base, cfg);
        d.base.max_insns = max_insns;
        d.env = TempIdx(0);
        for i in 0..NUM_GPRS {
            d.gpr[i] = TempIdx(1 + i as u32);
        }
        d.pc = TempIdx(1 + NUM_GPRS as u32);
        d.load_res = TempIdx(1 + NUM_GPRS as u32 + 1);
        d.load_val = TempIdx(1 + NUM_GPRS as u32 + 2);
        RiscvTranslator::tb_start(&mut d, ir);
        loop {
            RiscvTranslator::insn_start(&mut d, ir);
            RiscvTranslator::translate_insn(&mut d, ir);
            if d.base.is_jmp != DisasJumpType::Next {
                break;
            }
            if d.base.num_insns >= d.base.max_insns {
                d.base.is_jmp = DisasJumpType::TooMany;
                break;
            }
        }
        RiscvTranslator::tb_stop(&mut d, ir);
        let gb = guest_base;
        dump_ops_with(ir, w, |pc, w| insn_annotation(pc, gb, w))
            .expect("write failed");
        (d.base.pc_next, d.base.is_jmp)
    }
}

fn main() {
    let args = parse_args();

    let data = fs::read(&args.elf_path).unwrap_or_else(|e| {
        let p = &args.elf_path;
        eprintln!("failed to read {p}: {e}");
        process::exit(1);
    });

    let info = elf::parse(&data).unwrap_or_else(|e| {
        eprintln!("ELF parse error: {e}");
        process::exit(1);
    });

    let (base_addr, image) = build_image(&info);
    let image_end = base_addr + image.len() as u64;
    // guest_base: host pointer such that guest_base + vaddr
    // points to the right byte in `image`.
    let guest_base = image.as_ptr().wrapping_sub(base_addr as usize);

    let start_pc = args.start.unwrap_or(info.entry);
    let cfg = RiscvCfg::default();
    let max_count = args.count.unwrap_or(usize::MAX);

    let mut out: Box<dyn Write> = match &args.output {
        Some(path) => {
            let f = fs::File::create(path).unwrap_or_else(|e| {
                eprintln!("cannot create {path}: {e}");
                process::exit(1);
            });
            Box::new(BufWriter::new(f))
        }
        None => Box::new(BufWriter::new(io::stdout().lock())),
    };

    let mut ir = Context::new();
    let mut pc = start_pc;
    let mut tb_count = 0usize;

    while pc >= base_addr && pc < image_end && tb_count < max_count {
        writeln!(out, "TB #{tb_count} @ 0x{pc:x}").expect("write failed");
        let (next_pc, _) = translate_tb(
            &mut ir,
            pc,
            guest_base,
            cfg,
            args.max_insns,
            &mut out,
        );
        writeln!(out).expect("write failed");
        tb_count += 1;
        pc = next_pc;
    }
}
