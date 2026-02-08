# tcg-rs

A Rust reimplementation of [QEMU](https://www.qemu.org/)'s **TCG** (Tiny Code Generator) — the dynamic binary translation engine that converts guest architecture instructions into host machine code at runtime.

> **Status**: Early development. The core IR definitions and x86-64 backend initialization are implemented.

## Overview

tcg-rs aims to provide a clean, safe, and modular Rust implementation of QEMU's TCG subsystem. The project follows QEMU's proven architecture while leveraging Rust's type system, memory safety, and trait-based extensibility.

```
Guest Binary → Frontend (decode) → TCG IR → Optimizer → Backend (codegen) → Host Binary
                                      ↓
                              TranslationBlock Cache
```

## Crate Structure

| Crate | Status | Description |
|-------|--------|-------------|
| `tcg-core` | Implemented | IR definitions: opcodes, types, temps, ops, context, labels, translation blocks |
| `tcg-backend` | Implemented | Host code generation trait + x86-64 backend (prologue/epilogue, TB control flow) |
| `tcg-tests` | Implemented | 88 tests covering all public APIs |
| `tcg-ir` | Planned | IR generation API (`tcg_gen_*` equivalents) |
| `tcg-opt` | Planned | IR optimizer: constant/copy propagation, DCE |
| `tcg-frontend` | Planned | Guest instruction decoding trait + per-arch decoders |
| `tcg-exec` | Planned | CPU execution loop, TB cache, TB linking/invalidation |
| `tcg-mmu` | Planned | Software TLB, guest memory access |
| `tcg-runtime` | Planned | Runtime helper functions called from generated code |

## Key Design Decisions

- **Unified type-polymorphic opcodes**: A single `Add` opcode works on both I32 and I64 (type carried in `Op::op_type`), reducing opcode count by ~40% compared to QEMU's split design.
- **Trait-based backends**: `HostCodeGen` trait instead of conditional compilation, enabling multi-backend support and testability.
- **Minimal `unsafe`**: Confined to JIT code buffer (mmap/mprotect) and guest memory access. All IR manipulation is safe Rust.
- **Constant deduplication**: Per-type `HashMap` in `Context` avoids duplicate constant temps.
- **`RegSet` as `u64` bitmap**: Register allocation hot path uses bit operations instead of collection types.

## Building

```bash
cargo build                  # Build all crates
cargo test                   # Run all 88 tests
cargo clippy -- -D warnings  # Lint check
cargo fmt --check            # Format check
```

## What's Implemented

### tcg-core

- **Type system**: `Type` (I32/I64/I128/V64/V128/V256), `Cond` (with QEMU-compatible encoding), `MemOp` (bit-packed), `RegSet` (u64 bitmap)
- **Opcodes**: ~70 unified opcodes with static `OpDef` table, `OpFlags` for properties (INT, SIDE_EFFECTS, BB_EXIT, CARRY_IN/OUT, etc.)
- **Temporaries**: Five lifetime kinds (Ebb, Tb, Global, Fixed, Const) with register allocator state
- **Labels**: Forward reference support with back-patching via `LabelUse`/`RelocKind`
- **Operations**: `Op` with fixed-size args array, `LifeData` for liveness analysis
- **Context**: Translation context with global preservation across `reset()`, constant deduplication
- **Translation blocks**: `TranslationBlock` with dual exit slots, `JumpCache` (4096-entry direct-mapped)

### tcg-backend

- **CodeBuffer**: mmap-based JIT memory with W^X (Write XOR Execute) discipline
- **x86-64 backend**:
  - System V ABI prologue/epilogue with `TCG_AREG0 = RBP` (env pointer)
  - Dual epilogue entries: zero-return path + TB return path
  - `exit_tb`, `goto_tb` (4-byte aligned for atomic patching), `goto_ptr`
  - Stack frame: callee-saved registers + 128B call args + 1024B spill area

## QEMU Reference

This project references the following QEMU source files:

- `tcg/tcg.c`, `tcg/tcg-op.c` — Core codegen and IR emission
- `tcg/optimize.c` — IR optimizer
- `accel/tcg/cpu-exec.c` — Execution loop
- `tcg/i386/tcg-target.c.inc` — x86-64 backend
- `include/tcg/tcg-opc.h` — Opcode definitions

## Documentation

- [Design Document](docs/design.md) — Detailed architecture and design rationale (Chinese)

## License

[MIT](LICENSE)
