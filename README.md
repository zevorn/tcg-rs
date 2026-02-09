<h1 align="center">tcg-rs</h1>
<p align="center">
  English | <a href="README.zh.md">中文</a>
</p>

A Rust reimplementation of [QEMU](https://www.qemu.org/)'s **TCG** (Tiny Code Generator) — the dynamic binary translation engine that converts guest architecture instructions into host machine code at runtime.

> **Status**: The complete translation pipeline is working — IR builder, liveness analysis, constraint-driven register allocator, and x86-64 codegen produce executable host code from TCG IR.

## Overview

tcg-rs aims to provide a clean, safe, and modular Rust implementation of QEMU's TCG subsystem. The project follows QEMU's proven architecture while leveraging Rust's type system, memory safety, and trait-based extensibility.

```
IR Builder (gen_*)  ->  Liveness Analysis  ->  RegAlloc + Codegen  ->  Execute
  ir_builder.rs          liveness.rs           regalloc.rs          translate.rs
                                               codegen.rs
```

## Crate Structure

| Crate | Status | Description |
|-------|--------|-------------|
| `tcg-core` | Implemented | IR definitions (opcodes, types, temps, ops, context, labels, TBs) + IR builder (`gen_*` methods) |
| `tcg-backend` | Implemented | Liveness analysis, constraint system, register allocator, x86-64 codegen, translation pipeline |
| `tcg-tests` | Implemented | 522 tests: unit, backend regression, and end-to-end integration |
| `tcg-opt` | Planned | IR optimizer: constant/copy propagation, DCE |
| `tcg-frontend` | Planned | Guest instruction decoding trait + per-arch decoders |
| `tcg-exec` | Planned | CPU execution loop, TB cache, TB linking/invalidation |
| `tcg-mmu` | Planned | Software TLB, guest memory access |
| `tcg-runtime` | Planned | Runtime helper functions called from generated code |

## Key Design Decisions

- **Unified type-polymorphic opcodes**: A single `Add` opcode works on both I32 and I64 (type carried in `Op::op_type`), reducing opcode count by ~40% compared to QEMU's split design.
- **Constraint-driven register allocation**: Declarative `ArgConstraint`/`OpConstraint` types mirror QEMU's `TCGArgConstraint` + `C_O*_I*` macro system. The allocator is fully generic — no per-opcode branches. Adding a new opcode requires only a constraint table entry.
- **Trait-based backends**: `HostCodeGen` trait (including `op_constraint()`) instead of conditional compilation, enabling multi-backend support and testability.
- **Minimal `unsafe`**: Confined to JIT code buffer (mmap/mprotect) and generated code execution. All IR manipulation is safe Rust.
- **`RegSet` as `u64` bitmap**: Register allocation hot path uses bit operations instead of collection types.

## Building

```bash
cargo build                  # Build all crates
cargo test                   # Run all 522 tests
cargo clippy -- -D warnings  # Lint check
cargo fmt --check            # Format check
```

## What's Implemented

### tcg-core

- **Type system**: `Type` (I32/I64/I128/V64/V128/V256), `Cond` (QEMU-compatible encoding), `MemOp` (bit-packed), `RegSet` (u64 bitmap)
- **Opcodes**: ~70 unified opcodes with static `OpDef` table, `OpFlags` for properties
- **Temporaries**: Five lifetime kinds (Ebb, Tb, Global, Fixed, Const) with register allocator state
- **Labels**: Forward reference support with back-patching via `LabelUse`/`RelocKind`
- **Operations**: `Op` with fixed-size args array, `LifeData` for liveness
- **Context**: Translation context with global preservation across `reset()`, constant deduplication
- **IR builder**: `gen_add/sub/mul/and/or/xor/shl/shr/sar/neg/not/mov/setcond/brcond/br/ld/st/exit_tb/goto_tb`
- **Translation blocks**: `TranslationBlock` with dual exit slots, `JumpCache` (4096-entry direct-mapped)

### tcg-backend

- **Constraint system** (`constraint.rs`): `ArgConstraint`/`OpConstraint` types with builder functions (`o1_i2_alias`, `o1_i2_alias_fixed`, `n1_i2`, etc.)
- **Liveness analysis** (`liveness.rs`): Backward pass computing dead/sync flags per arg
- **Register allocator** (`regalloc.rs`): Constraint-driven greedy allocator mirroring QEMU's `tcg_reg_alloc_op()` — alias reuse, forced eviction, post-input fixup
- **Translation pipeline** (`translate.rs`): `translate_and_execute()` chains liveness → regalloc+codegen → JIT execution
- **x86-64 backend**:
  - Full GPR instruction encoder (emitter.rs): arithmetic, shifts, data movement, memory, mul/div, bit ops, branches, setcc/cmovcc
  - Constraint table (constraints.rs): per-opcode register constraints aligned with QEMU's `tcg_target_op_def()`
  - Simplified codegen (codegen.rs): constraint guarantees eliminate all register juggling — each opcode emits minimal instructions
  - System V ABI prologue/epilogue with `TCG_AREG0 = RBP`
  - `exit_tb`, `goto_tb` (4-byte aligned for atomic patching), `goto_ptr`

### tcg-tests

- **Unit tests**: Core data structure APIs (types, opcodes, temps, labels, ops, context, TBs)
- **Backend regression**: x86-64 instruction encoding, codegen aliasing behavior
- **Integration tests**: End-to-end pipeline with minimal RISC-V CPU state — ALU ops, branches, loops, memory access, complex multi-op sequences

## QEMU Reference

This project references the following QEMU source files:

- `tcg/tcg.c` — Register allocator (`tcg_reg_alloc_op`) and codegen
- `tcg/tcg-op.c` — IR emission (`tcg_gen_*`)
- `tcg/optimize.c` — IR optimizer
- `tcg/i386/tcg-target.c.inc` — x86-64 backend + constraint table (`tcg_target_op_def`)
- `include/tcg/tcg.h` — `TCGArgConstraint`, `TCGTemp`, `TCGContext`
- `include/tcg/tcg-opc.h` — Opcode definitions

## Documentation

- [Design Document](docs/design.md) — Architecture, data structures, constraint system, translation pipeline
- [x86-64 Backend](docs/x86_64-backend.md) — Instruction encoder, constraint table, codegen dispatch
- [Coding Style](docs/coding-style.md) — Naming conventions, formatting rules

## License

[MIT](LICENSE)
