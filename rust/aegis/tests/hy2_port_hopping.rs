//! 集成测试：hysteria2 端口跳跃（port hopping）的完整生命周期与错误处理。
//!
//! 覆盖用户报告的问题：创建跳跃端口 hy2 后删除再创建，端口不会还原。
//! 通过公共 API 验证真实行为，无需任何测试钩子（删除路径已与服务重载分离）。

use aegis::core::singbox::SingBoxConfigManager;
use aegis::core::xray::port_allocator::PortAllocator;
use serde_json::json;
use std::path::Path;

/// 读取分配文件中的所有 hysteria2 锁定范围主端口
fn locked_main_ports(alloc_path: &Path) -> Vec<u16> {
    let content = std::fs::read_to_string(alloc_path).unwrap();
    let data: serde_json::Value = serde_json::from_str(&content).unwrap();
    data["locked_ranges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["protocol"].as_str() == Some("hysteria2"))
        .filter_map(|r| r["start"].as_u64().map(|p| p as u16))
        .collect()
}

/// 构造一个含 3 个 hysteria2 inbound 的批量配置（结构同服务器上的 batch_hy2_*.json）
fn write_multi_hy2_config(path: &Path, main_ports: &[u16]) {
    let inbounds: Vec<_> = main_ports
        .iter()
        .enumerate()
        .map(|(i, port)| {
            json!({
                "type": "hysteria2",
                "tag": format!("HYSTERIA2-{}-it", i + 1),
                "listen": "::",
                "listen_port": port,
                "users": [{"password": format!("it_pw_{}", i + 1)}],
                "tls": {
                    "enabled": true,
                    "server_name": "wwps",
                    "alpn": ["h3"],
                    "key_path": "/etc/wwps/wwps-box/certs/tls.key",
                    "certificate_path": "/etc/wwps/wwps-box/certs/tls.cer"
                }
            })
        })
        .collect();
    std::fs::write(
        path,
        serde_json::to_string_pretty(&json!({ "inbounds": inbounds })).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn released_range_is_reused_on_recreation() {
    // Arrange: 分配一个跳跃范围（创建）
    let dir = tempfile::tempdir().unwrap();
    let alloc_path = dir.path().join(".port_alloc");
    let (main_port, hop) = PortAllocator::allocate_hysteria2_at(&alloc_path)
        .await
        .unwrap();
    assert_eq!(main_port, 10000, "首个空闲范围应从 10000 开始");
    assert_eq!(hop, (10001, 10099));

    // Act: 删除配置（释放范围）后重新创建
    PortAllocator::release_hysteria2_range_at(&alloc_path, main_port)
        .await
        .unwrap();
    let (main_port2, hop2) = PortAllocator::allocate_hysteria2_at(&alloc_path)
        .await
        .unwrap();

    // Assert: 释放后重新创建应还原同一端口范围
    assert_eq!(main_port2, main_port, "删除释放后重建应还原同一主端口");
    assert_eq!(hop2, hop, "跳跃范围应一致");
}

#[tokio::test]
async fn unreleased_range_is_not_reused() {
    // Arrange: 分配一个跳跃范围，但不释放（模拟删除路径漏调 release 的旧行为）
    let dir = tempfile::tempdir().unwrap();
    let alloc_path = dir.path().join(".port_alloc");
    let (first, _) = PortAllocator::allocate_hysteria2_at(&alloc_path)
        .await
        .unwrap();

    // Act: 直接再次分配
    let (second, _) = PortAllocator::allocate_hysteria2_at(&alloc_path)
        .await
        .unwrap();

    // Assert: 未释放的范围被跳过 → 端口不断迁移、旧端口永不还原（bug 机制）
    assert_eq!(first, 10000);
    assert_eq!(second, 10100, "未释放的范围应被跳过");
}

#[tokio::test]
async fn delete_specific_handles_corrupt_config_and_removes_file() {
    // Arrange: 故意构造损坏 JSON 的 hysteria2 配置
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batch_hy2_corrupt.json");
    std::fs::write(&path, "{\"inbounds\": [ { \"type\": \"hysteria2\", ").unwrap();

    // Act: 删除路径应对解析错误宽容（跳过清理、文件照删、不崩溃）
    let result = SingBoxConfigManager::delete_specific_configuration(path.to_str().unwrap()).await;

    // Assert
    assert!(result.is_ok(), "损坏配置的删除应成功：{:?}", result);
    assert!(!path.exists(), "损坏配置文件应被删除");
}

#[tokio::test]
async fn delete_specific_removes_multi_inbound_config() {
    // Arrange: 3-inbound 批量 hy2 配置（旧代码只提取 inbounds[0] 的端口）
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batch_hy2_multi.json");
    write_multi_hy2_config(&path, &[20001, 20101, 20201]);

    // Act
    let result = SingBoxConfigManager::delete_specific_configuration(path.to_str().unwrap()).await;

    // Assert
    assert!(result.is_ok(), "多 inbound 删除应成功：{:?}", result);
    assert!(!path.exists(), "配置文件应被删除");
}

#[tokio::test]
async fn delete_specific_keeps_tuic_config_working() {
    // Arrange: 非 hysteria2 文件（tuic）——删除不应触发 hysteria2 清理
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batch_tuic.json");
    let json = json!({
        "inbounds": [
            {"type": "tuic", "listen_port": 30001, "uuid": "u", "password": "p"}
        ]
    });
    std::fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();

    // Act
    let result = SingBoxConfigManager::delete_specific_configuration(path.to_str().unwrap()).await;

    // Assert
    assert!(result.is_ok(), "tuic 文件删除应成功：{:?}", result);
    assert!(!path.exists(), "tuic 配置文件应被删除");
}

#[tokio::test]
async fn delete_flow_releases_allocated_ranges_end_to_end() {
    // 端到端：分配 3 个范围 → 生成 3-inbound 配置 → 删除配置 → 全部范围释放
    // Arrange: 分配 3 个跳跃范围（对应 3-inbound 批量配置）
    let dir = tempfile::tempdir().unwrap();
    let alloc_path = dir.path().join(".port_alloc");
    let mut main_ports = Vec::new();
    for _ in 0..3 {
        let (main_port, _) = PortAllocator::allocate_hysteria2_at(&alloc_path)
            .await
            .unwrap();
        main_ports.push(main_port);
    }
    let path = dir.path().join("batch_hy2_end_to_end.json");
    write_multi_hy2_config(&path, &main_ports);
    assert_eq!(
        locked_main_ports(&alloc_path).len(),
        3,
        "前置条件：3 个范围已锁定"
    );

    // Act: 删除配置（release 写入指定的分配文件）
    let result = SingBoxConfigManager::delete_specific_configuration_at(
        path.to_str().unwrap(),
        Some(&alloc_path),
    )
    .await;

    // Assert: 文件删除且所有锁定范围释放
    assert!(result.is_ok(), "删除应成功：{:?}", result);
    assert!(!path.exists(), "配置文件应被删除");
    assert!(
        locked_main_ports(&alloc_path).is_empty(),
        "删除后所有锁定范围均应释放（旧代码只释放 inbounds[0]）"
    );
}
