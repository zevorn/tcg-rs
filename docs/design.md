# tcg-rs 设计文档

## 1. Workspace 分层

```
tcg-rs/
├── core/           # IR 定义层：纯数据结构，零依赖
├── backend/        # 代码生成层：依赖 tcg-core + libc
├── decodetree/     # 解码器生成器：解析 .decode 文件，生成 Rust 解码器
├── frontend/       # 客户指令解码层：依赖 tcg-core + decodetree（构建时）
└── tests/          # 测试层：依赖 tcg-core + tcg-backend + tcg-frontend
```

**设计意图**：遵循 QEMU 的 `include/tcg/` (定义) 与 `tcg/` (实现) 分离原则。`tcg-core` 是纯粹的数据定义，不包含任何平台相关代码或 `unsafe`，`tcg-frontend` 和未来的 `tcg-opt` 都只需依赖 `tcg-core`。`decodetree` 是独立的构建时工具 crate，解析 QEMU 风格的 `.decode` 文件并生成 Rust 解码器代码。测试独立成 crate 是为了保持源码文件干净，且外部 crate 测试能验证公共 API 的完整性。

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
enum Opcode { Mov, Add, Sub, ..., Count }  // 158 variants + sentinel
```

**关键决策：类型多态而非类型分裂**

QEMU 原始设计中 `add_i32` 和 `add_i64` 是不同的 opcode。我们改为统一的 `Add`，实际类型由 `Op::op_type` 字段携带。原因：

1. 减少 opcode 数量（统一多态设计）
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
| `o2_i2_fixed(o0, o1, i1)` | 双固定输出 + 别名 | MulS2/MulU2 (RAX:RDX) |
| `o2_i3_fixed(o0, o1, i2)` | 双固定输出 + 双别名 | DivS2/DivU2 (RAX:RDX) |
| `o1_i4_alias2(o0, i0..i3)` | 输出别名 input2 | MovCond (CMOV) |

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
Guest Binary → Frontend (decode) → IR Builder (gen_*) → Liveness → RegAlloc + Codegen → Execute
                riscv/trans.rs      ir_builder.rs        liveness.rs  regalloc.rs        translate.rs
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

#### 4.3.1 架构概述

QEMU 的寄存器分配器 `tcg_reg_alloc_op()`（`tcg/tcg.c`）是完全
通用的——不含任何 per-opcode 分支。每个 opcode 的特殊需求（如
SUB 的破坏性语义、SHL 的 RCX 要求）全部通过 `TCGArgConstraint`
声明式描述，分配器只需读取约束并执行统一逻辑。

tcg-rs 的 `regalloc_op()` 对齐这一架构：

```
                    ┌──────────────┐
                    │ OpConstraint │  ← backend.op_constraint(opc)
                    └──────┬───────┘
                           │
  ┌────────────────────────▼────────────────────────┐
  │              regalloc_op() — 通用路径             │
  │                                                  │
  │  1. 按约束加载输入  →  2. fixup  →  3. 释放死输入 │
  │  4. 按约束分配输出  →  5. emit   →  6. 释放死输出 │
  │                        7. sync globals           │
  └──────────────────────────────────────────────────┘
```

这意味着新增 opcode 时只需在约束表中添加一行，分配器和 codegen
无需任何修改。

#### 4.3.2 分配器状态

```rust
struct RegAllocState {
    reg_to_temp: [Option<TempIdx>; 16],
    free_regs: RegSet,
    allocatable: RegSet,
}
```

| 字段 | 含义 |
|------|------|
| `reg_to_temp` | 16 个宿主寄存器各自映射到哪个 temp（None=空闲） |
| `free_regs` | 当前空闲且可分配的寄存器位图 |
| `allocatable` | 可分配寄存器集合（不变，排除 RSP/RBP） |

**初始化**：`free_regs = allocatable`，然后遍历所有 Fixed temp
（如 env/RBP），将其标记为已占用（`assign(reg, tidx)`）。

**Temp 状态机**：每个 `Temp` 有 `val_type` 字段追踪其当前位置：

```
                  temp_load_to()
    ┌──────┐    ┌──────────────┐    ┌─────┐
    │ Dead │───→│ Const / Mem  │───→│ Reg │
    └──────┘    └──────────────┘    └──┬──┘
       ↑                               │
       └───────── temp_dead() ─────────┘
                                  (局部 temp)
