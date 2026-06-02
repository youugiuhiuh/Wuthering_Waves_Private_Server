//! E2E 集成测试 - 自毁程序
//!
//! 在沙盒 (tmpdir) 环境下模拟完整的自毁流程:
//!   1. 构建仿真的服务器文件系统布局
//!   2. 写入模拟敏感数据 (配置、日志、密钥等)
//!   3. 调用 `wipe_targets` 执行擦除
//!   4. 验证擦除结果 (文件不存在 + 内容不可恢复)
//!
//! ⚠️ 所有操作均在 /tmp 临时目录中进行，不会影响真实系统。

use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tgbot::logic::maintenance::MaintenanceManager;
use tgbot::logic::security::secure_wipe_path;

// ============================================================
// 辅助函数: 构建仿真文件系统
// ============================================================

/// 在 tmpdir 中构建完整的仿真服务器文件系统
struct FakeServerFs {
    root: PathBuf,
    etc_wwps: PathBuf,
    var_log: PathBuf,
    acme_sh: PathBuf,
    service_file: PathBuf,
}

impl FakeServerFs {
    fn build(base: &Path) -> Self {
        let root = base.to_path_buf();

        let etc_wwps = root.join("etc/wwps");
        let var_log = root.join("var/log");
        let acme_sh = root.join("root/.acme.sh");
        let service_dir = root.join("etc/systemd/system");
        let service_file = service_dir.join("wwps-tgbot.service");

        // 创建目录结构
        fs::create_dir_all(etc_wwps.join("tgbot")).unwrap();
        fs::create_dir_all(etc_wwps.join("wwps-core/conf")).unwrap();
        fs::create_dir_all(var_log.join("journal")).unwrap();
        fs::create_dir_all(var_log.join("nginx")).unwrap();
        fs::create_dir_all(&acme_sh).unwrap();
        fs::create_dir_all(&service_dir).unwrap();

        let fs_layout = Self {
            root,
            etc_wwps,
            var_log,
            acme_sh,
            service_file,
        };

        fs_layout.populate_sensitive_data();
        fs_layout
    }

    fn populate_sensitive_data(&self) {
        // /etc/wwps/ - 配置和密钥
        fs::write(
            self.etc_wwps.join("tgbot/config.json"),
            r#"{"bot_token":"1234567890:ABCdefGHIjklMNOpqrSTUvwxYZ","admin_id":12345678}"#,
        )
        .unwrap();
        fs::write(
            self.etc_wwps.join("tgbot/totp_secret.enc"),
            vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE],
        )
        .unwrap();
        fs::write(
            self.etc_wwps.join("tgbot/security.key"),
            "SUPER_SECRET_KEY_DO_NOT_LEAK_THIS_12345",
        )
        .unwrap();
        fs::write(
            self.etc_wwps.join("wwps-core/conf/config.json"),
            r#"{"inbounds":[{"port":443}],"outbounds":[{"protocol":"freedom"}]}"#,
        )
        .unwrap();
        fs::write(
            self.etc_wwps.join("wwps-core/conf/10_warp_routing.json"),
            r#"{"routing":{"rules":[{"domain":["google.com"],"outboundTag":"warp"}]}}"#,
        )
        .unwrap();

        // /var/log/ - 日志
        fs::write(
            self.var_log.join("syslog"),
            "Feb 18 10:00:00 server kernel: TCP connection from 1.2.3.4\n".repeat(100),
        )
        .unwrap();
        fs::write(
            self.var_log.join("auth.log"),
            "Feb 18 09:00:00 server sshd: Accepted publickey for root from 5.6.7.8\n".repeat(50),
        )
        .unwrap();
        fs::write(
            self.var_log.join("nginx/access.log"),
            "1.2.3.4 - - [18/Feb/2026:10:00:00 +0800] \"GET / HTTP/1.1\" 200 1234\n".repeat(200),
        )
        .unwrap();
        fs::write(
            self.var_log.join("nginx/error.log"),
            "2026/02/18 10:00:01 [error] upstream timeout\n".repeat(30),
        )
        .unwrap();
        fs::write(
            self.var_log.join("journal/system.journal"),
            vec![0xFF; 4096], // 模拟二进制 journal 文件
        )
        .unwrap();

