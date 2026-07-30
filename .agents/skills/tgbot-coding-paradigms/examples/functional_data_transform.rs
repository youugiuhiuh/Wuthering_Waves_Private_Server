//! Functional Data Transformation Examples / 函数式数据转换示例
//!
//! This file demonstrates correct and incorrect usage of functional paradigm
//! in rust/tgbot codebase.

use std::collections::HashSet;
use anyhow::Result;

// ============================================================================
// CORRECT EXAMPLES / 正确示例
// ============================================================================

/// ✅ Functional: 使用 Iterator 链进行数据转换
/// 场景: 从端口分配数据中提取范围
pub async fn get_locked_ranges() -> Vec<(u16, u16)> {
    let data = load_port_alloc_data().await;
    data.locked_ranges
        .iter()
        .map(|r| (r.start, r.end))
        .collect()
}

/// ✅ Functional: 端口范围搜索 - 表达"寻找什么"而非"如何寻找"
/// 场景: 在给定范围内查找连续的空闲端口
fn find_consecutive_range(occupied: &HashSet<u16>, size: u16) -> Result<u16> {
    const MIN_PORT: u16 = 10000;
    const MAX_PORT: u16 = 60000;

    (MIN_PORT..=(MAX_PORT.saturating_sub(size)))
        .find(|&start| {
            // 使用 all() 检查整个范围是否空闲
            (start..(start + size))
                .all(|port| !occupied.contains(&port))
        })
        .ok_or_else(|| anyhow::anyhow!(
            "在 {}-{} 范围内找不到连续的 {} 个空闲端口",
            MIN_PORT, MAX_PORT, size
        ))
}

/// ✅ Functional: 过滤并转换
/// 场景: 对目标列表进行安全擦除，返回结果
pub fn wipe_targets<'a>(targets: &'a [&'a str]) -> Vec<(&'a str, Result<()>)> {
    targets
        .iter()
        .map(|&target| {
            let path = std::path::Path::new(target);
            let result = if path.exists() {
                secure_wipe_path(path)
            } else {
                Ok(()) // 目标不存在视为成功
            };
            (target, result)
        })
        .collect()
}

/// ✅ Functional: 使用 filter_map 处理 Option
/// 场景: 从配置中提取有效的 inbound 配置
pub fn extract_valid_inbounds(configs: &[Config]) -> Vec<&Config> {
    configs
        .iter()
        .filter(|c| c.is_valid())
        .collect()
}

/// ✅ Functional: 使用 partition 分离数据
/// 场景: 根据状态分离配置
pub fn partition_by_status(configs: &[Config]) -> (Vec<&Config>, Vec<&Config>) {
    configs
        .iter()
        .partition(|c| c.is_active())
}

// ============================================================================
// ANTI-PATTERNS / 反模式
// ============================================================================

/// ❌ Imperative: 不必要的可变状态
/// 场景: 获取锁定的端口范围
pub async fn get_locked_ranges_bad() -> Vec<(u16, u16)> {
    let data = load_port_alloc_data().await;
    let mut ranges = Vec::new();  // ❌ 不必要的 mut
    for range in &data.locked_ranges {
        ranges.push((range.start, range.end));
    }
    ranges
}

/// ❌ Imperative: 手动循环实现 Iterator 功能
/// 场景: 查找连续端口范围
fn find_consecutive_range_bad(occupied: &HashSet<u16>, size: u16) -> Result<u16> {
    const MIN_PORT: u16 = 10000;
    const MAX_PORT: u16 = 60000;

    // ❌ 手动循环，焦点在"如何做"而非"做什么"
    for main_port in MIN_PORT..=(MAX_PORT.saturating_sub(size)) {
        let mut found = true;
        for port in main_port..(main_port + size) {
            if occupied.contains(&port) {
                found = false;
                break;
            }
        }
        if found {
            return Ok(main_port);
        }
    }
    anyhow::bail!("找不到空闲端口范围")
}

/// ❌ Imperative: 手动字符串拼接
/// 场景: 构建逗号分隔的端口列表
fn build_port_list_bad(ports: &[u16]) -> String {
    let mut result = String::new();
    for (i, port) in ports.iter().enumerate() {
        if i > 0 {
            result.push(',');
        }
        result.push_str(&port.to_string());
    }
    result
}

// ✅ Functional: 使用 join 构建字符串列表
fn build_port_list_good(ports: &[u16]) -> String {
    ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

// ============================================================================
// MOCK TYPES / 模拟类型
// ============================================================================

#[derive(Debug, Default)]
struct PortAllocData {
    locked_ranges Vec<LockedRange>,
}

#[derive(Debug)]
struct LockedRange {
    start: u16,
    end: u16,
    protocol: String,
    created_at: i64,
}

#[derive(Debug)]
struct Config {
    port: u16,
    active: bool,
    valid: bool,
}

impl Config {
    fn is_active(&self) -> bool {
        self.active
    }

    fn is_valid(&self) -> bool {
        self.valid
    }
}

// 模拟异步加载
async fn load_port_alloc_data() -> PortAllocData {
    PortAllocData::default()
}

// 模拟安全擦除
fn secure_wipe_path(path: &std::path::Path) -> Result<()> {
    Ok(())
}