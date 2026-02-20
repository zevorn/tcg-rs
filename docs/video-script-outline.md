# 技术视频脚本大纲：用 AI 零代码构建超越 QEMU 的二进制翻译引擎

> 视频标题建议：**「用 Claude + Codex 零代码写出 4.5 万行 Rust，性能超 QEMU TCG 30%」**
>
> 预计时长：25–35 分钟 | 目标受众：系统级开发者、编译器/虚拟化爱好者、AI 辅助编程关注者

---

## 第一幕：开场与悬念（2 分钟）

### 1.1 Hook

- 直接展示终端画面：`tcg-riscv64` 运行 dhrystone，旁边 `qemu-riscv64` 同时运行
- 对比计时结果，tcg-rs 快 30%+
- 抛出问题：「这个引擎有 4.5 万行 Rust 代码、816 个测试、10 个 crate——但我没有手写过一行代码」

### 1.2 自我介绍

- 简要介绍自己的背景（QEMU/虚拟化方向）
- 说明项目动机：验证 AI 能否独立完成系统级底层软件工程

---

## 第二幕：什么是二进制动态翻译（3 分钟）

### 2.1 概念科普

- 类比：把一本日语书实时翻译成中文朗读出来
- 技术定义：运行时将客户架构（RISC-V）机器码翻译为宿主架构（x86-64）机器码
- QEMU TCG 是这个领域的事实标准，已有 20 年历史

### 2.2 翻译流水线动画

```
Guest Binary → Frontend (decode) → TCG IR → Optimizer → Backend (codegen) → Host Binary
                                      ↓
                              TranslationBlock Cache
```

- 用动画逐步展示每个阶段的作用
- 强调这不是解释执行，而是 JIT 编译——生成真正的 x86-64 机器码

### 2.3 为什么这很难

- 需要精通两种 ISA（RISC-V + x86-64）
- 需要实现寄存器分配器、指令编码器、优化器
- 需要处理 JIT 代码缓冲区（mmap/mprotect）
- 需要多线程安全的 TB 缓存和链路管理
- QEMU 的 TCG 部分约 5 万行 C 代码，经过 20 年打磨

---

## 第三幕：AI 工具链介绍（2 分钟）

### 3.1 Claude Code

- 定位：交互式 CLI 代理，擅长架构设计、代码生成、重构
- 在本项目中的角色：架构师 + 主力开发者
- 特点：理解 QEMU 源码，能对照 C 实现生成等价 Rust 代码

### 3.2 Codex

- 定位：批量代码生成与补全
- 在本项目中的角色：辅助实现重复性模式代码
- 特点：适合生成大量相似结构的指令翻译函数

### 3.3 协作模式

- Claude 负责架构决策和核心逻辑
- Codex 负责模式化代码批量生成
- 人类负责需求定义、验证和性能调优方向

---

## 第四幕：项目架构全景（4 分钟）

### 4.1 Crate 结构总览

| Crate | 职责 | 代码量 |
|-------|------|--------|
| `tcg-core` | IR 定义：158 个 opcodes、temps、labels | 核心层 |
| `tcg-backend` | 优化器 + 寄存器分配 + x86-64 codegen | 代码生成层 |
| `tcg-frontend` | RISC-V 解码 + 184 条指令翻译 | 前端层 |
| `tcg-exec` | MTTCG 执行循环 + TB 缓存 | 执行层 |
| `tcg-linux-user` | ELF 加载 + syscall 仿真 | 用户态层 |
| `decode` | .decode 解析器与代码生成器 | 工具层 |
| `disas` | RISC-V 反汇编器 | 工具层 |
| `tests` | 816 个分层测试 | 质量保障 |

- 展示 `cargo build` 编译全部 crate 的过程
- 强调零外部 runtime 依赖，纯 Rust 实现

### 4.2 与 QEMU 的对照映射

- 屏幕左右分栏：左边 QEMU C 源码路径，右边 tcg-rs Rust 对应文件
- 重点对比：`tcg.c` → `backend/`、`cpu-exec.c` → `exec/`、`translate.c` → `frontend/`

