# Superpowers 强制工作流（简洁版）

无论任务是「实现新功能」、「重构」还是「修复 bug」，都必须严格遵守以下顺序，不得跳过任何步骤：

1. 先使用 **brainstorming** 技能，通过苏格拉底式提问细化需求、探索方案，分段展示设计供验证，并保存设计文档。我明确批准设计前，禁止进入后续任何步骤。
2. 设计批准后，必须立即使用 **using-git-worktrees** 技能在新分支创建隔离工作树，运行项目初始化，并确认测试基线干净。
3. 工作树就绪后，使用 **writing-plans** 技能将设计拆解为细粒度任务（每个任务 2-5 分钟），每个任务须包含精确文件路径、完整代码和验证步骤。
4. 计划批准后，使用 **subagent-driven-development** 或 **executing-plans** 执行——每个任务分发独立子 Agent，经过「规格合规」与「代码质量」两阶段审查，或按批次执行并设置人工检查点。
5. 实现阶段严格使用 **test-driven-development** 技能，执行 RED → GREEN → REFACTOR 循环：先写失败测试、确认失败，再写最小实现、确认通过，最后提交。测试前编写的代码一律删除。
6. 每个任务完成后使用 **requesting-code-review** 对照计划审查代码，按严重程度上报问题，Critical 问题阻塞后续进展。
7. 所有任务完成后使用 **finishing-a-development-branch** 收尾：验证测试、选择处置方式（merge / PR / keep / discard），并清理工作树。

**自动读取规则**：  
如果任务涉及新功能、重构或修复，请在执行任何步骤前自动加载并使用对应 Superpowers 技能。如果技能未触发，请明确说明原因并尝试手动加载（use skill XXX）。

**模式选择规则**（由 main-workflow 技能强制执行）：

| 模式 | 触发条件 | 工作流 |
|------|---------|--------|
| **strict** | 新功能、重构、架构变更、数据库变更、安全逻辑、影响 >3 文件 | 完整流程：brainstorming → worktree → plans → subagent → TDD → review → finish |
| **normal** | 标准 bugfix、中等复杂度任务、小功能 | plans → executing → review（跳过 brainstorming/worktree，除非风险增加）|
| **rapid** | 文档、注释、typo 修复、格式化、简单单文件修改 | implement → validate（不调用 brainstorming/worktree/TDD/subagents）|

**语言特定规则**：

- 处理 Rust 代码时，必须加载 **rust-lint-format** 技能并在完成任务前执行强制规则
- 处理 Go 代码时，必须加载 **go-lint-format** 技能并在完成任务前执行强制规则
- 添加或删除 Go/Rust 依赖时，必须加载 **dependency-management** 技能，使用 `go get` / `cargo add` / `cargo remove` 命令，禁止直接编辑依赖文件

**上下文优化规则**：
- 避免不必要的仓库扫描
- 只加载相关技能（基于模式和语言）
- 最小化 token 使用
- 阻止 >200 行的 patch，拆分大更改
- 阻止不相关的重构
- 阻止修改 >3 文件，除非 strict 模式要求

此规则优先级最高，始终生效，不得违反。
