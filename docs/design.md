# tcg-rs 设计文档

## 1. Workspace 分层

```
tcg-rs/
├── tcg-core/       # IR 定义层：纯数据结构，零依赖
├── tcg-backend/    # 代码生成层：依赖 tcg-core + libc
└── tcg-tests/      # 测试层：依赖 tcg-core + tcg-backend
```

**设计意图**：遵循 QEMU 的 `include/tcg/` (定义) 与 `tcg/` (实现) 分离原则。`tcg-core` 是纯粹的数据定义，不包含任何平台相关代码或 `unsafe`，未来的 `tcg-ir`、`tcg-opt`、`tcg-frontend` 都只需依赖 `tcg-core`。测试独立成 crate 是为了保持源码文件干净，且外部 crate 测试能验证公共 API 的完整性。

---

## 2. tcg-core 核心数据结构

### 2.1 Type 系统 (`types.rs`)

```
Type: I32 | I64 | I128 | V64 | V128 | V256
```

- `#[repr(u8)]` 确保枚举值可直接用作数组索引（`Type as usize`）
- 整数/向量分类方法 (`is_integer()` / `is_vector()`) 用于后续优化器和后端的类型分派
- `TYPE_COUNT = 6` 配合 Context 中的 `const_table: [HashMap; TYPE_COUNT]` 实现按类型分桶的常量去重

### 2.2 Cond 条件码 (`types.rs`)

```
Cond: Never=0, Always=1, Eq=8, Ne=9, Lt=10, ..., TstEq=18, TstNe=19
```

- **编码值直接对齐 QEMU**（`tcg.h` 中 `TCGCond` 的数值），这样未来做前端翻译时可以零成本转换
- `invert()` 和 `swap()` 都是 involution（自逆），测试中专门验证了这一性质
- `TstEq`/`TstNe` 是 QEMU 7.x+ 新增的 test-and-branch 条件，提前纳入

### 2.3 MemOp (`types.rs`)

```
MemOp(u16) — bit-packed: [1:0]=size, [2]=sign, [3]=bswap, [6:4]=align
```

- 位域打包设计直接映射 QEMU 的 `MemOp`，保持二进制兼容
- 提供语义化构造器 `ub()/sb()/uw()/sw()/ul()/sl()/uq()` 避免手写位操作

### 2.4 RegSet (`types.rs`)

```
RegSet(u64) — 64-bit bitmap, supports up to 64 host registers
```

- 用 `u64` 位图而非 `HashSet` 或 `Vec`，因为寄存器分配是热路径，位操作（union/intersect/subtract）比集合操作快一个数量级
- `const fn` 方法允许在编译期构造常量寄存器集（如 `RESERVED_REGS`）

### 2.5 统一多态 Opcode (`opcode.rs`)

```
enum Opcode { Mov, Add, Sub, ..., Count }  // ~70 variants + sentinel
```

**关键决策：类型多态而非类型分裂**

QEMU 原始设计中 `add_i32` 和 `add_i64` 是不同的 opcode。我们改为统一的 `Add`，实际类型由 `Op::op_type` 字段携带。原因：

1. 减少 opcode 数量约 40%（从 ~150 降到 ~70）
2. 优化器可以用统一逻辑处理，不需要 `match (Add32, Add64) => ...`
3. 后端通过 `op.op_type` 选择 32/64 位指令编码，逻辑更清晰
4. `OpFlags::INT` 标记哪些 opcode 是多态的，非多态的（如 `ExtI32I64`）有固定类型

### 2.6 OpDef 静态表 (`opcode.rs`)

```rust
pub static OPCODE_DEFS: [OpDef; Opcode::Count as usize] = [ ... ];
```

- 用 `Opcode::Count` 作为 sentinel 确保表大小与枚举同步——如果新增 opcode 忘记加表项，编译期就会报错
- 每个 `OpDef` 记录 `nb_oargs/nb_iargs/nb_cargs/flags`，这是优化器和寄存器分配器的核心元数据
- `OpFlags` 用位标志而非 `Vec<Flag>`，因为标志检查在编译循环中极其频繁

### 2.7 Temp 临时变量 (`temp.rs`)

```
TempKind: Ebb | Tb | Global | Fixed | Const
```

五种生命周期直接映射 QEMU 的 `TCGTempKind`：