---

## 第五幕：核心技术亮点逐一拆解（8 分钟）

### 5.1 统一多态 Opcode 设计

- QEMU 痛点：`add_i32` 和 `add_i64` 是两个不同 opcode，导致 ~250 个 opcode
- tcg-rs 方案：统一 `Opcode::Add`，类型信息存在 `Op::op_type` 中
- 结果：158 个 opcode vs QEMU 的 ~250 个，减少 40%
- 展示 `core/src/opcode.rs` 中的枚举定义

### 5.2 约束驱动寄存器分配器

- 展示 `backend/src/regalloc.rs`（834 行）
- 核心思想：每个 opcode 只需声明一行约束，分配器完全泛型
- 对比 QEMU 的 per-opcode switch-case 分支
- 代码演示：`o1_i2_alias`、`o1_i2_alias_fixed` 约束声明
- 添加新 opcode 只需在约束表加一行，零分配器代码改动

### 5.3 x86-64 后端指令编码器

- 展示 `backend/src/x86_64/emitter.rs`（1,232 行）
- QEMU 风格的 `u32` 操作码常量 + 前缀标志编码
- 智能立即数：`mov_ri` 自动选择最短编码（xor/mov32/movabs）
- 完整 SIB 寻址支持
- 用 `tcg-irbackend --disas` 工具实时展示生成的 x86-64 汇编

### 5.4 IR 优化器

- 展示 `backend/src/optimize.rs`（635 行）
- 单遍优化：常量折叠 → 拷贝传播 → 代数简化 → 分支折叠
- 用 `tcg-irdump` 展示优化前后的 IR 对比
- 举例：`x + 0 → x`、`x * 1 → x`、`x ^ x → 0`、常量条件分支消除

### 5.5 MTTCG 执行引擎与热路径优化

- 展示 `exec/src/exec_loop.rs` 的核心循环
- 双层 TB 查找：JumpCache（4096 项）→ 全局 hash（32K 桶）
- `goto_tb` 直接链路 patch：TB 间零开销跳转
- 两个独创优化：
  - `next_tb_hint`：链路复用上次目标 TB，减少重复查找
  - `exit_target`：原子缓存间接跳转目标
- 用 `TCG_STATS=1` 展示实际命中率数据

### 5.6 RISC-V 前端（184 条指令）

- 展示 `frontend/src/riscv/trans.rs`（2,317 行）
- 支持 RV64I/M/F/D/C 全部用户态扩展
- QEMU 风格的 `BinOp` 函数指针模式
- 浮点辅助函数（`fpu.rs` 1,038 行）

---

## 第六幕：质量保障体系（3 分钟）

### 6.1 测试金字塔

```
            ┌──────────────┐
            │  Guest 程序   │  18 tests — 端到端
            ├──────────────┤
            │   Difftest   │  35 tests — vs QEMU
            ├──────────────┤
            │  前端指令测试  │  91 tests
            ├──────────────┤
            │   集成测试    │  105 tests
       ┌────┴──────────────┴────┐
       │       单元测试          │  567 tests
       └────────────────────────┘
```

### 6.2 差分测试框架

- 展示 `tests/src/frontend/difftest.rs`
- 工作流：tcg-rs 执行 → 生成汇编 → 交叉编译 → qemu-riscv64 执行 → 对比寄存器
- 运行 `cargo test -p tcg-tests` 展示 816 个测试全部通过

### 6.3 AI 生成代码的可靠性讨论

- 816 个测试是 AI 自己写的，也是 AI 自己通过的
- 差分测试是终极验证：不信任 AI，信任 QEMU 参考实现
- 人类的角色：定义测试策略，审查边界情况

---

## 第七幕：性能对比与分析（4 分钟）

### 7.1 基准测试环境

- 硬件配置、OS 版本、编译选项
- 对比对象：`qemu-riscv64`（最新 stable）vs `tcg-riscv64`（release build）

### 7.2 Dhrystone 对比演示

