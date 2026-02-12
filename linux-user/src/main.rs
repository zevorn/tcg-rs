use std::env;
use std::process;

use tcg_backend::X86_64CodeGen;
use tcg_core::context::Context;
use tcg_core::tb::{EXCP_EBREAK, EXCP_ECALL, EXCP_UNDEF};
use tcg_core::TempIdx;
use tcg_exec::exec_loop::{cpu_exec_loop, ExitReason};
use tcg_exec::{ExecEnv, GuestCpu};
use tcg_frontend::riscv::cpu::{RiscvCpu, NUM_GPRS};
use tcg_frontend::riscv::{RiscvDisasContext, RiscvTranslator};
use tcg_frontend::{translator_loop, DisasJumpType, TranslatorOps};
use tcg_linux_user::guest_space::GuestSpace;
use tcg_linux_user::loader::{load_elf, ElfInfo};
use tcg_linux_user::syscall::{handle_syscall, SyscallResult};

/// Wrapper: RiscvCpu + guest_base for GuestCpu trait.
struct LinuxCpu {
    cpu: RiscvCpu,
}

impl GuestCpu for LinuxCpu {
    fn get_pc(&self) -> u64 {
        self.cpu.pc
    }

    fn get_flags(&self) -> u32 {
        0
    }

    fn gen_code(&mut self, ir: &mut Context, pc: u64, max_insns: u32) -> u32 {
        let base = self.cpu.guest_base as *const u8;
        if ir.nb_globals() == 0 {
            let mut d = RiscvDisasContext::new(pc, base);
            d.base.max_insns = max_insns;
            translator_loop::<RiscvTranslator>(&mut d, ir);
            d.base.num_insns * 4
        } else {
            let mut d = RiscvDisasContext::new(pc, base);
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
            d.base.num_insns * 4
        }
    }

    fn env_ptr(&mut self) -> *mut u8 {
        &mut self.cpu as *mut RiscvCpu as *mut u8
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: tcg-riscv64 <elf> [args...]");
        process::exit(1);
    }

    let elf_path =
        std::fs::canonicalize(&args[1]).expect("failed to resolve elf path");
    let elf_path = elf_path.to_str().unwrap();
    let guest_argv: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();

    // Load ELF
    let mut space = GuestSpace::new().expect("failed to create guest space");
    let info: ElfInfo =
        load_elf(std::path::Path::new(elf_path), &mut space, &guest_argv, &[])
            .expect("failed to load ELF");

    // Set up CPU
    let mut lcpu = LinuxCpu {
        cpu: RiscvCpu::new(),
    };
    lcpu.cpu.pc = info.entry;
    lcpu.cpu.gpr[2] = info.sp; // SP = x2
    lcpu.cpu.guest_base = space.guest_base() as u64;

    // mmap_next starts after brk
    let mut mmap_next =
        tcg_linux_user::guest_space::page_align_up(info.brk) + 0x1000_0000; // 256 MB gap

    // Run
    let show_stats = env::var("TCG_STATS").is_ok();
    let mut env = ExecEnv::new(X86_64CodeGen::new());
    loop {
        let reason = unsafe { cpu_exec_loop(&mut env, &mut lcpu) };
        match reason {
            ExitReason::Exit(v) if v == EXCP_ECALL as usize => {
                // ECALL
                match handle_syscall(
                    &mut space,
                    &mut lcpu.cpu.gpr,
                    &mut mmap_next,
                    elf_path,
                ) {
                    SyscallResult::Continue(ret) => {
                        lcpu.cpu.gpr[10] = ret;
                        lcpu.cpu.pc += 4; // skip past ECALL
                    }
                    SyscallResult::Exit(code) => {
                        if show_stats {
                            eprint!("{}", env.per_cpu.stats);
                        }
                        process::exit(code);
                    }
                }
            }
            ExitReason::Exit(v) if v == EXCP_EBREAK as usize => {
                if show_stats {
                    eprint!("{}", env.per_cpu.stats);
                }
                eprintln!("ebreak at pc={:#x}", lcpu.cpu.pc);
                process::exit(1);
            }
            ExitReason::Exit(v) if v == EXCP_UNDEF as usize => {
                if show_stats {
                    eprint!("{}", env.per_cpu.stats);
                }
                eprintln!("illegal instruction at pc={:#x}", lcpu.cpu.pc);
                process::exit(1);
            }
            ExitReason::Exit(v) => {
                if show_stats {
                    eprint!("{}", env.per_cpu.stats);
                }
                eprintln!("unexpected exit {v}");
                process::exit(1);
            }
            ExitReason::BufferFull => {
                if show_stats {
                    eprint!("{}", env.per_cpu.stats);
                }
                eprintln!("code buffer full");
                process::exit(1);
            }
        }
    }
}
