# 仓库指南（Repository Guidelines）

## 项目结构与模块组织
- core/: IR 类型、操作码、临时变量、标签与 IR builder（gen_*）。
- backend/: 活跃分析、约束系统、寄存器分配与 x86-64 代码生成。
- tests/: 后端回归与端到端 IR/TB 执行测试。
- docs/: 设计与后端文档。

## 构建、测试与开发命令
    cargo build                 # 构建全部 crate
    cargo test                  # 运行全量测试
    cargo test -p tcg-tests     # 仅运行后端与集成测试
    cargo clippy -- -D warnings # 静态检查
    cargo fmt --check           # 格式检查
不使用 CI/CD 自动化；构建、测试、发布均为手工操作。

## 编码风格与命名规范
- 默认缩进 4 空格；若文件已有风格则保持一致。
- Rust 命名：模块与函数使用 snake_case，类型使用 CamelCase。
- 注释尽量少且用英文，只解释非显然逻辑。
- 变更以“小而明确”为优先，默认清理过时代码。

## 测试指南
- 使用 Rust 内置测试框架（#[test]）。
- 测试命名采用 test_*，保持用例窄、确定性强。
- 后端回归放在 tests/src/backend/，IR/TB 执行用例放在
  tests/src/integration/。
- 修复缺陷时必须补充覆盖该场景的回归测试。

## 提交与 PR 指南

Commit message 必须使用英文编写。格式如下：

```
module: subject

具体修改内容的详细说明。

Signed-off-by: Name <email>
```

## 角色职责与质量要求
- 主要职责：审查与 review 代码、编写测题、把关代码质量。
- 优先发现行为风险、回归可能与测试缺口，并给出可复现依据。

## 文档与参考
- 行为变化需同步更新 docs/。
- 对齐 QEMU 行为时，注明对应源码位置与约束来源。
