# CLAUDE.md

本文件为 Claude Code (claude.ai/code) 在本仓库中工作时提供指导。

## 项目概述

tcg-rs 是 QEMU TCG（Tiny Code Generator）的 Rust 重新实现——一个动态二进制翻译引擎，在运行时将客户架构指令转换为宿主机器码。参考实现位于 `~/qemu/tcg/`、`~/qemu/accel/tcg/` 和 `~/qemu/include/tcg/`。

## 构建与开发命令

```bash
cargo build                          # 构建所有 crate
cargo build --release                # Release 构建
cargo test                           # 运行所有测试
cargo test -p tcg-core               # 测试单个 crate
cargo test -- test_name              # 运行指定测试
cargo clippy -- -D warnings          # Lint 检查
cargo fmt --check                    # 格式检查
cargo fmt                            # 自动格式化
cargo doc --open                     # 生成并打开文档
```

## Git Commit 规范

Commit message 必须使用英文编写。格式如下：

```
module: subject

具体修改内容的详细说明。

Signed-off-by: Name <email>
```

**Subject 行规则**：

- 格式为 `module: subject`，其中 `module` 是受影响的主要模块名
- 常用 module 名：`tcg-core`、`tcg-backend`、`tcg-tests`、`docs`、`project`（跨模块变更）
- subject 使用小写开头，祈使语气（如 `add`、`fix`、`remove`），不加句号
- 总长度不超过 72 字符

**Body 规则**：

- 与 subject 之间空一行
- 说明本次变更的内容和原因（what & why），而非如何实现（how）
- 每行不超过 72 字符

**示例**：

```
tcg-core: add vector opcode support

Add V64/V128/V256 vector opcodes to the unified opcode enum.
Each vector op carries OpFlags::VECTOR for backend dispatch.

Signed-off-by: Chao Liu <chao.liu.zevorn@gmail.com>
```

## 架构

### 翻译流水线

```
Guest Binary → Frontend (decode) → TCG IR → Optimizer → Backend (codegen) → Host Binary
                                      ↓
                              TranslationBlock Cache
```

### Crate 结构

| Crate | 职责 | QEMU 参考 |
|-------|------|----------|
| `tcg-core` | IR 定义：opcodes、types、temps、TCGOp、TCGContext、labels | `include/tcg/tcg.h`、`tcg/tcg-opc.h`、`tcg/tcg-common.c` |
| `tcg-ir` | IR 生成 API（`tcg_gen_*` 等价物），op 发射 | `tcg/tcg-op.c`、`tcg/tcg-op-ldst.c`、`tcg/tcg-op-vec.c`、`tcg/tcg-op-gvec.c` |
| `tcg-opt` | IR 优化器：常量/拷贝传播、DCE、代数化简 | `tcg/optimize.c` |
| `tcg-backend` | 宿主代码生成 trait + 各架构后端 | `tcg/tcg.c`（codegen 部分）、`tcg/<arch>/tcg-target.c.inc` |
| `tcg-frontend` | 客户指令解码 trait + 各架构解码器 | `target/<arch>/translate.c`、`accel/tcg/translator.c` |
| `tcg-exec` | CPU 执行循环、TB 缓存（jump cache + hash table）、TB 链接/失效 | `accel/tcg/cpu-exec.c`、`accel/tcg/translate-all.c`、`accel/tcg/tb-maint.c` |
| `tcg-mmu` | 软件 TLB、客户内存访问（快速/慢速路径） | `accel/tcg/cputlb.c` |
| `tcg-runtime` | 生成代码调用的运行时辅助函数 | `accel/tcg/tcg-runtime.c`、`accel/tcg/tcg-runtime-gvec.c` |

### 核心数据结构（C → Rust 映射）