```

- **Dead**：未分配，不占用任何资源
- **Const**：编译期常量，需要 `movi` 加载到寄存器
- **Mem**：全局变量在内存中，需要 `ld` 加载到寄存器
- **Reg**：已在宿主寄存器中，可直接使用

全局变量和固定 temp 不会进入 Dead 状态——`temp_dead()` 对
它们是 no-op。

#### 4.3.3 主循环分派

`regalloc_and_codegen()` 前向遍历 ops 列表，按 opcode 分派：

| Op 类型 | 处理策略 | 原因 |
|---------|---------|------|
| Nop/InsnStart | 跳过 | 无代码生成 |
| Mov | 专用路径 | 寄存器重命名优化（QEMU 也单独处理） |
| SetLabel | sync → 解析 label → back-patch | 控制流汇合点 |
| Br | sync → emit jmp | 无条件跳转 |
| BrCond | 约束加载 → sync → emit cmp+jcc | 需要 sync 在 emit 之前 |
| ExitTb/GotoTb | sync → 委托 tcg_out_op | TB 退出 |
| GotoPtr | 约束加载 → sync → emit jmp *reg | 间接跳转 |
| Mb | 直接 emit mfence | 内存屏障 |
| **其他** | **`regalloc_op()`** | **通用约束驱动路径** |

**为什么 BrCond 不走通用路径？** 因为 BrCond 需要在 emit 之前
sync globals（分支目标可能是另一个 BB），而通用路径的 sync 在
emit 之后。此外 BrCond 的前向引用需要在 emit 之后记录
`label.add_use()`。

#### 4.3.4 通用 op 处理流程（`regalloc_op`）

以 `Sub t3, t1, t2`（约束 `o1_i2_alias`）为例，详细说明
8 个阶段：

**阶段 1：处理输入**

```
for i in 0..nb_iargs:
    arg_ct = ct.args[nb_oargs + i]
    tidx   = op.args[nb_oargs + i]
    required  = arg_ct.regs       // 允许的寄存器集合
    forbidden = i_allocated       // 已分配给前面输入的寄存器

    if arg_ct.ialias && is_dead(input) && !is_readonly(temp):
        // 输入死亡且可写 → 可以复用其寄存器给输出
        preferred = op.output_pref[alias_index]
        reg = temp_load_to(tidx, required, forbidden, preferred)
        i_reusable[i] = true
    else:
        reg = temp_load_to(tidx, required, forbidden, EMPTY)

    i_regs[i] = reg
    i_allocated |= reg
```

关键点：
- `forbidden` 累积确保不同输入分配到不同寄存器
- `ialias` 输入优先加载到输出偏好的寄存器（减少后续 mov）
- `is_readonly` 检查：全局变量、固定 temp、常量不可复用

**阶段 2：Fixup（重新读取 i_regs）**

```
i_allocated = EMPTY
for i in 0..nb_iargs:
    reg = ctx.temp(op.args[nb_oargs + i]).reg
    i_regs[i] = reg
    i_allocated |= reg
```

**为什么需要 fixup？** 当后面的输入分配触发驱逐时，前面输入
的寄存器可能已经改变。典型场景：

```
Shl t3, t1, t2  (约束: o1_i2_alias_fixed(R, R, RCX))

假设 t1 当前在 RCX，t2 当前在 RAX：
  input 0 (t1): ialias, required=R → 加载到 RCX, i_regs[0]=RCX
  input 1 (t2): fixed=RCX, required={RCX}
    → required & ~forbidden = {RCX} & ~{RCX} = EMPTY
    → 强制驱逐 RCX 的占用者 (t1)
    → evict_reg: t1 是局部 → mov t1 到空闲寄存器 (如 RDX)
    → t2 加载到 RCX, i_regs[1]=RCX

此时 i_regs[0] 仍然是 RCX（过时！），但 t1 实际在 RDX。
fixup 阶段重新读取：i_regs[0] = RDX（正确）。
```

**阶段 3：处理输出**

```
for k in 0..nb_oargs:
    arg_ct = ct.args[k]
    dst_tidx = op.args[k]

    if arg_ct.oalias:
        ai = arg_ct.alias_index
        if i_reusable[ai]:
            // 输入已死亡 → 直接复用其寄存器
            reg = i_regs[ai]
        else:
            // 输入仍活跃 → 复制输入到新寄存器，
            // 输出占据原寄存器
            old_reg = i_regs[ai]
            copy_reg = reg_alloc(allocatable,
                                 i_allocated | o_allocated,
                                 EMPTY)
            emit mov(copy_reg, old_reg)
            // 更新输入 temp 指向 copy_reg
            reg = old_reg

    elif arg_ct.newreg:
        // 输出不得与任何输入重叠
        reg = reg_alloc(required,
                        i_allocated | o_allocated,
                        EMPTY)
    else:
        // 普通输出
        reg = reg_alloc(required, o_allocated, EMPTY)

    assign(reg, dst_tidx)
    o_regs[k] = reg
    o_allocated |= reg