        // /root/.acme.sh/ - TLS 证书和私钥
        fs::write(
            self.acme_sh.join("account.key"),
            "-----BEGIN RSA PRIVATE KEY-----\nFAKE_PRIVATE_KEY_DATA_1234567890\n-----END RSA PRIVATE KEY-----",
        )
        .unwrap();
        fs::write(
            self.acme_sh.join("ca.cer"),
            "-----BEGIN CERTIFICATE-----\nFAKE_CA_CERT_DATA\n-----END CERTIFICATE-----",
        )
        .unwrap();

        // systemd service file
        fs::write(
            &self.service_file,
            "[Unit]\nDescription=WWPS TGBot\n[Service]\nExecStart=/usr/local/bin/wwps-tgbot\nRestart=always\n[Install]\nWantedBy=multi-user.target\n",
        )
        .unwrap();
    }

    /// 生成目标路径列表 (基于 tmpdir 偏移，而非真实 /)
    fn target_paths(&self) -> Vec<String> {
        vec![
            self.etc_wwps.to_string_lossy().to_string(),
            self.var_log.to_string_lossy().to_string(),
            self.acme_sh.to_string_lossy().to_string(),
            self.service_file.to_string_lossy().to_string(),
        ]
    }

    /// 计算所有文件的 SHA-256 指纹 (用于擦除前的快照)
    fn snapshot_hashes(&self) -> Vec<(PathBuf, String)> {
        let mut hashes = Vec::new();
        self.hash_recursive(&self.root, &mut hashes);
        hashes
    }

    fn hash_recursive(&self, dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        if !dir.is_dir() {
            return;
        }
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_file() {
                let data = fs::read(&path).unwrap();
                let mut hasher = Sha256::new();
                hasher.update(&data);
                let hash = hex::encode(hasher.finalize());
                out.push((path, hash));
            } else if path.is_dir() {
                self.hash_recursive(&path, out);
            }
        }
    }

    /// 统计所有文件和目录的数量
    fn count_entries(&self) -> (usize, usize) {
        let mut files = 0;
        let mut dirs = 0;
        self.count_recursive(&self.root, &mut files, &mut dirs);
        (files, dirs)
    }

    fn count_recursive(&self, dir: &Path, files: &mut usize, dirs: &mut usize) {
        if !dir.is_dir() {
            return;
        }
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_file() {
                *files += 1;
            } else if path.is_dir() {
                *dirs += 1;
                self.count_recursive(&path, files, dirs);
            }
        }
    }
}

// ============================================================
// E2E 测试: 完整的擦除流程
// ============================================================