| Kind | 生命周期 | 典型用途 |
|------|---------|---------|
| `Ebb` | 单个扩展基本块 | 算术中间结果 |
| `Tb` | 整个翻译块 | 跨 BB 的值 |
| `Global` | 跨 TB，backed by CPUState | `pc`, `sp` 等 |
| `Fixed` | 固定绑定到宿主寄存器 | `env` (RBP) |
| `Const` | 编译期常量 | 立即数 |

`Temp` 结构体同时承载 IR 属性（`ty`, `kind`）和寄存器分配状态（`val_type`, `reg`, `mem_coherent`），这是 QEMU 的设计——避免额外的 side table 查找。

### 2.8 Label 前向引用 (`label.rs`)

```
Label { present, has_value, value, uses: Vec<LabelUse> }
LabelUse { offset, kind: RelocKind::Rel32 }
```

- 支持前向引用：分支指令可以在 label 定义之前引用它
- `uses` 记录所有未解析的引用位置，`set_value()` 时后端遍历 `uses` 做 back-patching
- `RelocKind` 目前只有 `Rel32`（x86-64 的 RIP-relative 32 位位移），未来扩展 AArch64 时加 `Adr21` 等

### 2.9 Op IR 操作 (`op.rs`)

```rust
struct Op {
    opc: Opcode,
    op_type: Type,        // 多态 opcode 的实际类型
    param1/param2: u8,    // opcode-specific (CALLI/CALLO/VECE)
    life: LifeData,       // 活跃性分析结果
    output_pref: [RegSet; 2],  // 寄存器分配提示
    args: [TempIdx; 10],  // 参数（输出+输入+常量）
}
```

- `args` 是固定大小数组而非 `Vec`，避免堆分配——每个 TB 可能有数百个 Op
- `oargs()/iargs()/cargs()` 通过 `OpDef` 的参数计数做切片，零成本抽象
- `LifeData(u32)` 用 2 bit per arg 编码 dead/sync 状态，紧凑且高效

### 2.10 Context 翻译上下文 (`context.rs`)

```rust
struct Context {
    temps: Vec<Temp>,
    ops: Vec<Op>,
    labels: Vec<Label>,
    nb_globals: u32,
    const_table: [HashMap<u64, TempIdx>; TYPE_COUNT],
    // frame, reserved_regs, gen_insn_end_off...
}
```

**关键设计**：

- **Globals 在 temps 数组前端**：`temps[0..nb_globals]` 是全局变量，`reset()` 时 `truncate(nb_globals)` 保留它们，清除所有局部变量。这避免了每次翻译新 TB 时重新注册全局变量
- **常量去重**：`const_table` 按类型分桶，相同 `(type, value)` 的常量只创建一个 Temp。QEMU 中这是重要的内存优化，因为很多指令共享相同的立即数（0, 1, -1 等）
- **断言保护**：`new_global()` 和 `new_fixed()` 要求在任何局部变量分配之前调用，通过 `assert_eq!(temps.len(), nb_globals)` 强制执行

### 2.11 TranslationBlock (`tb.rs`)

```rust
struct TranslationBlock {
    pc, flags, cflags,          // 查找键
    host_offset, host_size,     // 生成的宿主代码位置
    jmp_insn_offset: [Option<u32>; 2],   // goto_tb 跳转指令偏移
    jmp_reset_offset: [Option<u32>; 2],  // 解链时的重置偏移
}
```

- **双出口设计**：每个 TB 最多 2 个直接跳转出口（对应条件分支的 taken/not-taken），`jmp_insn_offset` 记录跳转指令位置用于 TB chaining 时原子修补
- **JumpCache**：`Box<[Option<usize>; 4096]>` 直接映射缓存，`(pc >> 2) & 0xFFF` 索引，O(1) 查找。用 `Box` 避免 4096 * 8 = 32KB 在栈上分配
- **哈希函数**：`pc * 0x9e3779b97f4a7c15 ^ flags`，黄金比例常数确保良好的分布

---

## 3. tcg-backend 代码生成层

### 3.1 CodeBuffer (`code_buffer.rs`)

```
mmap(PROT_READ|PROT_WRITE) → emit code → mprotect(PROT_READ|PROT_EXEC)
```

- **W^X 纪律**：写入和执行互斥，`set_executable()` / `set_writable()` 切换权限
- `emit_u8/u16/u32/u64/bytes` + `patch_u32` 覆盖了所有 x86-64 指令编码需求
- `write_unaligned` 处理非对齐写入（x86 允许，但 ARM 不允许——未来需要注意）