```

**oalias copy-away 示例**：

```
Sub t3, t1, t2  (oalias: output aliases input 0)

假设 t1 仍然活跃（后续还有使用）：
  → t1 在 RAX，t2 在 RBX
  → 不能直接复用 RAX（t1 还活着）
  → copy_reg = 分配空闲寄存器 (如 RCX)
  → emit: mov RCX, RAX  (t1 的值保存到 RCX)
  → t1.reg = RCX
  → output t3 占据 RAX (原 t1 的寄存器)
  → emit: sub RAX, RBX  (RAX = RAX - RBX)
```

**阶段 4：Fixup（输出可能驱逐/移动了输入）**

```
for i in 0..nb_iargs:
    temp = ctx.temp(op.args[nb_oargs + i])
    if temp.val_type == Reg:
        i_regs[i] = temp.reg
```

输出分配可能需要特定寄存器（如 MulS2 的 RAX），导致占据该
寄存器的输入被驱逐到其他寄存器。此 fixup 确保 `i_regs` 在
emit 时反映输入的实际位置。

**阶段 5：发射宿主代码**

```
backend.tcg_out_op(buf, ctx, op, &o_regs, &i_regs, &cargs)
```

此时所有约束已满足：
- oalias 输出与对应输入在同一寄存器
- 固定约束的输入在指定寄存器（如 RCX）
- newreg 输出不与任何输入重叠

codegen 可以直接发射最简指令序列。

**阶段 6：释放死亡输入**

```
for i in 0..nb_iargs:
    if life.is_dead(nb_oargs + i):
        tidx = op.args[nb_oargs + i]
        if tidx not in op.args[0..nb_oargs]:  // 跳过别名输出
            temp_dead_input(tidx)
```

死亡输入在 emit 之后释放（而非之前），确保 `i_regs` 在代码
发射期间始终有效。`temp_dead_input` 使用 `reg_to_temp` 守卫：
仅当寄存器仍映射到该输入时才释放，避免释放已被别名输出接管
的寄存器。

**阶段 7：释放死亡输出**

```
for k in 0..nb_oargs:
    if life.is_dead(k):
        temp_dead(op.args[k])
```

**阶段 8：同步全局变量**

```
for i in 0..nb_iargs:
    if life.is_sync(nb_oargs + i):
        temp_sync(op.args[nb_oargs + i])
```

活跃性分析标记了哪些全局变量在此 op 后需要同步回内存。

#### 4.3.5 寄存器分配策略（`reg_alloc`）

`reg_alloc(required, forbidden, preferred)` 使用 4 级优先策略：

```
candidates = required & allocatable & ~forbidden

1. preferred & candidates & free_regs  → 最优：偏好且空闲
2. candidates & free_regs              → 次优：任意空闲
3. candidates.first()                  → 驱逐：evict 占用者
4. (required & allocatable).first()    → 强制驱逐（见 4.3.6）
```

第 1 级的 `preferred` 来自 `op.output_pref`，由活跃性分析
设置，用于减少 ialias 输入到输出之间的 mov。

#### 4.3.6 驱逐机制（`evict_reg`）

当需要的寄存器被占用时，驱逐占用者：

| 占用者类型 | 驱逐策略 |
|-----------|---------|
| 全局变量 | `temp_sync` 写回内存 → 标记 `Mem` → 释放寄存器 |
| 局部 temp | `mov` 到另一个空闲寄存器 → 更新映射 |
| 固定 temp | 不应发生（固定 temp 的寄存器不在 allocatable 中） |

**强制驱逐**：当 `candidates` 为空（所有满足约束的寄存器都在
forbidden 中），说明存在固定约束冲突。此时忽略 forbidden 集合，
从 `required & allocatable` 中选择并驱逐。这只在固定约束场景
发生（如 input0 占据 RCX，input1 要求 RCX）。

驱逐后，被驱逐的 temp 的 `reg` 字段已更新，但调用者的
`i_regs[]` 数组仍持有旧值。这就是 fixup 阶段存在的原因。

#### 4.3.7 具体示例：Shl 的完整分配流程

```
IR:  Shl t3, t1, t2   (I64)
约束: o1_i2_alias_fixed(R_NO_RCX, R_NO_RCX, RCX)
  args[0] = t3 (output, oalias input 0, regs=R_NO_RCX)
  args[1] = t1 (input, ialias output 0, regs=R_NO_RCX)
  args[2] = t2 (input, fixed RCX)