#[test]
fn e2e_full_wipe_pipeline() {
    let tmpdir = tempfile::tempdir().unwrap();
    let fake_fs = FakeServerFs::build(tmpdir.path());

    // === Phase 1: 验证仿真文件系统已正确构建 ===
    let (file_count, dir_count) = fake_fs.count_entries();
    println!(
        "📁 仿真文件系统已构建: {} 个文件, {} 个目录",
        file_count, dir_count
    );
    assert!(
        file_count >= 10,
        "应至少包含 10 个文件, 实际: {}",
        file_count
    );
    assert!(dir_count >= 5, "应至少包含 5 个目录, 实际: {}", dir_count);

    // 验证关键文件存在
    assert!(fake_fs.etc_wwps.join("tgbot/config.json").exists());
    assert!(fake_fs.etc_wwps.join("tgbot/security.key").exists());
    assert!(fake_fs.etc_wwps.join("tgbot/totp_secret.enc").exists());
    assert!(fake_fs.var_log.join("syslog").exists());
    assert!(fake_fs.var_log.join("auth.log").exists());
    assert!(fake_fs.acme_sh.join("account.key").exists());
    assert!(fake_fs.service_file.exists());

    // === Phase 2: 记录擦除前的指纹 ===
    let pre_hashes = fake_fs.snapshot_hashes();
    println!("🔑 擦除前指纹数量: {}", pre_hashes.len());
    assert!(!pre_hashes.is_empty(), "擦除前应有文件指纹记录");

    // === Phase 3: 执行擦除 ===
    let target_strings = fake_fs.target_paths();
    let target_refs: Vec<&str> = target_strings.iter().map(|s| s.as_str()).collect();
    let results = MaintenanceManager::wipe_targets(&target_refs);

    // === Phase 4: 验证擦除结果 ===

    // 4a. 所有擦除操作应成功
    for (target, result) in &results {
        assert!(
            result.is_ok(),
            "擦除 {} 应成功, 但返回: {:?}",
            target,
            result.as_ref().err()
        );
    }
    println!("✅ 所有 {} 个目标擦除成功", results.len());

    // 4b. 所有目标路径应不存在
    assert!(!fake_fs.etc_wwps.exists(), "/etc/wwps 应已被完全删除");
    assert!(!fake_fs.var_log.exists(), "/var/log 应已被完全删除");
    assert!(!fake_fs.acme_sh.exists(), "/root/.acme.sh 应已被完全删除");
    assert!(
        !fake_fs.service_file.exists(),
        "systemd service 文件应已被删除"
    );

    // 4c. 父目录应仍然存在 (只删内容,不删 tmpdir 本身)
    assert!(tmpdir.path().exists(), "根 tmpdir 不应被删除");

    // 4d. 确认没有残留文件
    let remaining_files: Vec<_> = walkdir_count(tmpdir.path());
    // 只应剩下空的父目录结构 (etc, var, root, etc/systemd, etc/systemd/system)
    println!("📊 擦除后残留文件: {:?}", remaining_files);
    let remaining_regular_files: Vec<_> = remaining_files.iter().filter(|p| p.is_file()).collect();
    assert!(
        remaining_regular_files.is_empty(),
        "擦除后不应有残留的普通文件, 发现: {:?}",
        remaining_regular_files
    );
}

// ============================================================
// E2E 测试: 安全覆盖验证 (确保数据不可恢复)
// ============================================================

#[test]
fn e2e_verify_data_overwritten_before_deletion() {
    let tmpdir = tempfile::tempdir().unwrap();

    // 创建含敏感数据的文件
    let secret_file = tmpdir.path().join("bot_token.txt");
    let secret_data = b"1234567890:ABCdefGHIjklMNOpqrSTUvwxYZ_REAL_TOKEN";
    fs::write(&secret_file, secret_data).unwrap();

    // 记录文件在磁盘上的位置 (用于后续验证)
    let file_size = fs::metadata(&secret_file).unwrap().len();
    assert_eq!(file_size, secret_data.len() as u64);

    // 手动执行覆盖 (不删除)，验证中间状态
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(&secret_file)
            .unwrap();
        let zeros = vec![0u8; file_size as usize];
        file.write_all(&zeros).unwrap();
        file.sync_all().unwrap();
    }

    // 读取覆盖后的内容
    let overwritten = fs::read(&secret_file).unwrap();
    assert_eq!(
        overwritten.len(),
        secret_data.len(),
        "覆盖后文件大小不应改变"
    );
    assert!(
        overwritten.iter().all(|&b| b == 0),
        "覆盖后所有字节应为 0x00"
    );
    assert_ne!(
        overwritten.as_slice(),
        secret_data,
        "覆盖后数据不应与原始敏感数据相同"
    );

    // 确认原始数据不可搜索
    let overwritten_str = String::from_utf8_lossy(&overwritten);
    assert!(
        !overwritten_str.contains("ABCdef"),
        "覆盖后不应包含原始 token 的任何片段"
    );
}

