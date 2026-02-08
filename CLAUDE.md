# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

tcg-rs is a Rust reimplementation of QEMU's TCG (Tiny Code Generator) — the dynamic binary translation engine that converts guest architecture instructions into host machine code at runtime. The reference implementation lives at `~/qemu/tcg/`, `~/qemu/accel/tcg/`, and `~/qemu/include/tcg/`.

## Build & Development Commands

```bash
cargo build                          # Build all crates
cargo build --release                # Release build
cargo test                           # Run all tests
cargo test -p tcg-core               # Test a single crate
cargo test -- test_name              # Run a specific test
cargo clippy -- -D warnings          # Lint
cargo fmt --check                    # Check formatting
cargo fmt                            # Auto-format
cargo doc --open                     # Generate and open docs
```

## Architecture

### Translation Pipeline

```
Guest Binary → Frontend (decode) → TCG IR → Optimizer → Backend (codegen) → Host Binary
                                      ↓
                              TranslationBlock Cache
```

### Crate Structure

| Crate | Responsibility | QEMU Reference |
|-------|---------------|----------------|
| `tcg-core` | IR definitions: opcodes, types, temps, TCGOp, TCGContext, labels | `include/tcg/tcg.h`, `tcg/tcg-opc.h`, `tcg/tcg-common.c` |
| `tcg-ir` | IR generation API (`tcg_gen_*` equivalents), op emission | `tcg/tcg-op.c`, `tcg/tcg-op-ldst.c`, `tcg/tcg-op-vec.c`, `tcg/tcg-op-gvec.c` |
| `tcg-opt` | IR optimizer: constant/copy propagation, DCE, algebraic simplification | `tcg/optimize.c` |
| `tcg-backend` | Host code generation trait + per-arch backends | `tcg/tcg.c` (codegen parts), `tcg/<arch>/tcg-target.c.inc` |
| `tcg-frontend` | Guest instruction decoding trait + per-arch decoders | `target/<arch>/translate.c`, `accel/tcg/translator.c` |
| `tcg-exec` | CPU execution loop, TB cache (jump cache + hash table), TB linking/invalidation | `accel/tcg/cpu-exec.c`, `accel/tcg/translate-all.c`, `accel/tcg/tb-maint.c` |
| `tcg-mmu` | Software TLB, guest memory access (fast/slow path) | `accel/tcg/cputlb.c` |
| `tcg-runtime` | Runtime helper functions called from generated code | `accel/tcg/tcg-runtime.c`, `accel/tcg/tcg-runtime-gvec.c` |

### Key Data Structures (C → Rust Mapping)

| QEMU C Struct | Rust Equivalent | Purpose |
|---------------|----------------|---------|
| `TCGOpcode` (DEF macro enum) | `enum Opcode` | ~150 IR opcodes (arithmetic, logic, memory, control flow, vector) |
| `TCGType` | `enum Type { I32, I64, I128, V64, V128, V256 }` | IR value types |
| `TCGTemp` | `struct Temp` | IR variable (global, local, const, fixed-reg) |
| `TCGTempKind` | `enum TempKind { Ebb, Tb, Global, Fixed, Const }` | Variable lifetime/scope |
| `TCGOp` | `struct Op` | Single IR operation with opcode + args |
| `TCGContext` | `struct Context` | Per-thread translation state: temps, ops list, code buffer, register allocator |
| `TCGLabel` | `struct Label` | Branch target within a TB |
| `TranslationBlock` | `struct TranslationBlock` | Cached translated code block: guest PC → host code mapping |
| `CPUJumpCache` | Per-CPU direct-mapped TB cache | 4096-entry fast lookup by PC hash |
| `TBContext.htable` | Global TB hash table | 32768-bucket lookup by (phys_pc, pc, flags) |
| `TCGCond` | `enum Cond { Eq, Ne, Lt, Ge, Ltu, Geu, ... }` | Comparison conditions |
| `MemOp` | `enum MemOp` | Memory access size/signedness/endianness/alignment |

### Translation Block Lifecycle