初始状态: t1 在 RAX, t2 在 RBX, t1 和 t2 均在此 op 后死亡
```

**Step 1 — 处理输入**：

```
input 0 (t1): ialias=true, dead=true, !readonly
  required = R_NO_RCX (排除 RCX 的可分配寄存器)
  forbidden = EMPTY
  preferred = output_pref[0]
  → t1 已在 RAX (满足 required) → i_regs[0] = RAX
  → i_reusable[0] = true
  → i_allocated = {RAX}

input 1 (t2): fixed RCX
  required = {RCX}
  forbidden = {RAX}
  → required & ~forbidden = {RCX} (RCX 空闲)
  → t2 在 RBX，不满足 required
  → temp_load_to: emit mov RCX, RBX
  → i_regs[1] = RCX
  → i_allocated = {RAX, RCX}
```

**Step 2 — Fixup**：

```
重新读取: t1.reg = RAX ✓, t2.reg = RCX ✓
（本例无冲突，fixup 无变化）
```

**Step 3 — 处理输出**：

```
output 0 (t3): oalias, alias_index=0
  i_reusable[0] = true → reg = i_regs[0] = RAX
  → assign(RAX, t3)
  → o_regs[0] = RAX
```

**Step 4 — Fixup + Emit**：

```
i_regs fixup: 无变化（输出未驱逐输入）
backend.tcg_out_op(Shl, oregs=[RAX], iregs=[RAX, RCX])
  → emit: shl RAX, cl    (一条指令，无需 mov/push/pop)
```

**Step 5 — 释放死亡输入**：

```
t1 dead, 但 t1==t3 (别名) → 跳过
t2 dead → temp_dead_input(t2): 释放 RCX
```

`R_NO_RCX` 约束确保 input0/output 不会被分配到 RCX，从根本上
避免了 output 与 fixed shift-count 的寄存器冲突。

#### 4.3.8 与 QEMU 的差异

| 方面 | QEMU | tcg-rs |
|------|------|--------|
| 溢出 | 支持溢出到栈帧 `CPU_TEMP_BUF` | 不支持（14 GPR 足够） |
| 立即数约束 | `re`/`ri` 允许立即数直接编码 | 所有输入必须在寄存器中 |
| 输出偏好 | `output_pref` 由约束系统设置 | 由活跃性分析设置 |
| 常量输入 | 可内联到指令编码 | 必须先 `movi` 到寄存器 |
| 内存输入 | 部分指令支持 `[mem]` 操作数 | 必须先 `ld` 到寄存器 |

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

`tests/src/integration/mod.rs` 使用最小 RISC-V CPU 状态
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

### 4.6 前端翻译框架

#### 4.6.1 decodetree 解码器生成器

`decodetree` crate 实现了 QEMU 的 decodetree 工具的 Rust 版本，解析 `.decode` 文件并生成 Rust 解码器代码。

**输入**：`frontend/src/riscv/insn32.decode`（RV64IMAFDC 指令模式）

**生成的代码**：
- `Args*` 结构体：每个参数集对应一个结构体（如 `ArgsR { rd, rs1, rs2 }`）
- `extract_*` 函数：从 32 位指令字中提取字段（支持多段拼接、符号扩展）
- `Decode<Ir>` trait：每个模式对应一个 `trans_*` 方法
- `decode()` 函数：if-else 链按 fixedmask/fixedbits 匹配指令

**构建集成**：`frontend/build.rs` 在编译时调用 `decodetree::generate()`，输出到 `$OUT_DIR/riscv32_decode.rs`，通过 `include!` 宏引入。

#### 4.6.2 TranslatorOps trait

`frontend/src/lib.rs` 定义了架构无关的翻译框架：

```rust
trait TranslatorOps {
    type Disas;
    fn init_disas_context(ctx: &mut Self::Disas, ir: &mut Context);
    fn tb_start(ctx: &mut Self::Disas, ir: &mut Context);
    fn insn_start(ctx: &mut Self::Disas, ir: &mut Context);
    fn translate_insn(ctx: &mut Self::Disas, ir: &mut Context);
    fn tb_stop(ctx: &mut Self::Disas, ir: &mut Context);
}
```

`translator_loop()` 实现了 QEMU `accel/tcg/translator.c` 中的翻译循环：`tb_start → (insn_start + translate_insn)* → tb_stop`。

#### 4.6.3 RISC-V 前端

**CPU 状态**（`riscv/cpu.rs`）：

```rust
#[repr(C)]
struct RiscvCpu {
    gpr: [u64; 32],     // x0-x31
    fpr: [u64; 32],     // f0-f31 (raw bits, NaN-boxed)
    pc: u64,
    guest_base: u64,
    load_res: u64,       // LR 保留地址
    load_val: u64,       // LR 加载值
    fflags: u64,         // 浮点异常标志
    frm: u64,            // 浮点舍入模式
    ustatus: u64,        // 用户态状态寄存器
    // uie, utvec, uscratch, uepc, ucause, utval, uip
}
```

**翻译上下文**（`riscv/mod.rs`）：`RiscvDisasContext` 将 32 个 GPR、32 个 FPR、PC 及浮点 CSR 注册为 TCG 全局变量（backed by `RiscvCpu` 字段），env 指针固定到 RBP。

**指令翻译**（`riscv/trans.rs`）：实现 `Decode<Context>` trait 的 `trans_*` 方法，覆盖 RV64IMAFDC 整数、浮点和压缩指令集，使用 QEMU 风格的 `gen_xxx` 辅助函数模式：

```rust
type BinOp = fn(&mut Context, Type, TempIdx, TempIdx, TempIdx) -> TempIdx;