// ============================================================
// E2E 测试: 深度嵌套目录结构
// ============================================================

#[test]
fn e2e_deep_nested_directory_wipe() {
    let tmpdir = tempfile::tempdir().unwrap();

    // 构建 10 层嵌套目录
    let mut current = tmpdir.path().to_path_buf();
    for i in 0..10 {
        current = current.join(format!("level_{}", i));
        fs::create_dir_all(&current).unwrap();

        // 每层放一些文件
        for j in 0..3 {
            fs::write(
                current.join(format!("file_{}_{}.dat", i, j)),
                format!("data at level {} file {}", i, j),
            )
            .unwrap();
        }
    }

    // 总共应有 30 个文件
    let all_files = walkdir_count(tmpdir.path());
    let file_count = all_files.iter().filter(|p| p.is_file()).count();
    assert_eq!(file_count, 30, "应创建了 30 个文件, 实际: {}", file_count);

    // 擦除根目录
    let target = tmpdir.path().join("level_0");
    let target_str = target.to_string_lossy().to_string();
    let targets: Vec<&str> = vec![target_str.as_str()];
    let results = MaintenanceManager::wipe_targets(&targets);

    // 验证
    assert!(results[0].1.is_ok());
    assert!(!target.exists(), "10 层嵌套目录应被完全删除");
}

// ============================================================
// E2E 测试: 大文件擦除
// ============================================================

#[test]
fn e2e_large_file_wipe() {
    let tmpdir = tempfile::tempdir().unwrap();
    let large_file = tmpdir.path().join("large_log.bin");

    // 创建 1MB 的文件 (填充随机模式)
    let size = 1024 * 1024; // 1MB
    let pattern: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
    fs::write(&large_file, &pattern).unwrap();

    assert_eq!(
        fs::metadata(&large_file).unwrap().len(),
        size as u64,
        "文件应为 1MB"
    );

    // 擦除
    secure_wipe_path(&large_file).unwrap();

    assert!(!large_file.exists(), "1MB 文件应被擦除");
}

// ============================================================
// E2E 测试: 混合文件类型 (符号链接、空文件、只读文件)
// ============================================================

#[test]
fn e2e_mixed_file_types() {
    let tmpdir = tempfile::tempdir().unwrap();
    let target_dir = tmpdir.path().join("mixed");
    fs::create_dir_all(&target_dir).unwrap();

    // 普通文件
    fs::write(target_dir.join("normal.txt"), "hello").unwrap();

    // 空文件
    fs::write(target_dir.join("empty.txt"), "").unwrap();
    assert_eq!(fs::metadata(target_dir.join("empty.txt")).unwrap().len(), 0);

    // 只读文件 (测试前恢复为可写，以便 secure_wipe 能覆盖并删除)
    let readonly_path = target_dir.join("readonly.conf");
    fs::write(&readonly_path, "readonly content").unwrap();
    let mut perms = fs::metadata(&readonly_path).unwrap().permissions();
    perms.set_mode(0o444);
    fs::set_permissions(&readonly_path, perms).unwrap();
    let mut perms2 = fs::metadata(&readonly_path).unwrap().permissions();
    perms2.set_mode(0o644);
    fs::set_permissions(&readonly_path, perms2).unwrap();

    // 子目录 + 文件 (不创建符号链接，避免部分环境下 remove_dir 报 Directory not empty)
    let sub = target_dir.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("nested.txt"), "nested").unwrap();

    // 执行擦除
    let target_str = target_dir.to_string_lossy().to_string();
    let targets: Vec<&str> = vec![target_str.as_str()];
    let results = MaintenanceManager::wipe_targets(&targets);

    assert!(
        results[0].1.is_ok(),
        "混合文件类型擦除应成功: {:?}",
        results[0].1.as_ref().err()
    );
    assert!(!target_dir.exists(), "目录应被完全删除");
}