```bash
# 实时终端演示
TIMEFORMAT=%R; time target/release/tcg-riscv64 target/guest/riscv64/dhrystone
TIMEFORMAT=%R; time qemu-riscv64 target/guest/riscv64/dhrystone
```

- 多次运行取平均值
- 展示 tcg-rs 的性能统计输出（`TCG_STATS=1`）

### 7.3 性能优势来源分析

- Rust 零成本抽象 vs C 的运行时开销
- 统一 opcode 减少分派开销
- 约束驱动分配器减少分支预测失败
- `next_tb_hint` + `exit_target` 减少 hash 查找
- 更紧凑的代码生成（智能立即数编码）
- 专注用户态场景，无需支持全系统模拟的复杂性

### 7.4 诚实讨论局限性

- 目前只支持 RISC-V → x86-64 单一路径
- QEMU 支持 20+ 架构对，通用性远超 tcg-rs
- syscall 仿真覆盖有限
- 全局 `translate_lock` 在高并发下仍是瓶颈

---

## 第八幕：AI 开发工作流复盘（4 分钟）

### 8.1 开发时间线

- 从零到第一个 TB 执行
- 从单条指令到 184 条指令覆盖
- 从单线程到 MTTCG
- 从 hello world 到 dhrystone 跑通

### 8.2 AI 擅长的部分

- 对照 QEMU C 代码生成等价 Rust 实现
- 批量生成模式化的指令翻译函数（184 条）
- 编写分层测试和差分测试框架
- 架构设计和 crate 划分
- 编写详尽的技术文档（98 KB 设计文档）

### 8.3 AI 不擅长 / 需要人类介入的部分

- 性能调优的方向判断（哪里是瓶颈）
- JIT 代码缓冲区的 unsafe 边界设计
- 多线程竞态条件的微妙 bug
- 最终的正确性验证和边界情况审查

### 8.4 关键经验

- CLAUDE.md 是项目的「大脑」——详细的架构文档让 AI 保持上下文一致性
- 差分测试是 AI 生成代码的安全网
- 人类的价值在于定义「做什么」和「为什么」，AI 负责「怎么做」

---

## 第九幕：工具链演示（3 分钟）

### 9.1 IR 转储工具

```bash
# 将 ELF 翻译为 IR 并查看
tcg-irdump target/guest/riscv64/hello --arch riscv64 --count 5

# 导出二进制 IR
tcg-irdump target/guest/riscv64/hello --emit-bin hello.tcgir
```

### 9.2 后端代码生成工具

```bash
# 从 IR 生成 x86-64 并反汇编
tcg-irbackend hello.tcgir --disas
```

- 展示完整流水线：Guest ELF → IR → x86-64 汇编

### 9.3 端到端运行

```bash
# 运行 guest 程序
target/release/tcg-riscv64 target/guest/riscv64/hello
target/release/tcg-riscv64 target/guest/riscv64/argv_echo foo bar

# 带性能统计
TCG_STATS=1 target/release/tcg-riscv64 target/guest/riscv64/dhrystone
```

---

## 第十幕：总结与展望（2 分钟）

### 10.1 项目成果数据

| 指标 | 数值 |
|------|------|
| 总代码量 | 45,195 行 Rust |
| 测试数量 | 816 个 |
| Crate 数量 | 10 个 |
| IR Opcodes | 158 个 |
| 指令翻译 | 184 条 RISC-V 指令 |
| 设计文档 | 98 KB |
| 性能提升 | 超 QEMU TCG 30%+ |

### 10.2 核心观点

- AI 已经能够独立完成系统级底层软件工程
- 关键不是 AI 写代码的能力，而是人类定义问题和验证结果的能力
- CLAUDE.md 驱动的开发模式是 AI 辅助编程的最佳实践之一

### 10.3 未来方向

- 更多客户架构支持（ARM、x86）
- SIMD/向量指令后端
- 全系统模拟支持
- 开源社区协作

### 10.4 结尾 CTA

- 项目开源地址
- 欢迎 star / issue / PR
- 关注后续视频：深入讲解各子系统实现细节
