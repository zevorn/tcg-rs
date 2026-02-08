# x86-64 Backend 指令编码器

## 1. 概述

`tcg-backend/src/x86_64/emitter.rs` 实现了 x86-64 宿主架构的完整 GPR 指令编码器，参考 QEMU 的 `tcg/i386/tcg-target.c.inc`。采用分层编码架构：

```
前缀标志 (P_*) + 操作码常量 (OPC_*)
        ↓
核心编码函数 (emit_opc / emit_modrm / emit_modrm_offset)
        ↓
指令发射器 (emit_arith_rr / emit_mov_ri / emit_jcc / ...)
        ↓
X86_64CodeGen (prologue / epilogue / exit_tb / goto_tb)
```

## 2. 编码基础设施

### 2.1 前缀标志 (P_*)

操作码常量使用 `u32` 类型，高位编码前缀信息：

| 标志 | 值 | 含义 |
|------|-----|------|
| `P_EXT` | 0x100 | 0x0F 转义前缀 |
| `P_EXT38` | 0x200 | 0x0F 0x38 三字节转义 |
| `P_EXT3A` | 0x10000 | 0x0F 0x3A 三字节转义 |
| `P_DATA16` | 0x400 | 0x66 操作数大小前缀 |
| `P_REXW` | 0x1000 | REX.W = 1（64 位操作） |
| `P_REXB_R` | 0x2000 | REG 字段字节寄存器访问 |
| `P_REXB_RM` | 0x4000 | R/M 字段字节寄存器访问 |
| `P_SIMDF3` | 0x20000 | 0xF3 前缀 |
| `P_SIMDF2` | 0x40000 | 0xF2 前缀 |

### 2.2 操作码常量 (OPC_*)

常量命名遵循 QEMU 的 `tcg-target.c.inc` 风格（使用 `#![allow(non_upper_case_globals)]`）：

```rust
pub const OPC_ARITH_EvIb: u32 = 0x83;        // 算术 reg, imm8
pub const OPC_MOVL_GvEv: u32 = 0x8B;         // MOV 加载
pub const OPC_JCC_long: u32 = 0x80 | P_EXT;  // 条件跳转 rel32
pub const OPC_BSF: u32 = 0xBC | P_EXT;       // 位扫描
pub const OPC_LZCNT: u32 = 0xBD | P_EXT | P_SIMDF3; // 前导零计数
```

### 2.3 核心编码函数

| 函数 | 用途 |
|------|------|
| `emit_opc(buf, opc, r, rm)` | 发射 REX 前缀 + 转义字节 + 操作码 |
| `emit_modrm(buf, opc, r, rm)` | 寄存器-寄存器 ModR/M（mod=11） |
| `emit_modrm_ext(buf, opc, ext, rm)` | 组操作码的 /r 扩展 |
| `emit_modrm_offset(buf, opc, r, base, offset)` | 内存 [base+disp] |
| `emit_modrm_sib(buf, opc, r, base, index, shift, offset)` | SIB 寻址 |
| `emit_modrm_ext_offset(buf, opc, ext, base, offset)` | 组操作码 + 内存 |

## 3. 指令分类

### 3.1 算术指令

| 函数 | 指令 | 说明 |
|------|------|------|
| `emit_arith_rr(op, rexw, dst, src)` | ADD/SUB/AND/OR/XOR/CMP/ADC/SBB | 寄存器-寄存器 |
| `emit_arith_ri(op, rexw, dst, imm)` | 同上 | 寄存器-立即数（自动选择 imm8/imm32） |
| `emit_arith_mr(op, rexw, base, offset, src)` | 同上 | 内存-寄存器（存储操作） |
| `emit_arith_rm(op, rexw, dst, base, offset)` | 同上 | 寄存器-内存（加载操作） |
| `emit_neg(rexw, reg)` | NEG | 取反 |
| `emit_not(rexw, reg)` | NOT | 按位取反 |
| `emit_inc(rexw, reg)` | INC | 自增 |
| `emit_dec(rexw, reg)` | DEC | 自减 |