### 3.2 HostCodeGen trait (`lib.rs`)

```rust
trait HostCodeGen {
    fn emit_prologue(&mut self, buf: &mut CodeBuffer);
    fn emit_epilogue(&mut self, buf: &mut CodeBuffer);
    fn patch_jump(&mut self, buf: &mut CodeBuffer, jump_offset, target_offset);
    fn epilogue_offset(&self) -> usize;
    fn init_context(&self, ctx: &mut Context);
    fn op_constraint(&self, opc: Opcode) -> &'static OpConstraint;
    // + register allocator primitives: tcg_out_mov/movi/ld/st/op
}
```

- Trait-based 而非条件编译，允许同一二进制支持多后端（测试/模拟场景）
- `init_context()` 让后端向 Context 注入平台特定配置（保留寄存器、栈帧布局）
- `op_constraint()` 返回每个 opcode 的寄存器约束，供通用寄存器分配器消费（见 3.3）

### 3.3 约束系统 (`constraint.rs`)

```rust
struct ArgConstraint {
    regs: RegSet,       // allowed registers
    oalias: bool,       // output aliases an input
    ialias: bool,       // input is aliased to an output
    alias_index: u8,    // which arg it aliases
    newreg: bool,       // output must not overlap any input
}

struct OpConstraint {
    args: [ArgConstraint; MAX_OP_ARGS],
}
```

声明式描述每个 opcode 的寄存器分配需求，对齐 QEMU 的 `TCGArgConstraint` + `C_O*_I*` 宏系统。

**约束类型**：

| 约束 | 含义 | QEMU 等价 | 典型用途 |
|------|------|-----------|---------|
| `oalias` | 输出复用输入的寄存器 | `"0"` (alias) | 破坏性二元运算 (SUB/AND/...) |
| `ialias` | 输入可被输出复用 | 对应 oalias 的输入端 | 与 oalias 配对 |
| `newreg` | 输出不得与任何输入重叠 | `"&"` (newreg) | SetCond (setcc 只写低字节) |
| `fixed` | 单寄存器约束 | `"c"` (RCX) | 移位计数必须在 RCX |

**Builder 函数**：

| 函数 | 签名 | 用途 |
|------|------|------|
| `o1_i2(o0, i0, i1)` | 三地址 | Add (LEA) |
| `o1_i2_alias(o0, i0, i1)` | 输出别名 input0 | Sub/Mul/And/Or/Xor |
| `o1_i1_alias(o0, i0)` | 一元别名 | Neg/Not |
| `o1_i2_alias_fixed(o0, i0, reg)` | 别名 + 固定 | Shl/Shr/Sar (RCX) |
| `n1_i2(o0, i0, i1)` | newreg 输出 | SetCond |
| `o0_i2(i0, i1)` | 无输出 | BrCond/St |

### 3.4 x86-64 栈帧布局 (`regs.rs`)

```
高地址
┌─────────────────────┐
│ return address (8B) │  ← call 指令压入
├─────────────────────┤
│ push rbp    (8B)    │  ← CALLEE_SAVED[0]
│ push rbx    (8B)    │
│ push r12    (8B)    │
│ push r13    (8B)    │
│ push r14    (8B)    │
│ push r15    (8B)    │  PUSH_SIZE = 56B
├─────────────────────┤
│ STATIC_CALL_ARGS    │  128B (outgoing call args)
│ CPU_TEMP_BUF        │  1024B (spill slots)
│                     │  STACK_ADDEND = FRAME_SIZE - PUSH_SIZE
├─────────────────────┤
│                     │  ← RSP (16-byte aligned)
└─────────────────────┘
低地址
```

- `FRAME_SIZE` 编译期计算并 16 字节对齐，满足 System V ABI 要求
- `TCG_AREG0 = RBP`：env 指针固定在 RBP，匹配 QEMU 约定。所有 TB 代码通过 RBP 访问 CPUState

### 3.5 Prologue/Epilogue (`emitter.rs`)

**Prologue**:

1. `push` 6 个 callee-saved 寄存器（RBP 在最前）
2. `mov rbp, rdi` — 将第一个参数（env 指针）存入 TCG_AREG0
3. `sub rsp, STACK_ADDEND` — 分配栈帧
4. `jmp *rsi` — 跳转到第二个参数（TB 宿主代码地址）

**Epilogue（双入口）**:

- `epilogue_return_zero`: `xor eax, eax` → fall through（用于 `goto_ptr` 查找失败）
- `tb_ret`: `add rsp` → `pop` 寄存器 → `ret`（用于 `exit_tb` 正常返回）

这个双入口设计避免了 `exit_tb(0)` 时多余的 `mov rax, 0` 指令。

### 3.6 TB 控制流指令

- **`exit_tb(val)`**：val==0 时直接 `jmp epilogue_return_zero`；否则 `mov rax, val` + `jmp tb_ret`
- **`goto_tb`**：发射 `E9 00000000`（JMP rel32），NOP 填充确保 disp32 字段 4 字节对齐，使得 TB chaining 时的原子修补是安全的
- **`goto_ptr(reg)`**：`jmp *reg`，用于间接跳转（lookup_and_goto_ptr 之后）

---

## 4. 翻译流水线

完整的翻译流水线将 TCG IR 转换为可执行的宿主机器码：

```
IR Builder (gen_*)  →  Liveness Analysis  →  RegAlloc + Codegen  →  Execute
  ir_builder.rs          liveness.rs           regalloc.rs         translate.rs
                                               codegen.rs
```

### 4.1 IR Builder (`ir_builder.rs`)

`impl Context` 上的 `gen_*` 方法，将高层操作转换为 `Op` 并追加到
ops 列表。每个方法创建 `Op::with_args()` 并设置正确的 opcode、
type 和 args 布局。

**常量参数编码**：条件码、偏移量、label ID 等常量参数编码为
`TempIdx(raw_value as u32)` 存入 `args[]`，与 QEMU 约定一致。

**已实现的 IR 生成方法**：

| 类别 | 方法 | 签名 |
|------|------|------|
| 二元 ALU | `gen_add/sub/mul/and/or/xor/shl/shr/sar` | (ty, d, a, b) → d |
| 一元 | `gen_neg/not/mov` | (ty, d, s) → d |
| 条件设置 | `gen_setcond` | (ty, d, a, b, cond) → d |
| 内存访问 | `gen_ld` / `gen_st` | (ty, dst/src, base, offset) |
| 控制流 | `gen_br/brcond/set_label` | (label_id) / (ty, a, b, cond, label) |
| TB 出口 | `gen_goto_tb/exit_tb` | (tb_idx) / (val) |
| 边界 | `gen_insn_start` | (pc) |

### 4.2 活跃性分析 (`liveness.rs`)

反向遍历 ops 列表，为每个 op 计算 `LifeData`，标记哪些参数在
该 op 之后死亡（dead）以及哪些全局变量需要同步回内存（sync）。

**算法**：

1. 初始化 `temp_state[0..nb_temps]` = false（全部死亡）
2. TB 末尾：所有全局变量标记为活跃
3. 反向遍历每个 op：
   - 遇到 `BB_END` 标志：所有全局变量标记为活跃
   - 输出参数：若 `!temp_state[tidx]` → 标记 dead；
     然后 `temp_state[tidx] = false`
   - 输入参数：若 `!temp_state[tidx]` → 标记 dead（最后使用），
     若为全局变量则标记 sync；然后 `temp_state[tidx] = true`
4. 将计算的 `LifeData` 写回 `op.life`

### 4.3 寄存器分配器 (`regalloc.rs`)

约束驱动的贪心逐 op 分配器，前向遍历 ops 列表，对齐 QEMU 的
`tcg_reg_alloc_op()`。MVP 不支持溢出（spill）——14 个可分配
GPR 对简单 TB 足够。

**状态**：

```
RegAllocState {
    reg_to_temp: [Option<TempIdx>; 16],  // 寄存器→临时变量映射
    free_regs: RegSet,                    // 空闲寄存器位图
    allocatable: RegSet,                  // 可分配寄存器集合
}
```

**主循环（`regalloc_and_codegen`）**：

对每个 op 按类型分派：

| Op 类型 | 处理策略 |
|---------|---------|
| Nop/InsnStart | 跳过 |
| Mov | 加载源→分配目标→emit host mov（寄存器重命名优化） |
| SetLabel | sync globals → 解析 label → back-patch 前向引用 |
| Br | sync globals → emit jmp（前向引用记录 patch 位置） |
| BrCond | 约束加载输入→sync globals→emit cmp+jcc→记录前向引用 |
| ExitTb/GotoTb | sync globals → 委托 `tcg_out_op` |
| **通用 op** | **`regalloc_op()` — 约束驱动通用路径** |

