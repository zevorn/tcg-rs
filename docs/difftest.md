# Difftest 框架：tcg-rs vs QEMU 差分测试

## 1. 概述

Difftest（差分测试）是验证指令模拟正确性的核心手段：对同一条 RISC-V 指令，分别通过 **tcg-rs 全流水线**和 **QEMU 参考实现**执行，比较两者的 CPU 状态输出。如果结果一致，则认为 tcg-rs 的翻译是正确的。

**源文件**：`tests/src/frontend/difftest.rs`

**依赖工具**（需预装）：

| 工具 | 用途 |
|------|------|
| `riscv64-linux-gnu-gcc` | RISC-V 交叉编译器 |
| `qemu-riscv64` | QEMU RISC-V 用户态模拟器 |

---

## 2. 整体架构

```
                    ┌─────────────────────┐
                    │   Test Case 定义     │
                    │  (insn + init regs)  │
                    └────────┬────────────┘
                             │
              ┌──────────────┴──────────────┐
              ▼                             ▼
     ┌────────────────┐           ┌─────────────────┐
     │   tcg-rs 侧    │           │    QEMU 侧      │
     │                │           │                 │
     │ 1. 编码指令     │           │ 1. 生成 .S 汇编  │
     │ 2. translator   │           │ 2. gcc 交叉编译  │
     │    _loop 解码   │           │ 3. qemu-riscv64  │
     │ 3. IR 生成      │           │    执行          │
     │ 4. liveness     │           │ 4. 解析 stdout   │
     │ 5. regalloc     │           │    (256 字节     │
     │ 6. x86-64       │           │     寄存器转储)  │
     │    codegen      │           │                 │
     │ 7. 执行         │           │                 │
     └───────┬────────┘           └────────┬────────┘
             │                             │
             ▼                             ▼
     ┌────────────────┐           ┌─────────────────┐
     │  RiscvCpu 状态  │           │  [u64; 32] 数组  │
     │  .gpr[0..32]   │           │  x0..x31 值      │
     │  .pc           │           │                 │
     └───────┬────────┘           └────────┬────────┘
             │                             │
             └──────────────┬──────────────┘
                            ▼
                   ┌─────────────────┐
                   │  assert_eq!()   │
                   │  比较指定寄存器  │
                   └─────────────────┘
```

---

## 3. QEMU 侧原理

### 3.1 汇编模板

对每个测试用例，框架动态生成一段 RISC-V 汇编源码，结构如下：

```asm
.global _start
_start:
    la gp, save_area       # x3 = 保存区基址

    # ── Phase 1: 加载初始寄存器值 ──
    li t0, <val1>           # 用 li 伪指令设置初值
    li t1, <val2>           # 汇编器自动展开为多条指令

    # ── Phase 2: 执行被测指令 ──
    add t2, t0, t1          # 实际的测试指令

    # ── Phase 3: 保存全部 32 个寄存器 ──
    sd x0,  0(gp)
    sd x1,  8(gp)
    ...
    sd x31, 248(gp)

    # ── Phase 4: write(1, save_area, 256) ──
    li a7, 64               # Linux write 系统调用号
    li a0, 1                # fd = stdout
    mv a1, gp               # buf = save_area
    li a2, 256              # count = 32 * 8
    ecall

    # ── Phase 5: exit(0) ──
    li a7, 93               # Linux exit 系统调用号
    li a0, 0
    ecall

.bss
.align 3
save_area: .space 256       # 32 × 8 字节
```

**关键设计点**：

1. **Phase 3 在 Phase 4 之前**：先保存所有寄存器，再执行系统调用。系统调用会覆盖 a0/a1/a2/a7，但此时已保存。
2. **x3(gp) 保留**：用作保存区基址指针，不能作为测试寄存器。保存的 x3 值是 save_area 地址，非测试值。
3. **`li` 伪指令**：汇编器自动将 64 位立即数展开为 `lui + addi + slli + addi` 等多条指令序列。

### 3.2 编译与执行流程

```
gen_alu_asm()          生成 .S 源码
    │
    ▼
riscv64-linux-gnu-gcc  交叉编译
  -nostdlib -static      无 libc，纯系统调用
  -o /tmp/xxx.elf        输出静态 ELF
    │
    ▼
qemu-riscv64 xxx.elf   用户态模拟执行
    │
    ▼
stdout (256 bytes)     32 个 little-endian u64
    │
    ▼
parse → [u64; 32]     解析为寄存器数组
```

临时文件使用 `pid_tid` 命名避免并行测试冲突，执行完毕后自动清理。

### 3.3 分支指令的特殊处理

分支指令的 QEMU 侧使用 taken/not-taken 模式：