`ArithOp` 枚举值对应 x86 的 /r 字段：Add=0, Or=1, Adc=2, Sbb=3, And=4, Sub=5, Xor=6, Cmp=7。

### 3.2 移位指令

| 函数 | 指令 | 说明 |
|------|------|------|
| `emit_shift_ri(op, rexw, dst, imm)` | SHL/SHR/SAR/ROL/ROR | 立即数移位（imm=1 使用短编码） |
| `emit_shift_cl(op, rexw, dst)` | 同上 | 按 CL 寄存器移位 |
| `emit_shld_ri(rexw, dst, src, imm)` | SHLD | 双精度左移 |
| `emit_shrd_ri(rexw, dst, src, imm)` | SHRD | 双精度右移 |

### 3.3 数据移动

| 函数 | 指令 | 说明 |
|------|------|------|
| `emit_mov_rr(rexw, dst, src)` | MOV r, r | 32/64 位寄存器传送 |
| `emit_mov_ri(rexw, reg, val)` | MOV r, imm | 智能选择：xor(0) / mov r32(u32) / mov r64 sign-ext(i32) / movabs(i64) |
| `emit_movzx(opc, dst, src)` | MOVZBL/MOVZWL | 零扩展 |
| `emit_movsx(opc, dst, src)` | MOVSBL/MOVSWL/MOVSLQ | 符号扩展 |
| `emit_bswap(rexw, reg)` | BSWAP | 字节序交换 |

### 3.4 内存操作

| 函数 | 指令 | 说明 |
|------|------|------|
| `emit_load(rexw, dst, base, offset)` | MOV r, [base+disp] | 加载 |
| `emit_store(rexw, src, base, offset)` | MOV [base+disp], r | 存储 |
| `emit_store_byte(src, base, offset)` | MOV byte [base+disp], r | 字节存储 |
| `emit_store_imm(rexw, base, offset, imm)` | MOV [base+disp], imm32 | 立即数存储 |
| `emit_lea(rexw, dst, base, offset)` | LEA r, [base+disp] | 地址计算 |
| `emit_load_sib(rexw, dst, base, index, shift, offset)` | MOV r, [b+i*s+d] | 索引加载 |
| `emit_store_sib(rexw, src, base, index, shift, offset)` | MOV [b+i*s+d], r | 索引存储 |
| `emit_lea_sib(rexw, dst, base, index, shift, offset)` | LEA r, [b+i*s+d] | 索引地址计算 |
| `emit_load_zx(opc, dst, base, offset)` | MOVZBL/MOVZWL [mem] | 零扩展加载 |
| `emit_load_sx(opc, dst, base, offset)` | MOVSBL/MOVSWL/MOVSLQ [mem] | 符号扩展加载 |

### 3.5 乘除指令

| 函数 | 指令 | 说明 |
|------|------|------|
| `emit_mul(rexw, reg)` | MUL | 无符号乘法 RDX:RAX = RAX * reg |
| `emit_imul1(rexw, reg)` | IMUL | 有符号乘法（单操作数） |
| `emit_imul_rr(rexw, dst, src)` | IMUL r, r | 双操作数乘法 |
| `emit_imul_ri(rexw, dst, src, imm)` | IMUL r, r, imm | 三操作数乘法 |
| `emit_div(rexw, reg)` | DIV | 无符号除法 |
| `emit_idiv(rexw, reg)` | IDIV | 有符号除法 |
| `emit_cdq()` | CDQ | 符号扩展 EAX → EDX:EAX |
| `emit_cqo()` | CQO | 符号扩展 RAX → RDX:RAX |

### 3.6 位操作

