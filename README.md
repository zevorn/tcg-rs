<h1 align="center">tcg-rs</h1>
<p align="center">
  English | <a href="README.zh.md">中文</a>
</p>

A Rust reimplementation of [QEMU](https://www.qemu.org/)'s **TCG** (Tiny Code Generator) — the dynamic binary translation engine that converts guest architecture instructions into host machine code at runtime.

> **Status**: The complete translation pipeline is working end-to-end — RISC-V guest instructions are decoded via a decode-generated decoder, translated to TCG IR, optimized (constant folding, copy propagation, algebraic simplification), register-allocated, compiled to x86-64 machine code, and executed. MTTCG execution, direct TB chaining, and linux-user ELF loading with syscall emulation are operational. A differential testing framework validates correctness against QEMU.

## Overview

tcg-rs aims to provide a clean, safe, and modular Rust implementation of QEMU's TCG subsystem. The project follows QEMU's proven architecture while leveraging Rust's type system, memory safety, and trait-based extensibility.

```
┌──────────────┐    ┌───────────────┐    ┌──────────────┐    ┌───────────┐    ┌──────────┐    ┌──────────────────┐    ┌─────────┐
│ Guest Binary │───→│ Frontend      │───→│ IR Builder   │───→│ Optimizer │───→│ Liveness │───→│ RegAlloc+Codegen │───→│ Execute │
│ (RISC-V)     │    │ (decode       │    │ (gen_*)      │    │           │    │ Analysis │    │ (x86-64)         │    │ (JIT)   │
└──────────────┘    │  + trans_*)   │    └──────────────┘    └───────────┘    └──────────┘    └──────────────────┘    └─────────┘
                    └───────────────┘
                     tcg-frontend         tcg-core             tcg-backend     tcg-backend     tcg-backend             tcg-backend
```

## Crate Structure

| Crate | Status | Description |
|-------|--------|-------------|
| `tcg-core` | Implemented | IR definitions (opcodes, types, temps, ops, context, labels, TBs) + IR builder (`gen_*` methods) |
| `tcg-backend` | Implemented | IR optimizer, liveness analysis, constraint system, register allocator, x86-64 codegen, translation pipeline |
| `tcg-exec` | Implemented | MTTCG-capable execution loop, TB store, direct chaining, per-vCPU jump cache, execution stats |
| `tcg-linux-user` | Implemented | ELF loader, guest address space, Linux syscall emulation, `tcg-riscv64` runner |
| `decode` | Implemented | QEMU-style `.decode` file parser and Rust code generator for instruction decoders |
| `tcg-frontend` | Implemented | Guest instruction decoding framework + RISC-V RV64IMAFDC frontend (184 instructions) |
| `tcg-tests` | Implemented | 816 tests: unit, backend regression, frontend translation, difftest (vs QEMU), MTTCG, and linux-user e2e |

## Key Design Decisions

- **Unified type-polymorphic opcodes**: A single `Add` opcode works on both I32 and I64 (type carried in `Op::op_type`), reducing opcode count by ~40% compared to QEMU's split design.
- **Constraint-driven register allocation**: Declarative `ArgConstraint`/`OpConstraint` types mirror QEMU's `TCGArgConstraint` + `C_O*_I*` macro system. The allocator is fully generic — no per-opcode branches. Adding a new opcode requires only a constraint table entry.
- **Trait-based backends**: `HostCodeGen` trait (including `op_constraint()`) instead of conditional compilation, enabling multi-backend support and testability.
- **Minimal `unsafe`**: Confined to JIT code buffer (mmap/mprotect) and generated code execution. All IR manipulation is safe Rust.
- **`RegSet` as `u64` bitmap**: Register allocation hot path uses bit operations instead of collection types.

## Building

```bash
cargo build                  # Build all crates
cargo test                   # Run all 816 tests
cargo clippy -- -D warnings  # Lint check
cargo fmt --check            # Format check
```

## What's Implemented

### tcg-core

- **Type system**: `Type` (I32/I64/I128/V64/V128/V256), `Cond` (QEMU-compatible encoding), `MemOp` (bit-packed), `RegSet` (u64 bitmap)
- **Opcodes**: 158 unified opcodes with static `OpDef` table, `OpFlags` for properties
- **Temporaries**: Five lifetime kinds (Ebb, Tb, Global, Fixed, Const) with register allocator state
- **Labels**: Forward reference support with back-patching via `LabelUse`/`RelocKind`
- **Operations**: `Op` with fixed-size args array, `LifeData` for liveness
- **Context**: Translation context with global preservation across `reset()`, constant deduplication
- **IR builder**: `gen_add/sub/mul/and/or/xor/shl/shr/sar/neg/not/mov/setcond/brcond/br/ld/st/exit_tb/goto_tb`
- **Translation blocks**: `TranslationBlock` with dual exit slots, `JumpCache` (4096-entry direct-mapped)

### tcg-backend

- **IR optimizer** (`optimize.rs`): Single-pass optimizer running before liveness analysis — constant folding (unary, binary, type-conversion ops), copy propagation, algebraic simplification (identity/annihilator rules), same-operand identities, branch constant folding (BrCond → Br/Nop)
- **Constraint system** (`constraint.rs`): `ArgConstraint`/`OpConstraint` types with builder functions (`o1_i2_alias`, `o1_i2_alias_fixed`, `n1_i2`, etc.)
- **Liveness analysis** (`liveness.rs`): Backward pass computing dead/sync flags per arg
- **Register allocator** (`regalloc.rs`): Constraint-driven greedy allocator mirroring QEMU's `tcg_reg_alloc_op()` — alias reuse, forced eviction, post-input fixup
- **Translation pipeline** (`translate.rs`): `translate_and_execute()` chains optimize → liveness → regalloc+codegen → JIT execution
- **x86-64 backend**:
  - Full GPR instruction encoder (emitter.rs): arithmetic, shifts, data movement, memory, mul/div, bit ops, branches, setcc/cmovcc
  - Constraint table (constraints.rs): per-opcode register constraints aligned with QEMU's `tcg_target_op_def()`
  - Simplified codegen (codegen.rs): constraint guarantees eliminate all register juggling — each opcode emits minimal instructions
  - System V ABI prologue/epilogue with `TCG_AREG0 = RBP`
  - `exit_tb`, `goto_tb` (4-byte aligned for atomic patching), `goto_ptr`

### tcg-exec

- **MTTCG state split**: `SharedState` (TB store + code buffer + backend) and
  `PerCpuState` (jump cache + stats) for vCPU-local hot data.
- **Thread-safe TB store**: lock-free read path (`AtomicUsize` length publish),
  hash mutation lock, and per-TB jump lock for chaining edges.
- **Execution hot path**: jump-cache hit → hash hit → translate; supports
  `next_tb_hint`, direct chaining (`goto_tb` slots), and `exit_target` cache.
- **Debug observability**: `ExecStats` exposes lookup hit rate, chain patch
  counts, and hint usage; `TCG_STATS=1` prints runtime profile.

### tcg-linux-user

- **ELF loader + stack layout** aligned with linux-user model, including
  `argv` propagation and auxv essentials.
- **Guest space management** with mmap/brk handling for user-mode execution.
- **Syscall emulation** for core Linux user-mode workflows used by tests.
- **Runner**: `tcg-riscv64 <elf> [args...]`, shared by linux-user e2e tests.

### tcg-tests

- **Unit tests**: Core data structure APIs (types, opcodes, temps, labels, ops, context, TBs)
- **Backend regression**: x86-64 instruction encoding, codegen aliasing behavior
- **Frontend translation**: 91 RISC-V instruction tests through the full decode→IR→codegen→execute pipeline (RV32I/RV64I/RVC/RV32F/RV64F)
- **Difftest**: Differential testing framework comparing tcg-rs results against QEMU (qemu-riscv64 user-mode) with edge-case values
- **Integration tests**: End-to-end pipeline with minimal RISC-V CPU state — ALU ops, branches, loops, memory access, complex multi-op sequences
- **MTTCG tests**: concurrent lookup/translation/chaining tests under
  `tests/src/exec/mttcg.rs` (26 tests)
- **linux-user guest tests**: `hello`, `hello_printf`, `hello_float`,
  `dhrystone`, `argv_echo`

### decode

- **Parser**: Parses QEMU-style `.decode` files (fields, argument sets, formats, patterns with bit-level matching)
- **Code generator**: Emits Rust code — `Args*` structs, `extract_*` functions, `Decode<Ir>` trait with `trans_*` methods, and `decode()` dispatch function
- **Build integration**: `frontend/build.rs` invokes decode at compile time to generate the RISC-V instruction decoder

### tcg-frontend

- **Translation framework** (`lib.rs`): `TranslatorOps` trait and `translator_loop()` — architecture-independent instruction translation loop
- **RISC-V frontend** (`riscv/`):
  - `cpu.rs`: `RiscvCpu` state (`#[repr(C)]`, 32 GPRs + 32 FPRs + PC + float CSRs)
  - `mod.rs`: `RiscvDisasContext` with GPRs/FPRs as TCG globals, `RiscvTranslator` implementing `TranslatorOps`
  - `trans.rs`: 184 `trans_*` methods implementing `Decode<Context>` trait, using QEMU-style `gen_xxx` helper pattern with `BinOp` function pointers
  - Implemented: RV64I (full), RV64M (mul/div/rem), RV64F/RV64D (float arithmetic, load/store, conversions, comparisons, FMA), RVC (compressed), load/store (guest memory via helper calls), user-mode CSRs (fflags/frm/fcsr)

## QEMU Reference

This project references the following QEMU source files:

- `tcg/tcg.c` — Register allocator (`tcg_reg_alloc_op`) and codegen
- `tcg/tcg-op.c` — IR emission (`tcg_gen_*`)
- `tcg/optimize.c` — IR optimizer
- `tcg/i386/tcg-target.c.inc` — x86-64 backend + constraint table (`tcg_target_op_def`)
- `include/tcg/tcg.h` — `TCGArgConstraint`, `TCGTemp`, `TCGContext`
- `include/tcg/tcg-opc.h` — Opcode definitions
- `target/riscv/translate.c` — RISC-V frontend translation
- `target/riscv/insn_trans/trans_rvi.c.inc` — RV64I instruction translation helpers
- `accel/tcg/translator.c` — `translator_loop` (architecture-independent translation loop)
- `accel/tcg/cpu-exec.c` — execution loop, TB chaining, exit protocol
- `accel/tcg/tb-maint.c` — TB invalidation and unlinking
- `docs/devel/decodetree.rst` — Decodetree pattern-based instruction decoder generator (QEMU reference)
- `docs/devel/multi-thread-tcg.rst` — MTTCG concurrency model

## Documentation

- [Design Document](docs/design.md) — Architecture, data structures, translation pipeline, exec layer, linux-user
- [IR Ops](docs/ir-ops.md) — Opcode catalog, Op structure, IR builder API
- [x86-64 Backend](docs/x86_64-backend.md) — Instruction encoder, constraint table, codegen dispatch
- [Testing](docs/testing.md) — Test architecture, running tests, difftest framework, guest programs
- [Coding Style](docs/coding-style.md) — Naming conventions, formatting rules

## License

[MIT](LICENSE)