**通用 op 处理流程（`regalloc_op`）**：

```
1. Process inputs:
   for each input i:
     if ialias && dead && !readonly:
       load into required regs, prefer output_pref → mark reusable
     else:
       load into required regs, forbidden = already allocated

2. Fixup: re-read i_regs after all inputs
   (eviction of earlier inputs by later fixed constraints)

3. Free dead inputs

4. Process outputs:
   for each output k:
     if oalias && input reusable: reuse input's register
     if oalias && input live: copy input away, take its register
     if newreg: allocate from required & ~(i_allocated | o_allocated)
     else: allocate from required & ~o_allocated

5. Emit host code: backend.tcg_out_op(buf, op, oregs, iregs, cargs)

6. Free dead outputs, sync globals at BB boundaries
```

**关键辅助函数**：

| 函数 | 用途 |
|------|------|
| `evict_reg()` | 驱逐寄存器占用者：全局→sync+Mem；局部→mov 到空闲寄存器 |
| `reg_alloc()` | 从 `required & ~forbidden` 分配，优先 preferred；含强制驱逐回退 |
| `temp_load_to()` | 加载 temp 到满足约束的寄存器（Const→movi，Mem→ld，Reg→mov if needed） |
| `temp_sync()` | 将全局变量写回内存（`tcg_out_st`） |
| `sync_globals()` | 在 BB 边界同步所有活跃全局变量 |
| `temp_dead()` | 释放死亡 temp 的寄存器（全局/固定 temp 不释放） |

**强制驱逐**：当 `required & ~forbidden` 为空时（如 input0 占据
RCX 而 input1 的固定约束要求 RCX），`reg_alloc()` 忽略 forbidden
集合，强制驱逐占用者。配合 fixup 阶段重新读取 `i_regs`，确保被
驱逐的 input 的新位置被正确记录。

### 4.4 流水线编排 (`translate.rs`)

将各阶段串联为完整流水线：

```
translate():
    liveness_analysis(ctx)
    tb_start = buf.offset()
    regalloc_and_codegen(ctx, backend, buf)
    return tb_start

translate_and_execute():
    buf.set_writable()
    tb_start = translate(ctx, backend, buf)
    buf.set_executable()
    prologue_fn = transmute(buf.base_ptr())
    return prologue_fn(env, tb_ptr)
```

**Prologue 调用约定**：
`fn(env: *mut u8, tb_ptr: *const u8) -> usize`
- RDI = env 指针（prologue 存入 RBP）
- RSI = TB 代码地址（prologue 跳转到此处）
- 返回值 RAX = `exit_tb` 的值

### 4.5 端到端集成测试

`tcg-tests/src/integration/mod.rs` 使用最小 RISC-V CPU 状态
验证完整流水线：

```rust
#[repr(C)]
struct RiscvCpuState {
    regs: [u64; 32],  // x0-x31, offset 0..256
    pc: u64,          // offset 256
}
```

通过 `ctx.new_global()` 将 x0-x31 和 pc 注册为全局变量，
backed by `RiscvCpuState` 字段。

**测试用例**：

| 测试 | 验证内容 |
|------|---------|
| `test_addi_x1_x0_42` | 常量加法：x1 = x0 + 42 |
| `test_add_x3_x1_x2` | 寄存器加法：x3 = x1 + x2 |
| `test_sub_x3_x1_x2` | 寄存器减法：x3 = x1 - x2 |
| `test_beq_taken` | 条件分支（taken 路径） |
| `test_beq_not_taken` | 条件分支（not-taken 路径） |
| `test_sum_loop` | 循环：计算 1+2+3+4+5=15 |

---

## 5. 设计权衡总结

| 决策                   | 选择                  | 理由                     |
|------------------------|---------------------|--------------------------|
| Opcode 多态 vs 分裂     | 统一多态              | 减少 40% opcode，简化优化器 |
| Op.args 固定数组 vs Vec | 固定 `[TempIdx; 10]` | 避免堆分配，TB 内有数百个 Op |
| RegSet 位图 vs HashSet | `u64` 位图           | 寄存器分配热路径，位操作更快  |
| 后端 trait vs 条件编译   | Trait               | 可测试性，未来多后端支持     |
| 常量去重                | 按类型分桶 HashMap    | 避免重复 Temp，节省内存     |
| JumpCache 堆分配        | `Box<[_; 4096]>`    | 32KB 不适合放栈上          |
| TCG_AREG0 = RBP        | 匹配 QEMU            | 二进制兼容，便于参考验证     |