```asm
    li t0, <rs1_val>
    li t1, <rs2_val>
    beq t0, t1, 1f      # 被测分支
    li t2, 0             # not-taken 路径
    j 2f
1:  li t2, 1             # taken 路径
2:
    # ... 保存寄存器 ...
```

最终通过 x7(t2) 的值判断分支是否被执行：
- `t2 = 1` → 分支 taken
- `t2 = 0` → 分支 not-taken

---

## 4. tcg-rs 侧原理

### 4.1 ALU 指令

直接复用现有的全流水线基础设施：

```rust
fn run_tcgrs(
    init: &[(usize, u64)],  // 初始寄存器值
    insns: &[u32],           // RISC-V 机器码序列
) -> RiscvCpu {
    // 1. 将 insns 编码为字节流作为 guest 代码
    // 2. 创建 X86_64CodeGen 后端
    // 3. emit_prologue / emit_epilogue
    // 4. translator_loop 解码 → IR
    // 5. translate_and_execute:
    //    liveness → regalloc → codegen → 执行
    // 6. 返回 RiscvCpu 状态
}
```

流水线：`RISC-V 机器码 → decodetree 解码 → trans_* → TCG IR → liveness → regalloc → x86-64 codegen → 执行`

### 4.2 分支指令

tcg-rs 中分支指令会**退出翻译块（TB）**，不会继续执行后续指令。因此分支 difftest 采用不同策略：

```rust
// 只执行单条分支指令
let branch_insn = (test.insn_fn)(5, 6, 16);
let cpu = run_tcgrs(&init, &[branch_insn]);

// 通过 PC 值判断 taken/not-taken
let tcgrs_taken = if cpu.pc == 16 { 1 } else { 0 };
// taken → PC = 0 + 16 = 16
// not-taken → PC = 0 + 4 = 4
```

然后与 QEMU 侧的 t2 值进行比较。

---

## 5. 寄存器约定

| 寄存器 | ABI 名 | 用途 |
|--------|--------|------|
| x3 | gp | **保留**：QEMU 侧保存区基址 |
| x5 | t0 | 源操作数 1（rs1） |
| x6 | t1 | 源操作数 2（rs2） |
| x7 | t2 | 目标寄存器（rd） |

**约束**：测试用例不能使用 x3 作为测试寄存器，因为 QEMU 侧的 `la gp, save_area` 会覆盖其值。其他 31 个寄存器均可自由使用。

---

## 6. 边界值策略

框架预定义了一组覆盖关键边界的 64 位常量：

| 常量 | 值 | 含义 |
|------|----|------|
| `V0` | `0` | 零 |
| `V1` | `1` | 最小正数 |
| `VMAX` | `0x7FFF_FFFF_FFFF_FFFF` | i64 最大值 |
| `VMIN` | `0x8000_0000_0000_0000` | i64 最小值 |
| `VNEG1` | `0xFFFF_FFFF_FFFF_FFFF` | -1（全 1） |
| `V32MAX` | `0x7FFF_FFFF` | i32 最大值 |
| `V32MIN` | `0xFFFF_FFFF_8000_0000` | i32 最小值（符号扩展） |
| `V32FF` | `0xFFFF_FFFF` | u32 最大值 |
| `VPATTERN` | `0xDEAD_BEEF_CAFE_BABE` | 随机位模式 |

每条指令使用 4-7 组边界值组合进行测试，重点覆盖：
- 溢出边界（MAX+1, MIN-1）
- 符号扩展（W-suffix 指令的 32→64 位扩展）
- 零值行为
- 全 1 位模式

---

## 7. 测试覆盖

当前共 **35 个 difftest**，覆盖以下指令类别：

| 类别 | 指令 | 测试数 |
|------|------|--------|
| R-type ALU | add, sub, sll, srl, sra, slt, sltu, xor, or, and | 10 |
| I-type ALU | addi, slti, sltiu, xori, ori, andi, slli, srli, srai | 9 |
| LUI | lui | 1 |
| W-suffix R | addw, subw, sllw, srlw, sraw | 5 |
| W-suffix I | addiw, slliw, srliw, sraiw | 4 |
| Branch | beq, bne, blt, bge, bltu, bgeu | 6 |

**未覆盖**（指令尚未实现）：
- Load/Store（lb/lh/lw/ld/sb/sh/sw/sd）
- M 扩展（mul/div/rem 系列）
- auipc, jal, jalr（PC 相关，需特殊处理）

---

## 8. 新增测试用例指南

### 8.1 新增 ALU 指令测试

以新增 `mulw` 指令的 difftest 为例：

**步骤 1**：添加指令编码器（如果尚未存在）

