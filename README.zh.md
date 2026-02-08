# tcg-rs

[QEMU](https://www.qemu.org/) **TCG**（Tiny Code Generator）的 Rust 重新实现——一个动态二进制翻译引擎，在运行时将客户架构指令转换为宿主机器码。

> **状态**：早期开发阶段。核心 IR 定义和 x86-64 后端初始化已实现。

[English](README.md) | 中文

## 概述

tcg-rs 旨在提供一个干净、安全、模块化的 QEMU TCG 子系统 Rust 实现。项目遵循 QEMU 经过验证的架构，同时利用 Rust 的类型系统、内存安全和基于 trait 的可扩展性。

```
Guest Binary → Frontend (decode) → TCG IR → Optimizer → Backend (codegen) → Host Binary
                                      ↓
                              TranslationBlock Cache
```

## Crate 结构

| Crate | 状态 | 描述 |
|-------|------|------|
| `tcg-core` | 已实现 | IR 定义：opcodes、types、temps、ops、context、labels、翻译块 |
| `tcg-backend` | 已实现 | 宿主代码生成 trait + x86-64 后端（prologue/epilogue、TB 控制流） |
| `tcg-tests` | 已实现 | 88 个测试覆盖所有公共 API |
| `tcg-ir` | 计划中 | IR 生成 API（`tcg_gen_*` 等价物） |
| `tcg-opt` | 计划中 | IR 优化器：常量/拷贝传播、DCE |
| `tcg-frontend` | 计划中 | 客户指令解码 trait + 各架构解码器 |
| `tcg-exec` | 计划中 | CPU 执行循环、TB 缓存、TB 链接/失效 |
| `tcg-mmu` | 计划中 | 软件 TLB、客户内存访问 |
| `tcg-runtime` | 计划中 | 生成代码调用的运行时辅助函数 |

## 关键设计决策

- **统一类型多态 Opcodes**：单个 `Add` opcode 同时适用于 I32 和 I64（类型由 `Op::op_type` 携带），相比 QEMU 的分裂设计减少约 40% 的 opcode 数量。
- **基于 Trait 的后端**：使用 `HostCodeGen` trait 而非条件编译，支持多后端和可测试性。
- **最小化 `unsafe`**：限制在 JIT 代码缓冲区（mmap/mprotect）和客户内存访问中。所有 IR 操作均为安全 Rust。
- **常量去重**：`Context` 中按类型分桶的 `HashMap` 避免重复的常量 temp。
- **`RegSet` 使用 `u64` 位图**：寄存器分配热路径使用位操作而非集合类型。

## 构建

```bash
cargo build                  # 构建所有 crate
cargo test                   # 运行全部 88 个测试
cargo clippy -- -D warnings  # Lint 检查
cargo fmt --check            # 格式检查
```

## 已实现内容

### tcg-core

- **类型系统**：`Type`（I32/I64/I128/V64/V128/V256）、`Cond`（QEMU 兼容编码）、`MemOp`（位域打包）、`RegSet`（u64 位图）
- **Opcodes**：约 70 个统一 opcode，配有静态 `OpDef` 表和 `OpFlags` 属性标志（INT、SIDE_EFFECTS、BB_EXIT、CARRY_IN/OUT 等）
- **临时变量**：五种生命周期（Ebb、Tb、Global、Fixed、Const），包含寄存器分配器状态
- **标签**：支持前向引用，通过 `LabelUse`/`RelocKind` 进行 back-patching
- **操作**：`Op` 使用固定大小参数数组，`LifeData` 用于活跃性分析
- **上下文**：翻译上下文，`reset()` 时保留全局变量，支持常量去重
- **翻译块**：`TranslationBlock` 双出口设计，`JumpCache`（4096 项直接映射缓存）

### tcg-backend

- **CodeBuffer**：基于 mmap 的 JIT 内存，遵循 W^X（写异或执行）纪律
- **x86-64 后端**：
  - System V ABI prologue/epilogue，`TCG_AREG0 = RBP`（env 指针）
  - 双 epilogue 入口：零返回路径 + TB 返回路径
  - `exit_tb`、`goto_tb`（4 字节对齐用于原子修补）、`goto_ptr`
  - 栈帧：callee-saved 寄存器 + 128B 调用参数区 + 1024B 溢出区

## QEMU 参考

本项目参考以下 QEMU 源文件：

- `tcg/tcg.c`、`tcg/tcg-op.c` — 核心 codegen 和 IR 发射
- `tcg/optimize.c` — IR 优化器
- `accel/tcg/cpu-exec.c` — 执行循环
- `tcg/i386/tcg-target.c.inc` — x86-64 后端
- `include/tcg/tcg-opc.h` — Opcode 定义

## 文档

- [设计文档](docs/design.md) — 详细的架构和设计原理

## 许可证

[MIT](LICENSE)
