<h1 align="center">tcg-rs</h1>
<p align="center">
  <a href="README.md">English</a> | 中文
</p>

[QEMU](https://www.qemu.org/) **TCG**（Tiny Code Generator）的 Rust 重新实现——一个动态二进制翻译引擎，在运行时将客户架构指令转换为宿主机器码。

> **状态**：完整的翻译流水线已端到端可工作——RISC-V 客户指令通过 decode 生成的解码器解码，翻译为 TCG IR，经 IR 优化（常量折叠、拷贝传播、代数简化）和活跃性分析，寄存器分配后编译为 x86-64 机器码并执行。MTTCG 执行、直接 TB 链路和 linux-user ELF 加载与 syscall 仿真均已可用。差分测试框架可对比 QEMU 验证正确性。

## 概述

tcg-rs 旨在提供一个干净、安全、模块化的 QEMU TCG 子系统 Rust 实现。项目遵循 QEMU 经过验证的架构，同时利用 Rust 的类型系统、内存安全和基于 trait 的可扩展性。

```
┌──────────────┐    ┌───────────────┐    ┌──────────────┐    ┌───────────┐    ┌──────────┐    ┌──────────────────┐    ┌─────────┐
│ Guest Binary │───→│ Frontend      │───→│ IR Builder   │───→│ Optimizer │───→│ Liveness │───→│ RegAlloc+Codegen │───→│ Execute │
│ (RISC-V)     │    │ (decode       │    │ (gen_*)      │    │           │    │ Analysis │    │ (x86-64)         │    │ (JIT)   │
└──────────────┘    │  + trans_*)   │    └──────────────┘    └───────────┘    └──────────┘    └──────────────────┘    └─────────┘
                    └───────────────┘
                     tcg-frontend         tcg-core             tcg-backend     tcg-backend     tcg-backend             tcg-backend
```

## Crate 结构

| Crate | 状态 | 描述 |
|-------|------|------|
| `tcg-core` | 已实现 | IR 定义（opcodes、types、temps、ops、context、labels、TBs）+ IR 构建器（`gen_*` 方法） |
| `tcg-backend` | 已实现 | IR 优化器、活跃性分析、约束系统、寄存器分配器、x86-64 代码生成、翻译流水线 |
| `tcg-exec` | 已实现 | 支持 MTTCG 的执行循环、TB 存储、直接链路、每 vCPU 跳转缓存、执行统计 |
| `tcg-linux-user` | 已实现 | ELF 加载、guest 地址空间、Linux syscall 仿真、`tcg-riscv64` 运行器 |
| `decode` | 已实现 | QEMU 风格 `.decode` 文件解析器和 Rust 代码生成器，用于生成指令解码器 |
| `tcg-frontend` | 已实现 | 客户指令解码框架 + RISC-V RV64IMAFDC 前端（184 条指令） |
| `tcg-tests` | 已实现 | 816 个测试：单元、后端回归、前端翻译、difftest、MTTCG、linux-user 端到端 |

## 关键设计决策

- **统一类型多态 Opcodes**：单个 `Add` opcode 同时适用于 I32 和 I64（类型由 `Op::op_type` 携带），相比 QEMU 的分裂设计减少约 40% 的 opcode 数量。
- **约束驱动寄存器分配**：声明式 `ArgConstraint`/`OpConstraint` 类型对齐 QEMU 的 `TCGArgConstraint` + `C_O*_I*` 宏系统。分配器完全通用——无 per-opcode 分支。新增 opcode 只需添加约束表条目。
- **基于 Trait 的后端**：使用 `HostCodeGen` trait（包含 `op_constraint()`）而非条件编译，支持多后端和可测试性。
- **最小化 `unsafe`**：限制在 JIT 代码缓冲区（mmap/mprotect）和生成代码执行中。所有 IR 操作均为安全 Rust。
- **`RegSet` 使用 `u64` 位图**：寄存器分配热路径使用位操作而非集合类型。

## 构建

```bash
cargo build                  # 构建所有 crate
cargo test                   # 运行全部 816 个测试
cargo clippy -- -D warnings  # Lint 检查
cargo fmt --check            # 格式检查
```

## 已实现内容

### tcg-core

- **类型系统**：`Type`（I32/I64/I128/V64/V128/V256）、`Cond`（QEMU 兼容编码）、`MemOp`（位域打包）、`RegSet`（u64 位图）
- **Opcodes**：158 个统一 opcode，配有静态 `OpDef` 表和 `OpFlags` 属性标志
- **临时变量**：五种生命周期（Ebb、Tb、Global、Fixed、Const），包含寄存器分配器状态
- **标签**：支持前向引用，通过 `LabelUse`/`RelocKind` 进行 back-patching
- **操作**：`Op` 使用固定大小参数数组，`LifeData` 用于活跃性分析
- **上下文**：翻译上下文，`reset()` 时保留全局变量，支持常量去重
- **IR 构建器**：`gen_add/sub/mul/and/or/xor/shl/shr/sar/neg/not/mov/setcond/brcond/br/ld/st/exit_tb/goto_tb`
- **翻译块**：`TranslationBlock` 双出口设计，`JumpCache`（4096 项直接映射缓存）

### tcg-backend

- **IR 优化器**（`optimize.rs`）：在活跃性分析前运行的单遍优化器——常量折叠（一元、二元、类型转换）、拷贝传播、代数简化（恒等/零化规则）、同操作数恒等式、分支常量折叠（BrCond → Br/Nop）
- **约束系统**（`constraint.rs`）：`ArgConstraint`/`OpConstraint` 类型及构建函数（`o1_i2_alias`、`o1_i2_alias_fixed`、`n1_i2` 等）
- **活跃性分析**（`liveness.rs`）：反向遍历计算每个参数的 dead/sync 标志
- **寄存器分配器**（`regalloc.rs`）：约束驱动贪心分配器，对齐 QEMU 的 `tcg_reg_alloc_op()`——别名复用、强制驱逐、输入后修正
- **翻译流水线**（`translate.rs`）：`translate_and_execute()` 串联 optimize → liveness → regalloc+codegen → JIT 执行
- **x86-64 后端**：
  - 完整 GPR 指令编码器（emitter.rs）：算术、移位、数据移动、内存、乘除、位操作、分支、setcc/cmovcc
  - 约束表（constraints.rs）：per-opcode 寄存器约束，对齐 QEMU 的 `tcg_target_op_def()`
  - 简化 codegen（codegen.rs）：约束保证消除所有寄存器杂耍——每个 opcode 发射最少指令
  - System V ABI prologue/epilogue，`TCG_AREG0 = RBP`
  - `exit_tb`、`goto_tb`（4 字节对齐用于原子修补）、`goto_ptr`

### tcg-exec

- **MTTCG 状态拆分**：`SharedState`（TB store + code buffer + backend）与
  `PerCpuState`（jump cache + stats），隔离多线程共享与热路径私有数据。
- **线程安全 TB 存储**：读路径无锁、hash 修改加锁、每 TB 链路状态独立锁。
- **执行热路径**：jump-cache 命中 → hash 命中 → translate，配合
  `next_tb_hint`、`goto_tb` 链路 patch、`exit_target` 缓存。
- **可观测性**：`ExecStats` 提供命中率、链路 patch 次数、hint 命中统计。

### tcg-linux-user

- **ELF 加载与栈布局**：支持 guest argv 透传和基础 auxv 布局。
- **guest 地址空间管理**：覆盖 mmap/brk 等用户态执行所需内存管理路径。
- **syscall 仿真**：提供 linux-user 基础系统调用处理。
- **运行器**：`tcg-riscv64 <elf> [args...]`，用于端到端 guest 运行测试。

### tcg-tests

- **单元测试**：核心数据结构 API（types、opcodes、temps、labels、ops、context、TBs）
- **后端回归测试**：x86-64 指令编码、codegen 别名行为
- **前端翻译测试**：91 个 RISC-V 指令测试，覆盖完整的 decode→IR→codegen→execute 流水线（RV32I/RV64I/RVC/RV32F/RV64F）
- **差分测试**：对比 tcg-rs 与 QEMU（qemu-riscv64 用户态）的指令模拟结果，使用边界值验证
- **集成测试**：使用最小 RISC-V CPU 状态的端到端流水线——ALU 运算、分支、循环、内存访问、复杂多操作序列
- **MTTCG 并发测试**：`tests/src/exec/mttcg.rs` 覆盖并发查找/翻译/链路场景（26 个测试）
- **linux-user guest 测试**：`hello`、`hello_printf`、`hello_float`、
  `dhrystone`、`argv_echo`

### decode

- **解析器**：解析 QEMU 风格的 `.decode` 文件（字段、参数集、格式、位级匹配模式）
- **代码生成器**：生成 Rust 代码——`Args*` 结构体、`extract_*` 函数、`Decode<Ir>` trait 及 `trans_*` 方法、`decode()` 分派函数
- **构建集成**：`frontend/build.rs` 在编译时调用 decode 生成 RISC-V 指令解码器

### tcg-frontend

- **翻译框架**（`lib.rs`）：`TranslatorOps` trait 和 `translator_loop()`——架构无关的指令翻译循环
- **RISC-V 前端**（`riscv/`）：
  - `cpu.rs`：`RiscvCpu` 状态（`#[repr(C)]`，32 个 GPR + 32 个 FPR + PC + 浮点 CSR）
  - `mod.rs`：`RiscvDisasContext`，GPR/FPR 作为 TCG 全局变量，`RiscvTranslator` 实现 `TranslatorOps`
  - `trans.rs`：184 个 `trans_*` 方法实现 `Decode<Context>` trait，使用 QEMU 风格的 `gen_xxx` 辅助函数模式和 `BinOp` 函数指针
  - 已实现：RV64I（完整）、RV64M（mul/div/rem）、RV64F/RV64D（浮点算术、load/store、类型转换、比较、FMA）、RVC（压缩指令）、load/store（通过 helper 调用访问客户内存）、用户态 CSR（fflags/frm/fcsr）

## QEMU 参考

本项目参考以下 QEMU 源文件：

- `tcg/tcg.c` — 寄存器分配器（`tcg_reg_alloc_op`）和代码生成
- `tcg/tcg-op.c` — IR 发射（`tcg_gen_*`）
- `tcg/optimize.c` — IR 优化器
- `tcg/i386/tcg-target.c.inc` — x86-64 后端 + 约束表（`tcg_target_op_def`）
- `include/tcg/tcg.h` — `TCGArgConstraint`、`TCGTemp`、`TCGContext`
- `include/tcg/tcg-opc.h` — Opcode 定义
- `target/riscv/translate.c` — RISC-V 前端翻译
- `target/riscv/insn_trans/trans_rvi.c.inc` — RV64I 指令翻译辅助函数
- `accel/tcg/translator.c` — `translator_loop`（架构无关的翻译循环）
- `accel/tcg/cpu-exec.c` — 执行循环、TB 链路与退出协议
- `accel/tcg/tb-maint.c` — TB 失效与解链
- `docs/devel/decodetree.rst` — Decodetree 基于模式的指令解码器生成器（QEMU 参考）
- `docs/devel/multi-thread-tcg.rst` — MTTCG 并发模型

## 文档

- [设计文档](docs/design.md) — 架构、数据结构、翻译流水线、执行层、linux-user
- [IR Ops](docs/ir-ops.md) — Opcode 目录、Op 结构、IR Builder API
- [x86-64 后端](docs/x86_64-backend.md) — 指令编码器、约束表、codegen 分派
- [测试体系](docs/testing.md) — 测试架构、运行方式、差分测试、客户程序
- [代码风格](docs/coding-style.md) — 命名规范、格式规则

## 许可证

[MIT](LICENSE)
