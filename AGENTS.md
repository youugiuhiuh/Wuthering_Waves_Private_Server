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

| 模式       | 触发条件                                                   | 工作流                                                                        |
| ---------- | ---------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **strict** | 新功能、重构、架构变更、数据库变更、安全逻辑、影响 >3 文件 | 完整流程：brainstorming → worktree → plans → subagent → TDD → review → finish |
| **normal** | 标准 bugfix、中等复杂度任务、小功能                        | plans → executing → review（跳过 brainstorming/worktree，除非风险增加）       |
| **rapid**  | 文档、注释、typo 修复、格式化、简单单文件修改              | implement → validate（不调用 brainstorming/worktree/TDD/subagents）           |

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
