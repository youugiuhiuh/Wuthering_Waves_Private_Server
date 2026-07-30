# tgbot 重构检查清单 / Refactoring Checklist

> 使用此清单审查 rust/tgbot 代码，遵循函数式/指令式/声明式编程范式  
> Use this checklist to review rust/tgbot code for proper paradigm usage

---

## 1. 数据转换 / Data Transformation

### 函数式检查 / Functional Checks

- [ ] **F1** 是否使用了 `filter()` 替代手动 if 条件过滤？
- [ ] **F2** 是否使用了 `map()` 替代手动数据转换？
- [ ] **F3** 是否使用了 `collect()` 替代手动 `Vec::push()`？
- [ ] **F4** 是否避免了不必要的 `mut` 可变变量？
- [ ] **F5** 是否使用 `Iterator` 方法链而非 for 循环？

### 反模式检查 / Anti-Pattern Checks

- [ ] **FA1** 是否有手动 for 循环可以改为 Iterator 链？
- [ ] **FA2** 是否有不必要的 `let mut result = Vec::new()`？
- [ ] **FA3** 是否有字符串手动拼接可以用 `join()` 替代？

---

## 2. I/O 操作 / I/O Operations

### 指令式检查 / Imperative Checks

- [ ] **I1** 顺序 I/O 操作是否使用 `async/await`？
- [ ] **I2** 错误处理是否使用 `?` 操作符？
- [ ] **I3** 是否避免了嵌套的 `match` / `if` 错误处理？
- [ ] **I4** 多步骤 I/O 是否有清晰的步骤编号/注释？

### 反模式检查 / Anti-Pattern Checks

- [ ] **IA1** 是否避免了使用迭代器链处理有顺序依赖的 I/O？
- [ ] **IA2** 是否避免了对顺序 I/O 使用 `futures::stream`？
- [ ] **IA3** 是否有函数既做数据转换又做 I/O（应分离）？

---

## 3. 配置生成 / Configuration Generation

### 声明式检查 / Declarative Checks

- [ ] **D1** JSON 配置是否使用 `json!` 宏？
- [ ] **D2** 是否避免了手动 `serde_json::Map` 构建？
- [ ] **D3** UI 键盘是否使用声明式 `InlineKeyboardMarkup::new()`？
- [ ] **D4** 静态配置是否使用 `const` 声明？

### 反模式检查 / Anti-Pattern Checks

- [ ] **DA1** 是否有可以合并到 `json!` 宏的手动构建？
- [ ] **DA2** 是否有冗长的 if-else 链可以用 `match` 替代？

---

## 4. 错误处理 / Error Handling

### 规范检查 / Standard Checks

- [ ] **E1** 函数返回是否使用 `Result<T, anyhow::Error>`？
- [ ] **E2** 是否优先使用 `?` 操作符而非 `match`/`if let`？
- [ ] **E3** 错误消息是否使用中文（与项目一致）？
- [ ] **E4** 是否避免了 `unwrap()` 除非明确知道不会失败？

---

## 5. 模块特定检查 / Module-Specific Checks

### singbox/config.rs

- [ ] **S1** `batch_create_hysteria2` 是否分离了数据生成与副作用？
- [ ] **S2** 是否使用了函数式方法处理配置列表？
- [ ] **S3** 是否保持了声明式 `json!` 配置生成？

### port_allocator.rs

- [ ] **P1** `find_consecutive_range` 是否使用 Iterator 而非手动循环？
- [ ] **P2** `get_locked_ranges` 是否使用函数式映射？

### main.rs

- [ ] **M1** Callback 处理是否使用 `match` 而非 if-else 链？
- [ ] **M2** UI 键盘构建是否保持声明式？

### maintenance.rs

- [ ] **MT1** 顺序安装步骤是否保持指令式？
- [ ] **MT2** 配置常量是否使用声明式 `const`？

---

## 6. 代码审查清单 / Code Review Checklist

### 重构前 / Before Refactoring

```markdown
□ 确认函数的主要目的：
  - 数据转换？→ 函数式
  - I/O 操作？→ 指令式
  - 配置生成？→ 声明式

□ 检查当前实现是否使用了正确的范式
□ 确认修改不会破坏现有功能
```

### 重构中 / During Refactoring

```markdown
□ 保持函数签名不变（如果被外部调用）
□ 确保错误处理行为一致
□ 验证测试仍然通过
```

### 重构后 / After Refactoring

```markdown
□ 运行 cargo build 确保编译通过
□ 运行 cargo clippy 检查代码风格
□ 检查是否有新的警告
□ 验证功能正常工作
```

---

## 7. 快速决策指南 / Quick Decision Guide

```
需要做数据转换吗？/ Need data transformation?
    │
    ├─ Yes → 使用函数式 / Use Functional
    │         .filter().map().collect()
    │
    └─ No → 需要 I/O 或状态变更吗？/ Need I/O or state mutation?
            │
            ├─ Yes → 使用指令式 / Use Imperative
            │         async/await, sequential steps
            │
            └─ No → 是配置或 UI 吗？/ Configuration or UI?
                    │
                    ├─ Yes → 使用声明式 / Use Declarative
                    │         json! macros, match
                    │
                    └─ No → 考虑是否需要重构
```

---

## 8. 优先级 / Priority

| 检查项 | 优先级 | 文件位置 |
|--------|--------|----------|
| F1-F5 | P0 | 所有数据转换函数 |
| I1-I4 | P0 | 所有 async I/O 函数 |
| D1-D4 | P1 | config.rs, singbox/config.rs |
| E1-E4 | P0 | 所有错误处理 |
| S1-S3 | P1 | singbox/config.rs |
| P1-P2 | P2 | port_allocator.rs |
| M1-M2 | P1 | main.rs |

---

## 9. 参考示例 / Reference Examples

### 函数式正确示例

```rust
// ✅ Good
pub fn process_items(items: &[Item]) -> Vec<Processed> {
    items
        .iter()
        .filter(|item| item.is_valid())
        .map(|item| item.process())
        .collect()
}
```

### 指令式正确示例

```rust
// ✅ Good
async fn setup_service() -> Result<()> {
    check_prerequisites().await?;
    initialize_directories().await?;
    configure_service().await?;
    start_service().await?;
    Ok(())
}
```

### 声明式正确示例

```rust
// ✅ Good
let config = json!({
    "log": {"level": "warning"},
    "inbounds": inbounds,
});
```

---

*Last updated: 2026-04-25*
*Version: 1.0*