fn gen_arith(&self, ir: &mut Context, a: &ArgsR, op: BinOp) -> bool;
fn gen_arith_imm(&self, ir: &mut Context, a: &ArgsI, op: BinOp) -> bool;
fn gen_shift_imm(&self, ir: &mut Context, a: &ArgsShift, op: BinOp) -> bool;
fn gen_arith_w(&self, ir: &mut Context, a: &ArgsR, op: BinOp) -> bool;
fn gen_branch(&mut self, ir: &mut Context, rs1: usize, rs2: usize, imm: i64, cond: Cond) -> bool;
```

每个 `trans_*` 方法成为一行调用，如 `trans_add → gen_arith(ir, a, Context::gen_add)`。

### 4.7 测试体系

| 测试类别 | 位置 | 数量 | 说明 |
|---------|------|------|------|
| decodetree 测试 | `tests/src/decodetree/` | 93 | 解析器、代码生成、字段提取、RVC |
| 核心单元测试 | `tests/src/core/` | 192 | types/opcodes/temps/labels/ops/context/TBs |
| 后端回归测试 | `tests/src/backend/` | 256 | x86-64 指令编码、代码缓冲区 |
| 前端翻译测试 | `tests/src/frontend/mod.rs` | 126 | RV32I/RV64I/RVC/RV32F 全流水线指令测试 |
| 差分测试 | `tests/src/frontend/difftest.rs` | 35 | 对比 QEMU qemu-riscv64 |
| 集成测试 | `tests/src/integration/` | 105 | 端到端 IR→执行 |
| 执行循环 | `tests/src/exec/` | 12 | TB 缓存、执行循环 |
| linux-user | `linux-user/tests/` | 6 | ELF 加载、客户程序执行 |

详细文档见 [`docs/testing.md`](testing.md)。

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
| `TCGType`                     | `enum Type`                    | `core/src/types.rs`             |
| `TCGTempVal`                  | `enum TempVal`                 | `core/src/types.rs`             |
| `TCGCond`                     | `enum Cond`                    | `core/src/types.rs`             |
| `MemOp`                       | `struct MemOp(u16)`            | `core/src/types.rs`             |
| `TCGRegSet`                   | `struct RegSet(u64)`           | `core/src/types.rs`             |
| `TCGOpcode` + DEF macros      | `enum Opcode`                  | `core/src/opcode.rs`            |
| `TCGOpDef`                    | `struct OpDef` + `OPCODE_DEFS` | `core/src/opcode.rs`            |
| `TCG_OPF_*`                   | `struct OpFlags`               | `core/src/opcode.rs`            |
| `TCGTempKind`                 | `enum TempKind`                | `core/src/temp.rs`              |
| `TCGTemp`                     | `struct Temp`                  | `core/src/temp.rs`              |
| `TCGLabel`                    | `struct Label`                 | `core/src/label.rs`             |
| `TCGLifeData`                 | `struct LifeData(u32)`         | `core/src/op.rs`                |
| `TCGOp`                       | `struct Op`                    | `core/src/op.rs`                |
| `TCGContext`                  | `struct Context`               | `core/src/context.rs`           |
| `TranslationBlock`            | `struct TranslationBlock`      | `core/src/tb.rs`                |
| `CPUJumpCache`                | `struct JumpCache`             | `core/src/tb.rs`                |
| `tcg_target_callee_save_regs` | `CALLEE_SAVED`                 | `backend/src/x86_64/regs.rs`    |
| `tcg_out_tb_start` (prologue) | `HostCodeGen::emit_prologue`   | `backend/src/x86_64/emitter.rs` |
| `tcg_code_gen_epilogue`       | `HostCodeGen::emit_epilogue`   | `backend/src/x86_64/emitter.rs` |
| `tcg_out_exit_tb`             | `X86_64CodeGen::emit_exit_tb`  | `backend/src/x86_64/emitter.rs` |
| `tcg_out_goto_tb`             | `X86_64CodeGen::emit_goto_tb`  | `backend/src/x86_64/emitter.rs` |
| `tcg_out_goto_ptr`            | `X86_64CodeGen::emit_goto_ptr` | `backend/src/x86_64/emitter.rs` |
| `tcg_gen_op*` (IR emission)   | `Context::gen_*`               | `core/src/ir_builder.rs`        |
| `liveness_pass_1`             | `liveness_analysis()`          | `backend/src/liveness.rs`       |
| `tcg_reg_alloc_op`            | `regalloc_op()`                | `backend/src/regalloc.rs`       |
| `TCGArgConstraint`            | `ArgConstraint`                | `backend/src/constraint.rs`     |
| `C_O*_I*` macros              | `o1_i2()` / `o1_i2_alias()` etc. | `backend/src/constraint.rs`  |
| `tcg_target_op_def`           | `op_constraint()`              | `backend/src/x86_64/constraints.rs` |
| `tcg_out_op` (dispatch)       | `HostCodeGen::tcg_out_op`      | `backend/src/x86_64/codegen.rs` |
| `tcg_out_mov`                 | `HostCodeGen::tcg_out_mov`     | `backend/src/x86_64/codegen.rs` |
| `tcg_out_movi`                | `HostCodeGen::tcg_out_movi`    | `backend/src/x86_64/codegen.rs` |
| `tcg_out_ld`                  | `HostCodeGen::tcg_out_ld`      | `backend/src/x86_64/codegen.rs` |
| `tcg_out_st`                  | `HostCodeGen::tcg_out_st`      | `backend/src/x86_64/codegen.rs` |
| `tcg_gen_code`                | `translate()`                  | `backend/src/translate.rs`      |
| `translator_loop`             | `translator_loop()`            | `frontend/src/lib.rs`           |
| `DisasContextBase`            | `DisasContextBase`             | `frontend/src/lib.rs`           |
| `disas_log` (decodetree)      | `decodetree::generate()`       | `decodetree/src/lib.rs`         |
| `target/riscv/translate.c`    | `RiscvDisasContext`            | `frontend/src/riscv/mod.rs`     |
| `trans_rvi.c.inc` (gen_xxx)   | `gen_arith/gen_branch/...`     | `frontend/src/riscv/trans.rs`   |

---

## 7. RISC-V 前端（RV64 用户态）

- 支持 RV64F/RV64D 浮点指令，包括浮点 load/store、算术运算、
  类型转换、比较/分类、FMA 系列（FMADD/FMSUB/FNMSUB/FNMADD）。
- 实现浮点相关用户态 CSR（`fflags`、`frm`、`fcsr`）及 U-mode
  状态/陷阱 CSR，带 FS 状态追踪（仅在写入 FPR 时标记 dirty）。
- 浮点运算通过 `gen_helper_call` 调用 `fpu.rs` 中的 C ABI
  辅助函数，由后端 `regalloc_call` 处理 caller-saved 寄存器
  保存/恢复。
