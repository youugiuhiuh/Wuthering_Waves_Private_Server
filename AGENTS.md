# 强制工作流与 Agent 规范

> **执行骨架**：全流程采用 **agent-skills**（Addy Osmani）；任务执行与派发保留 **superpowers:subagent-driven-development** 与 **superpowers:dispatching-parallel-agents**。

---

## 1. 模式路由矩阵（最高优先级，首先判定）

| 模式 | 触发条件 | 执行链路 |
| :--- | :--- | :--- |
| **strict** | 新功能、重构、架构/DB变更、安全逻辑、**修改 >3 文件** | `interview-me` / `idea-refine` → `spec` → `worktree` → `plans` → **子代理并发派发** → `TDD` → `review` → `ship` |
| **normal** | 标准 Bugfix、中等复杂度、小调整 | `plans` → `incremental-implementation`（或单子代理） → `review` |
| **rapid** | 文档、注释、Typo、纯格式化、简单单文件修改 | 直接实施 `implement` → `validate`（跳过 spec/worktree/TDD） |

---

## 2. 核心执行阶段与并发铁律

1. **需求探索与设计**：使用 `interview-me` + `idea-refine` 苏格拉底提问，产出 `spec` 设计文档；**未获明确批准前禁止写代码**。
2. **环境隔离**：使用 `git-workflow-and-versioning` 在新分支创建 Worktree，验证测试基线干净。
3. **任务拆解**：使用 `planning-and-task-breakdown` 拆分为 2~5 分钟粒度任务；**必须显式标识依赖边界与可并行组**。
4. **任务执行（Strict 模式并发强制）**：
   - **必须优先使用子代理派发**（`subagent-driven-development`）。
   - **独立任务必须并发派发**（`dispatching-parallel-agents` / parallel subagents）。
   - **并发安全铁律**：每个子代理必须严格隔离文件/目录上下文，禁止并发子代理写入同一文件。
5. **测试驱动（TDD）**：严格红绿循环（RED → GREEN → REFACTOR）。测试前写好的代码一律视为无效草稿并删除。
6. **质量审查**：`code-review-and-quality` 逐任务审查；安全/鉴权/数据迁移必须加载 `doubt-driven-development` 对抗覆核。
7. **收尾发布**：`shipping-and-launch` 全量测试验证，处理分支（PR/merge）并清理 Worktree。

---

## 3. 语言与工程规则

- **Rust**：必须加载 `rust-lint-format`，执行 `cargo fmt`、`clippy`、`nextest`。
- **Go**：必须加载 `go-lint-format`，执行规范检查。
- **依赖管理**：必须加载 `dependency-management`，使用 `cargo add/remove` 或 `go get`，严禁直接手改依赖配置。
- **Diff 规模控制**：阻止单次提交 >200 行 Patch；大变更强制拆分为微任务批次。
- **作用域控制**：单次任务修改 >3 文件时，强制升格至 strict 模式执行。

---

## 4. 代码阅读与分析引擎（CodeGraph & MCP）

- **日常阅读代码**：首选 `codegraph_explore`（调用链+源码）或 `codegraph_node`（读文件+Blast Radius）。
- **架构分析 / 热点查找**：使用 MCP 工具，固定参数 `project="home-ub-Dark-Wuthering_Waves_Private_Server"`。
  - 找符号：`search_graph(project, name_pattern="...")`
  - 查调用：`trace_path(project, function_name="...", direction="inbound|outbound")`
  - 读源码：`get_code_snippet(project, qualified_name="...")`
  - 架构总览：`get_architecture(project, aspects=["hotspots","entry_points","layers"])`
- **注意**：`query_graph` 的 Cypher 不支持在 `RETURN` 中使用算术减法（请分别返回 `start_line` 与 `end_line`）。仅在两套图谱工具均不可用时方可回退至常规 `grep`/`read`。
