//! TCG disassembler framework.
//!
//! Provides per-architecture instruction disassembly, mirroring
//! QEMU's `disas/` subsystem. Each guest architecture implements
//! a `print_insn_*` entry point that decodes raw bytes at a given
//! PC and returns a human-readable string plus instruction length.

pub mod riscv;
