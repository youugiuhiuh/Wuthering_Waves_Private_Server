# Superpowers 强制工作流（简洁版）

无论任务是「实现新功能」、「重构」还是「修复 bug」，都必须严格遵守以下顺序，不得跳过任何步骤：

1. 先使用 **brainstorming** 技能，通过苏格拉底式提问细化需求和设计。我明确批准设计前，禁止进入后续任何技能。
2. 设计批准后，使用 **writing-plans** 技能输出小任务计划（每个任务 2-5 分钟）。
3. 计划批准后，必须立即使用 **using-git-worktrees** 技能创建隔离工作树。
4. 然后严格使用 **test-driven-development** 技能（RED → GREEN → REFACTOR）。
5. 实现阶段允许使用 **subagent-driven-development** 并行处理。
6. 完成后必须使用 **requesting-code-review** 和 **finishing-a-development-branch** 进行审查和收尾。

**自动读取规则**：  
如果任务涉及新功能、重构或修复，请自动加载并使用对应 Superpowers 技能。如果技能未触发，请明确说明原因并尝试手动加载（use skill XXX）。

此规则优先级最高，始终生效，不得违反。
