# Agent Skills 强制工作流（addyosmani/agent-skills）

无论任务是「实现新功能」、「重构」还是「修复 bug」，都必须严格遵守以下顺序，不得跳过任何步骤。

技能文件位于 `~/.pi/agent/git/github.com/addyosmani/agent-skills/skills/<name>/SKILL.md`；未自动加载时用 `read` 读取该文件后再执行。

1. **DEFINE**：需求不清先用 **interview-me**（一次一问）或 **idea-refine**（方案探索）；需求明确则用 **spec-driven-development** 产出 `SPEC.md`（目标、命令、结构、风格、测试、边界）。我明确批准相关文档前，禁止进入后续任何步骤。
2. **PLAN**：用 **planning-and-task-breakdown** 将设计拆成少量、可独立验证的原子任务，写入 `tasks/plan.md`，每个任务含精确文件路径、验收标准与验证步骤；计划批准后执行。
3. **BUILD**：用 **incremental-implementation** 薄切片逐片实现，一次一片；需要对齐官方文档时用 **source-driven-development**，上下文不足用 **context-engineering**。高风险或不可逆改动（鉴权、数据迁移、支付、删除、部署）用 **doubt-driven-development** 做对抗式复核并取得明确许可。
4. **VERIFY**：实现阶段严格使用 **test-driven-development**（RED → GREEN → REFACTOR）：先写失败测试并确认失败，再写最小实现并确认通过；测试失败或构建中断转 **debugging-and-error-recovery**（复现 → 定位 → 修复 → 加防护）。测试前编写的代码一律删除。
5. **REVIEW**：每个任务完成后用 **code-review-and-quality** 对照计划做五轴审查，按严重程度上报，Critical 问题阻塞后续进展；过于复杂用 **code-simplification**，涉及安全用 **security-and-hardening**，涉及性能用 **performance-optimization**。
6. **SHIP**：用 **git-workflow-and-versioning** 做原子提交与分支管理；用 **documentation-and-adrs** 记录决策；用 **shipping-and-launch** 收尾（预发布清单、监控、回滚方案）。涉及废弃/迁移用 **deprecation-and-migration**。

**自动读取规则**：  
如果任务涉及新功能、重构或修复，请在执行任何步骤前自动加载并使用对应 Agent Skills 技能（技能清单见 `using-agent-skills`）。如果技能未触发，请明确说明原因并尝试手动加载（`read` 对应 `SKILL.md`）。

**核心行为**（始终生效）：先显式声明假设再实现；遇到冲突或模糊立即停下澄清，不得擅自猜测；技术上有问题要直说并给出替代方案；主动抵制过度复杂（能更少行数就别更多）；只改任务要求的范围，不做无关重构；未通过验证（测试、构建、运行时证据）不得宣称完成。

**模式选择规则**：

| 模式       | 触发条件                                                   | 工作流                                                                       |
| ---------- | ---------------------------------------------------------- | ---------------------------------------------------------------------------- |
| **strict** | 新功能、重构、架构变更、数据库变更、安全逻辑、影响 >3 文件 | 完整流程：spec → plan → incremental+TDD → review → ship                      |
| **normal** | 标准 bugfix、中等复杂度任务、小功能                        | spec(可选) → plan → incremental → TDD → review（跳过 complete DEFINE）        |
| **rapid**  | 文档、注释、typo 修复、格式化、简单单文件修改              | implement → validate（不跑完整 spec/plan/review）                             |

完整技能链（按需取用，非每项必跑）：`interview-me` → `idea-refine` → `spec-driven-development` → `planning-and-task-breakdown` → `incremental-implementation` → `test-driven-development` → `code-review-and-quality` → `git-workflow-and-versioning` → `documentation-and-adrs` → `shipping-and-launch`。

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

## CodeGraph（快速代码阅读）

**适用场景**：理解函数上下文、阅读源码、查看调用链、评估修改影响范围。

- **`codegraph_explore`**：首选。一次调用获取相关符号的完整源码 + 调用路径。适用于"这个函数怎么工作的？"、"这组符号的关系是什么？"
- **`codegraph_node`**：替代 Read 工具。读文件的同时附带依赖信息。传入 `file` 参数可代替 Read 读文件。
- **`codegraph_search`**：快速定位符号位置（不包含源码）。

> CodeGraph 是高效的**代码阅读器**——最适合理解已有代码。

## codebase-memory-mcp CLI（项目分析引擎）