| QEMU C 结构 | Rust 等价物 | 用途 |
|-------------|------------|------|
| `TCGOpcode`（DEF 宏枚举） | `enum Opcode` | ~70 个统一多态 IR opcodes |
| `TCGType` | `enum Type { I32, I64, I128, V64, V128, V256 }` | IR 值类型 |
| `TCGTemp` | `struct Temp` | IR 变量（global、local、const、fixed-reg） |
| `TCGTempKind` | `enum TempKind { Ebb, Tb, Global, Fixed, Const }` | 变量生命周期/作用域 |
| `TCGOp` | `struct Op` | 单个 IR 操作（opcode + args） |
| `TCGContext` | `struct Context` | 每线程翻译状态：temps、ops 列表、代码缓冲区、寄存器分配器 |
| `TCGLabel` | `struct Label` | TB 内的分支目标 |
| `TranslationBlock` | `struct TranslationBlock` | 缓存的翻译代码块：guest PC → host code 映射 |
| `CPUJumpCache` | `struct JumpCache` | 每 CPU 直接映射 TB 缓存，4096 项，按 PC 哈希查找 |
| `TBContext.htable` | 全局 TB 哈希表 | 32768 桶，按 (phys_pc, pc, flags) 查找 |
| `TCGCond` | `enum Cond { Eq, Ne, Lt, Ge, Ltu, Geu, ... }` | 比较条件 |
| `MemOp` | `struct MemOp(u16)` | 内存访问大小/符号/字节序/对齐 |

### 翻译块生命周期

1. **查找**：PC 哈希 → jump cache（每 CPU，4096 项）→ 全局哈希表（32K 桶）
2. **未命中 → 翻译**：前端解码客户指令 → 发射 TCG IR → 优化器运行 → 后端生成宿主代码
3. **缓存**：插入哈希表和 jump cache
4. **执行**：跳转到生成的宿主代码
5. **链接**：修补 TB 间的直接跳转（`goto_tb`/`exit_tb` 用于直接分支，`lookup_and_goto_ptr` 用于间接分支）
6. **失效**：自修改代码、页面取消映射或缓存满时——解链并移除

### 前端 Trait 设计

每个客户架构实现一个解码器 trait：

```rust
trait GuestDecoder {
    type Context: DisasContext;
    fn decode_insn(ctx: &mut Self::Context, insn: u32) -> DecodeResult;
    fn translate_insn(ctx: &mut Self::Context, ir: &mut IrBuilder) -> TranslateResult;
}
```

参考：`~/qemu/accel/tcg/translator.c`（`translator_loop`）和 `~/qemu/target/riscv/translate.c`。

### 后端 Trait 设计

每个宿主架构实现一个代码生成器 trait：

```rust
trait HostCodeGen {
    fn emit_prologue(&mut self, buf: &mut CodeBuffer);
    fn emit_epilogue(&mut self, buf: &mut CodeBuffer);
    fn patch_jump(&mut self, buf: &mut CodeBuffer, jump_offset: usize, target_offset: usize);
    fn epilogue_offset(&self) -> usize;
    fn init_context(&self, ctx: &mut Context);
}
```

参考：`~/qemu/tcg/<arch>/tcg-target.c.inc` 和 `~/qemu/tcg/<arch>/tcg-target.h`。

### x86-64 后端实现

x86-64 后端位于 `tcg-backend/src/x86_64/`，包含三个文件：

| 文件 | 职责 |
|------|------|
| `regs.rs` | 寄存器定义、ABI 常量（TCG_AREG0=RBP、栈帧布局） |
| `emitter.rs` | 指令编码器：前缀标志、操作码常量、核心编码函数、所有 GPR 指令发射器 |
| `mod.rs` | 模块导出 |

**编码架构**：采用 QEMU 风格的 `u32` 操作码常量，高位编码前缀标志（`P_EXT`、`P_REXW` 等），通过 `emit_opc` 统一处理 REX 前缀和转义字节。详见 [`docs/x86_64-backend.md`](docs/x86_64-backend.md)。