```rust
fn mulw(rd: u32, rs1: u32, rs2: u32) -> u32 {
    rv_r(0b0000001, rs2, rs1, 0b000, rd, OP_REG32)
}
```

**步骤 2**：添加测试函数

```rust
#[test]
fn difftest_mulw() {
    let cases: Vec<(u64, u64)> = vec![
        (V0, V0),           // 0 × 0
        (V1, VNEG1),        // 1 × (-1)
        (V32MAX, 2),        // 溢出边界
        (VPATTERN, V32FF),  // 随机模式
    ];
    for (a, b) in cases {
        difftest_alu(&rtype_test(
            "mulw", "mulw", mulw(7, 5, 6), a, b,
        ));
    }
}
```

`rtype_test` 辅助函数自动：
- 设置 x5=a, x6=b 作为源操作数
- 生成 `mulw t2, t0, t1` 汇编
- 检查 x7(t2) 的结果

### 8.2 新增 I-type 指令测试

以 `sltiu` 为例：

```rust
#[test]
fn difftest_sltiu() {
    let cases: Vec<(u64, i32)> = vec![
        (V0, 0),
        (V0, 1),
        (VNEG1, -1),  // imm 符号扩展后比较
    ];
    for (a, imm) in cases {
        difftest_alu(&itype_test(
            "sltiu",
            &format!("sltiu t2, t0, {imm}"),
            sltiu(7, 5, imm),
            a,
        ));
    }
}
```

`itype_test` 辅助函数：
- 设置 x5=a 作为源操作数
- 使用传入的汇编字符串（含立即数）
- 检查 x7(t2) 的结果

### 8.3 新增分支指令测试

```rust
#[test]
fn difftest_beq() {
    let cases: Vec<(u64, u64)> = vec![
        (V0, V0),       // 相等 → taken
        (V0, V1),       // 不等 → not-taken
        (VNEG1, VNEG1), // 负数相等 → taken
    ];
    for (a, b) in cases {
        difftest_branch(&BranchTest {
            name: "beq",
            mnemonic: "beq",
            insn_fn: beq,
            rs1_val: a,
            rs2_val: b,
        });
    }
}
```

`BranchTest` 结构体字段：
- `mnemonic`：QEMU 侧汇编助记符
- `insn_fn`：tcg-rs 侧指令编码函数
- `rs1_val`/`rs2_val`：两个源操作数的值

### 8.4 新增自定义测试（非标准模式）

如果指令不符合 R-type/I-type/Branch 的标准模式（例如 LUI 无源寄存器），可直接构造 `AluTest`：

```rust
#[test]
fn difftest_lui() {
    let imm = 0x12345_000u32 as i32;
    let upper = (imm as u32) >> 12;
    difftest_alu(&AluTest {
        name: "lui",
        asm: format!("lui t2, {upper}"),
        insn: lui(7, imm),
        init: vec![],       // 无需初始化源寄存器
        check_reg: 7,
    });
}
```

`AluTest` 字段说明：

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | `&str` | 测试名称（用于错误消息） |
| `asm` | `String` | QEMU 侧的汇编指令文本 |
| `insn` | `u32` | tcg-rs 侧的机器码 |
| `init` | `Vec<(usize, u64)>` | 初始寄存器 (idx, val) |
| `check_reg` | `usize` | 要比较的目标寄存器索引 |

---

## 9. 运行测试

```bash
# 运行全部 difftest
cargo test -p tcg-tests difftest

# 运行单个指令的 difftest
cargo test -p tcg-tests difftest_add

# 并行运行（默认）
cargo test -p tcg-tests difftest -- --test-threads=4

# 查看详细输出
cargo test -p tcg-tests difftest -- --nocapture
```

**失败输出示例**：

```
DIFFTEST FAIL [add]: x7 tcg-rs=0x64 qemu=0x65
```

含义：`add` 指令的 x7 寄存器，tcg-rs 计算结果为 `0x64`，QEMU 参考结果为 `0x65`，存在差异。

---

## 10. 限制与未来工作

1. **x3(gp) 不可测试**：QEMU 侧保留用于保存区基址。如需测试 x3，需改用其他寄存器作为基址。

2. **PC 相关指令**：auipc/jal/jalr 的结果依赖于指令在内存中的绝对地址，tcg-rs 和 QEMU 的加载地址不同，需要计算相对偏移后比较。

3. **Load/Store**：需要 guest 内存访问机制（QemuLd/QemuSt），待实现后可扩展 difftest。

4. **随机化测试**：当前使用固定边界值，未来可引入随机寄存器值生成器，提高覆盖率。

5. **多指令序列**：当前主要测试单条指令，未来可扩展为多指令序列的 difftest（需处理 TB 内控制流）。
