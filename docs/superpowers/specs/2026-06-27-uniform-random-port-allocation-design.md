# Uniform Random Port Allocation Design

## 问题

HY2 非跳跃模式下使用 `PortAllocator::allocate_port()` 进行顺序端口分配（从 10000 开始递增），而非随机端口。与 TUIC 的 `StdRng::from_entropy().gen_range(10000..60000)` 行为不一致。

此外，TUIC 的随机端口循环仅检查 `is_port_available()`（netstat），未检查 `is_port_in_locked_range()`（port_alloc 文件），可能漏掉已保存但尚未 reload 的 HY2 端口。

## 目标

统一所有协议的端口分配逻辑：使用随机端口 + 双重碰撞检查（`is_port_in_locked_range` + `is_port_available`）。

## 改动范围

### hy2_batch.rs

当前（非跳跃分支）：
```rust
let port = PortAllocator::allocate_port().await?;
(port, (port, port))
```

改为 KCP 模式的随机 + 双重检查：
```rust
let port = loop {
    let p = StdRng::from_entropy().gen_range(10000..60000);
    if PortAllocator::is_port_in_locked_range(p).await { continue; }
    if MaintenanceManager::is_port_available(p).await { break p; }
};
(port, (port, port))
```

需要添加 `rand` imports。

### tuic_batch.rs

当前：
```rust
let p = StdRng::from_entropy().gen_range(10000..60000);
if MaintenanceManager::is_port_available(p).await { break p; }
```

改为双重检查：
```rust
let p = StdRng::from_entropy().gen_range(10000..60000);
if PortAllocator::is_port_in_locked_range(p).await { continue; }
if MaintenanceManager::is_port_available(p).await { break p; }
```

### port_allocator.rs

删除 `allocate_port()` 及其关联的 `LockedRange` 和 `save_port_alloc` 调用。非跳跃 HY2 不再调用它，跳跃模式使用 `allocate_hysteria2()`。

## 不涉及

KCP、Reality Vision、XHTTP 已有双重检查：
- KCP: `kcp.rs:115-126`
- Reality/XHTTP: `generate_enhanced_config()` in `config.rs:286-317`

## 验证

- `cargo fmt` 通过
- `cargo clippy -- -D warnings` 无警告
- `cargo test` 全通过
- 无 `allocate_port` 残留引用