**已实现的指令类别**：算术（ADD/SUB/AND/OR/XOR/CMP/ADC/SBB）、移位（SHL/SHR/SAR/ROL/ROR/SHLD/SHRD）、数据移动（MOV/MOVZX/MOVSX/BSWAP）、内存（load/store/LEA 含 SIB 寻址）、乘除（MUL/IMUL/DIV/IDIV/CDQ/CQO）、位操作（BSF/BSR/LZCNT/TZCNT/POPCNT/BT*/ANDN）、分支（JMP/Jcc/CALL/SETcc/CMOVcc）、杂项（XCHG/PUSH/POP/INC/DEC/TEST/MFENCE/UD2/NOP）。

**未实现**：SIMD/向量指令（SSE/AVX/AVX512）将作为后续独立工作。

### Unsafe 边界

`unsafe` 仅在以下场景允许使用：

- JIT 代码缓冲区分配和执行（mmap + mprotect RWX 转换）
- 调用生成的宿主代码（从代码缓冲区进行 `fn()` 指针转换）
- 客户内存模拟的原始指针访问（TLB 快速路径）
- 后端代码发射器中的内联汇编
- 与外部库的 FFI 接口

所有其他代码必须是安全的 Rust。

## QEMU 参考路径

理解原始实现的关键源文件：

- **TCG 核心**：`~/qemu/tcg/tcg.c`（codegen + 寄存器分配器）、`~/qemu/tcg/tcg-op.c`（IR 发射）
- **优化器**：`~/qemu/tcg/optimize.c`（z_mask/o_mask/s_mask 位追踪、常量折叠、拷贝传播）
- **执行循环**：`~/qemu/accel/tcg/cpu-exec.c`（TB 查找 → 执行 → 链接循环）
- **TB 管理**：`~/qemu/accel/tcg/translate-all.c`、`~/qemu/accel/tcg/tb-maint.c`
- **软件 TLB**：`~/qemu/accel/tcg/cputlb.c`（快速路径内联、慢速路径辅助函数）
- **Opcodes**：`~/qemu/include/tcg/tcg-opc.h`（所有 IR ops 的 DEF 宏列表）
- **文档**：`~/qemu/docs/devel/tcg.rst`、`tcg-ops.rst`、`multi-thread-tcg.rst`
- **后端示例**：`~/qemu/tcg/aarch64/`、`~/qemu/tcg/i386/`、`~/qemu/tcg/riscv/`
- **前端示例**：`~/qemu/target/riscv/translate.c`、`~/qemu/target/arm/tcg/translate.c`
- **Decodetree**：`~/qemu/docs/devel/decodetree.rst`（基于模式的指令解码器生成器）

## 代码风格

代码行宽不超过 **80 列**。详细规范见 [`docs/coding-style.md`](docs/coding-style.md)。

核心规则：

- 缩进使用 4 个空格，禁止 Tab
- 代码行宽上限 80 列，注释和文档同样遵守
- 运行 `cargo fmt` 格式化，`cargo clippy -- -D warnings` 零警告
- 注释使用英文，仅在关键逻辑处添加
- 常量命名：QEMU 风格的操作码常量允许 `non_upper_case_globals`
- `unsafe` 仅限 JIT 执行和客户内存访问

## 设计原则

- **不向后兼容**：自由破坏、积极清理，不做迁移垫片。
- **基于 Trait 的可扩展性**：前端和后端是 trait 实现，而非条件编译。
- **IR 的 Arena 分配**：TCG ops 在每个 TB 中形成链表——使用 arena 分配器（如 `bumpalo` 或 typed-arena）替代 malloc 链。
- **枚举驱动的 Opcodes**：用带 `#[repr(u8)]` 的 Rust 枚举替代 C 的 `DEF()` 宏模式。
- **类型安全的 IR 构建器**：`tcg_gen_*` API 应利用 Rust 的类型系统在编译期防止混用 I32/I64 操作数。
- **最小化 `unsafe`**：限制在 JIT 执行和客户内存访问中；其他一切使用安全 Rust。