| 函数 | 指令 | 说明 |
|------|------|------|
| `emit_bsf(rexw, dst, src)` | BSF | 位扫描（正向） |
| `emit_bsr(rexw, dst, src)` | BSR | 位扫描（反向） |
| `emit_lzcnt(rexw, dst, src)` | LZCNT | 前导零计数 |
| `emit_tzcnt(rexw, dst, src)` | TZCNT | 尾随零计数 |
| `emit_popcnt(rexw, dst, src)` | POPCNT | 人口计数 |
| `emit_bt_ri(rexw, reg, bit)` | BT | 位测试 |
| `emit_bts_ri(rexw, reg, bit)` | BTS | 位测试并置位 |
| `emit_btr_ri(rexw, reg, bit)` | BTR | 位测试并复位 |
| `emit_btc_ri(rexw, reg, bit)` | BTC | 位测试并取反 |
| `emit_andn(rexw, dst, src1, src2)` | ANDN | BMI1: dst = ~src1 & src2（VEX 编码） |

### 3.7 分支与比较

| 函数 | 指令 | 说明 |
|------|------|------|
| `emit_jcc(cond, target)` | Jcc rel32 | 条件跳转 |
| `emit_jmp(target)` | JMP rel32 | 无条件跳转 |
| `emit_call(target)` | CALL rel32 | 函数调用 |
| `emit_jmp_reg(reg)` | JMP *reg | 间接跳转 |
| `emit_call_reg(reg)` | CALL *reg | 间接调用 |
| `emit_setcc(cond, dst)` | SETcc | 条件置字节 |
| `emit_cmovcc(cond, rexw, dst, src)` | CMOVcc | 条件传送 |
| `emit_test_rr(rexw, r1, r2)` | TEST r, r | 按位与测试 |
| `emit_test_bi(reg, imm)` | TEST r8, imm8 | 字节测试 |

### 3.8 杂项

| 函数 | 指令 | 说明 |
|------|------|------|
| `emit_xchg(rexw, r1, r2)` | XCHG | 交换 |
| `emit_push(reg)` | PUSH | 压栈 |
| `emit_pop(reg)` | POP | 出栈 |
| `emit_push_imm(imm)` | PUSH imm | 立即数压栈 |
| `emit_ret()` | RET | 返回 |
| `emit_mfence()` | MFENCE | 内存屏障 |
| `emit_ud2()` | UD2 | 未定义指令（调试陷阱） |
| `emit_nops(n)` | NOP | Intel 推荐的多字节 NOP（1-8 字节） |

## 4. 内存寻址特殊情况

x86-64 ModR/M 编码有两个特殊寄存器需要额外处理：

- **RSP/R12（low3=4）**：作为基址时必须使用 SIB 字节（`0x24` = index=RSP/none, base=RSP）
- **RBP/R13（low3=5）**：作为基址且偏移为 0 时，必须使用 `mod=01, disp8=0`（因为 `mod=00, rm=5` 被编码为 RIP 相对寻址）

`emit_modrm_offset` 自动处理这些特殊情况。

## 5. 条件码映射

`X86Cond` 枚举映射 TCG 条件到 x86 JCC 条件码：

| TCG Cond | X86Cond | JCC 编码 |
|----------|---------|----------|
| Eq / TstEq | Je | 0x4 |
| Ne / TstNe | Jne | 0x5 |
| Lt | Jl | 0xC |
| Ge | Jge | 0xD |
| Ltu | Jb | 0x2 |
| Geu | Jae | 0x3 |

`X86Cond::invert()` 通过翻转低位实现条件取反（如 Je ↔ Jne）。

## 6. QEMU 参考对照

| tcg-rs 函数 | QEMU 函数 |
|-------------|-----------|
| `emit_opc` | `tcg_out_opc` |
| `emit_modrm` | `tcg_out_modrm` |
| `emit_modrm_offset` | `tcg_out_modrm_sib_offset` |
| `emit_arith_rr` | `tgen_arithr` |
| `emit_arith_ri` | `tgen_arithi` |
| `emit_mov_ri` | `tcg_out_movi` |
| `emit_jcc` | `tcg_out_jxx` |
| `emit_vex_modrm` | `tcg_out_vex_modrm` |
| `X86_64CodeGen::emit_prologue` | `tcg_target_qemu_prologue` |