---

## 6. QEMU 参考映射

| QEMU C 结构/概念               | Rust 对应                       | 文件                                 |
|-------------------------------|--------------------------------|-------------------------------------|
| `TCGType`                     | `enum Type`                    | `tcg-core/src/types.rs`             |
| `TCGTempVal`                  | `enum TempVal`                 | `tcg-core/src/types.rs`             |
| `TCGCond`                     | `enum Cond`                    | `tcg-core/src/types.rs`             |
| `MemOp`                       | `struct MemOp(u16)`            | `tcg-core/src/types.rs`             |
| `TCGRegSet`                   | `struct RegSet(u64)`           | `tcg-core/src/types.rs`             |
| `TCGOpcode` + DEF macros      | `enum Opcode`                  | `tcg-core/src/opcode.rs`            |
| `TCGOpDef`                    | `struct OpDef` + `OPCODE_DEFS` | `tcg-core/src/opcode.rs`            |
| `TCG_OPF_*`                   | `struct OpFlags`               | `tcg-core/src/opcode.rs`            |
| `TCGTempKind`                 | `enum TempKind`                | `tcg-core/src/temp.rs`              |
| `TCGTemp`                     | `struct Temp`                  | `tcg-core/src/temp.rs`              |
| `TCGLabel`                    | `struct Label`                 | `tcg-core/src/label.rs`             |
| `TCGLifeData`                 | `struct LifeData(u32)`         | `tcg-core/src/op.rs`                |
| `TCGOp`                       | `struct Op`                    | `tcg-core/src/op.rs`                |
| `TCGContext`                  | `struct Context`               | `tcg-core/src/context.rs`           |
| `TranslationBlock`            | `struct TranslationBlock`      | `tcg-core/src/tb.rs`                |
| `CPUJumpCache`                | `struct JumpCache`             | `tcg-core/src/tb.rs`                |
| `tcg_target_callee_save_regs` | `CALLEE_SAVED`                 | `tcg-backend/src/x86_64/regs.rs`    |
| `tcg_out_tb_start` (prologue) | `HostCodeGen::emit_prologue`   | `tcg-backend/src/x86_64/emitter.rs` |
| `tcg_code_gen_epilogue`       | `HostCodeGen::emit_epilogue`   | `tcg-backend/src/x86_64/emitter.rs` |
| `tcg_out_exit_tb`             | `X86_64CodeGen::emit_exit_tb`  | `tcg-backend/src/x86_64/emitter.rs` |
| `tcg_out_goto_tb`             | `X86_64CodeGen::emit_goto_tb`  | `tcg-backend/src/x86_64/emitter.rs` |
| `tcg_out_goto_ptr`            | `X86_64CodeGen::emit_goto_ptr` | `tcg-backend/src/x86_64/emitter.rs` |
| `tcg_gen_op*` (IR emission)   | `Context::gen_*`               | `tcg-core/src/ir_builder.rs`        |
| `liveness_pass_1`             | `liveness_analysis()`          | `tcg-backend/src/liveness.rs`       |
| `tcg_reg_alloc_op`            | `regalloc_op()`                | `tcg-backend/src/regalloc.rs`       |
| `TCGArgConstraint`            | `ArgConstraint`                | `tcg-backend/src/constraint.rs`     |
| `C_O*_I*` macros              | `o1_i2()` / `o1_i2_alias()` etc. | `tcg-backend/src/constraint.rs`  |
| `tcg_target_op_def`           | `op_constraint()`              | `tcg-backend/src/x86_64/constraints.rs` |
| `tcg_out_op` (dispatch)       | `HostCodeGen::tcg_out_op`      | `tcg-backend/src/x86_64/codegen.rs` |
| `tcg_out_mov`                 | `HostCodeGen::tcg_out_mov`     | `tcg-backend/src/x86_64/codegen.rs` |
| `tcg_out_movi`                | `HostCodeGen::tcg_out_movi`    | `tcg-backend/src/x86_64/codegen.rs` |
| `tcg_out_ld`                  | `HostCodeGen::tcg_out_ld`      | `tcg-backend/src/x86_64/codegen.rs` |
| `tcg_out_st`                  | `HostCodeGen::tcg_out_st`      | `tcg-backend/src/x86_64/codegen.rs` |
| `tcg_gen_code`                | `translate()`                  | `tcg-backend/src/translate.rs`      |