1. **Lookup**: PC hash → jump cache (per-CPU, 4096 entries) → global hash table (32K buckets)
2. **Miss → Translate**: Frontend decodes guest instructions → emits TCG IR → optimizer runs → backend generates host code
3. **Cache**: Insert into hash table and jump cache
4. **Execute**: Jump to generated host code
5. **Link**: Patch direct jumps between TBs (`goto_tb`/`exit_tb` for direct branches, `lookup_and_goto_ptr` for indirect)
6. **Invalidate**: On self-modifying code, page unmap, or cache full — unlink and remove

### Frontend Trait Design

Each guest architecture implements a decoder trait:

```rust
trait GuestDecoder {
    type Context: DisasContext;
    fn decode_insn(ctx: &mut Self::Context, insn: u32) -> DecodeResult;
    fn translate_insn(ctx: &mut Self::Context, ir: &mut IrBuilder) -> TranslateResult;
}
```

Reference: `~/qemu/accel/tcg/translator.c` (`translator_loop`) and `~/qemu/target/riscv/translate.c`.

### Backend Trait Design

Each host architecture implements a code generator trait:

```rust
trait HostCodeGen {
    fn emit_op(&mut self, op: &Op, buf: &mut CodeBuffer);
    fn emit_prologue(&mut self, buf: &mut CodeBuffer);
    fn emit_epilogue(&mut self, buf: &mut CodeBuffer);
    fn patch_jump(&mut self, jump_site: usize, target: usize);
}
```

Reference: `~/qemu/tcg/<arch>/tcg-target.c.inc` and `~/qemu/tcg/<arch>/tcg-target.h`.

### Unsafe Boundaries

`unsafe` is acceptable only in:
- JIT code buffer allocation and execution (mmap + mprotect RWX transitions)
- Calling into generated host code (`fn()` pointer cast from code buffer)
- Raw pointer access for guest memory simulation (TLB fast path)
- Inline assembly in backend code emitters
- FFI if interfacing with external libraries

All other code must be safe Rust.

## QEMU Reference Paths

Key source files for understanding the original implementation:

- **TCG core**: `~/qemu/tcg/tcg.c` (codegen + register allocator), `~/qemu/tcg/tcg-op.c` (IR emission)
- **Optimizer**: `~/qemu/tcg/optimize.c` (z_mask/o_mask/s_mask bit tracking, constant folding, copy propagation)
- **Execution loop**: `~/qemu/accel/tcg/cpu-exec.c` (TB lookup → execute → link cycle)
- **TB management**: `~/qemu/accel/tcg/translate-all.c`, `~/qemu/accel/tcg/tb-maint.c`
- **Software TLB**: `~/qemu/accel/tcg/cputlb.c` (fast path inline, slow path helper)
- **Opcodes**: `~/qemu/include/tcg/tcg-opc.h` (DEF macro list of all IR ops)
- **Documentation**: `~/qemu/docs/devel/tcg.rst`, `tcg-ops.rst`, `multi-thread-tcg.rst`
- **Backend example**: `~/qemu/tcg/aarch64/`, `~/qemu/tcg/i386/`, `~/qemu/tcg/riscv/`
- **Frontend example**: `~/qemu/target/riscv/translate.c`, `~/qemu/target/arm/tcg/translate.c`
- **Decodetree**: `~/qemu/docs/devel/decodetree.rst` (pattern-based instruction decoder generator)

## Design Principles

- **No backward compatibility**: break freely, clean up aggressively, no migration shims.
- **Trait-based extensibility**: frontends and backends are trait implementations, not conditional compilation.
- **Arena allocation for IR**: TCG ops form a linked list per TB — use an arena allocator (e.g., `bumpalo` or typed-arena) instead of malloc chains.
- **Enum-driven opcodes**: replace C's `DEF()` macro pattern with a proper Rust enum with `#[repr(u8)]`.
- **Type-safe IR builder**: the `tcg_gen_*` API should use Rust's type system to prevent mixing I32/I64 operands at compile time.
- **Minimal `unsafe`**: confine to JIT execution and guest memory access; everything else safe.