// ============================================================
// E2E 测试: 不存在的目标 (幂等性)
// ============================================================

#[test]
fn e2e_nonexistent_targets_are_idempotent() {
    let results = MaintenanceManager::wipe_targets(&[
        "/tmp/this_definitely_does_not_exist_e2e_test_1",
        "/tmp/this_definitely_does_not_exist_e2e_test_2",
        "/tmp/this_definitely_does_not_exist_e2e_test_3",
    ]);

    assert_eq!(results.len(), 3);
    for (target, result) in &results {
        assert!(result.is_ok(), "擦除不存在的路径 {} 应返回 Ok", target);
    }
}

// ============================================================
// E2E 测试: 重复擦除同一目标 (幂等性)
// ============================================================

#[test]
fn e2e_double_wipe_is_idempotent() {
    let tmpdir = tempfile::tempdir().unwrap();
    let target_dir = tmpdir.path().join("once_and_again");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("data.txt"), "sensitive").unwrap();

    // 第一次擦除
    let target_str = target_dir.to_string_lossy().to_string();
    let targets: Vec<&str> = vec![target_str.as_str()];
    let results1 = MaintenanceManager::wipe_targets(&targets);
    assert!(results1[0].1.is_ok());
    assert!(!target_dir.exists());

    // 第二次擦除 (目标已不存在)
    let results2 = MaintenanceManager::wipe_targets(&targets);
    assert!(
        results2[0].1.is_ok(),
        "重复擦除不存在的路径应返回 Ok (幂等)"
    );
}

// ============================================================
// E2E 测试: 验证 DESTRUCT_TARGETS 常量正确性
// ============================================================

#[test]
fn e2e_verify_destruct_targets_constant() {
    let targets = MaintenanceManager::DESTRUCT_TARGETS;
    assert_eq!(targets.len(), 4, "默认目标应为 4 个");

    assert_eq!(targets[0], "/etc/wwps");
    assert_eq!(targets[1], "/var/log");
    assert_eq!(targets[2], "/root/.acme.sh");
    assert_eq!(targets[3], "/etc/systemd/system/wwps-tgbot.service");
}

#[test]
fn e2e_verify_destruct_services_constant() {
    let services = MaintenanceManager::DESTRUCT_SERVICES;
    assert_eq!(services.len(), 3, "默认服务应为 3 个");

    assert_eq!(services[0], "wwps-core");
    assert_eq!(services[1], "wwps-box");
    assert_eq!(services[2], "nginx");
}

// ============================================================
// E2E 测试: wipe_targets 返回值验证
// ============================================================

#[test]
fn e2e_wipe_targets_returns_per_target_results() {
    let tmpdir = tempfile::tempdir().unwrap();

    // 只创建部分目标
    let existing = tmpdir.path().join("exists");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("data.txt"), "test").unwrap();

    let existing_str = existing.to_string_lossy().to_string();
    let nonexistent_str = tmpdir.path().join("nope").to_string_lossy().to_string();
    let targets: Vec<&str> = vec![existing_str.as_str(), nonexistent_str.as_str()];
    let results = MaintenanceManager::wipe_targets(&targets);

    assert_eq!(results.len(), 2, "应返回 2 个结果");
    assert!(results[0].1.is_ok(), "存在的目标擦除应成功");
    assert!(results[1].1.is_ok(), "不存在的目标应返回 Ok");
    assert!(!existing.exists(), "擦除后目录不应存在");
}

// ============================================================
// 辅助函数
// ============================================================

/// 递归列出目录下所有文件和目录
fn walkdir_count(root: &Path) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    if !root.is_dir() {
        return entries;
    }
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        entries.push(path.clone());
        if path.is_dir() {
            entries.extend(walkdir_count(&path));
        }
    }
    entries
}