**适用场景**：架构分析、寻找重构目标、热点识别、跨模块依赖、语义搜索。

**⚠️ 使用 CLI 而非 MCP**：pi 的 codebase-memory-mcp MCP 工具当前不可用（配置指向无效二进制）。统一通过命令行调用：

```bash
codebase-memory-mcp cli <tool> --project <PROJECT> [--flag value ...]
```

- 项目名：先运行 `codebase-memory-mcp cli list_projects` 获取（本项目为 `home-fe-Dark-Wuthering_Waves_Private_Server`）。
- 性能优化：每次 cli 调用会启动临时 daemon，`codebase-memory-mcp daemon start` 可保持热进程、消除启动成本。

### 推荐查询模式

Cypher 通过 `query_graph` 执行，基础命令：

```bash
codebase-memory-mcp cli query_graph --project <PROJECT> --query "MATCH ... RETURN ..."
```

常用 Cypher（找巨型函数 / 热点 / 模块 / 类）：

```cypher
-- 找巨型函数（重构候选）
MATCH (f:Function) WHERE f.file_path CONTAINS "路径"
RETURN f.name, f.file_path, f.end_line - f.start_line AS length
ORDER BY length DESC LIMIT 10

-- 找热点（高 fan-in 瓶颈）
MATCH (f:Function)-[r:CALLS]-( )
WITH f, count(r) AS fan_in ORDER BY fan_in DESC LIMIT 10
RETURN f.name, f.file_path, f.start_line, fan_in

-- 查看模块结构
MATCH (m:Module) WHERE m.file_path CONTAINS "路径"
RETURN m.name, m.file_path ORDER BY m.file_path

-- 查看所有类/结构体
MATCH (c:Class) WHERE c.file_path CONTAINS "路径"
RETURN c.name, c.file_path, c.start_line ORDER BY c.file_path
```

CLI 工具速查（均需 `--project <PROJECT>`）：

- **`search_graph`**：自然语言语义搜索（BM25），适合模糊查询。`--name-pattern ".*Handler.*"` 按名匹配；`--query "telegram message send"` 按语义搜索。
- **`trace_path`**：调用链追踪。`--function-name <X> --direction inbound|outbound|both --depth N`。
- **`get_code_snippet`**：读取特定函数/类的源码。`--qualified-name <qn>`（需先用 search_graph 找到 qn）。
- **`get_architecture`**：获取项目架构总览（聚类/分层/热点/边界）。
- **`check_index_coverage`**：验证索引覆盖。`--paths "a.rs" --paths "b.rs"`（数组参数需重复 flag）或 `--scopes "."`。
- **`search_code`**：代码文本搜索。`--pattern <regex> --file-pattern *.rs`。
- **`list_projects` / `index_status` / `detect_changes` / `manage_adr`**：项目管理、索引状态、git diff 影响分析、ADR 管理。

> codebase-memory-mcp CLI 是强大的**项目分析器**——最适合做架构评估和重构规划。

## 选择策略

| 目标 | 推荐工具 | 原因 |
|------|---------|------|
| 理解某函数怎么工作的 | `codegraph_explore` | 一次调用 = 源码 + 调用链 |
| 读文件 + 看依赖 | `codegraph_node` | 替代 Read，附带 blast radius |
| 架构总览 / 模块清单 | `codebase-memory-mcp cli get_architecture` / `query_graph` | 完整的节点和关系查询 |
| 找巨型函数 / 重构目标 | `codebase-memory-mcp cli query_graph` | `ORDER BY length DESC` |
| 热点 / 瓶颈识别 | `codebase-memory-mcp cli query_graph` | fan-in 聚合查询 |
| 语义搜索（记不住符号名） | `codebase-memory-mcp cli search_graph --query` | BM25 语义搜索 |
| 跨服务 / 跨语言追踪 | `codebase-memory-mcp cli trace_path` | HTTP_CALLS 边 |
| 快速定位符号位置 | `codegraph_search` | 轻量快速 |

**黄金法则**：日常开发读代码用 CodeGraph（快、省 token）；做架构分析、重构评估、找瓶颈时用 `codebase-memory-mcp cli`。

**回退规则**：
仅当两个系统都不可用时，才回退到 `grep`/`Read` 等常规工具。

注意：存在 `.codegraph/` 但 `codegraph_*` 工具未加载时，优先检查 Pi 的 MCP 工具列表或 codegraph 扩展是否可用，而非直接回退。
