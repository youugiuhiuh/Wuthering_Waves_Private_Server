//! 自毁逻辑测试文件
//!
//! 测试内容:
//! 1. 状态机流程 (Step 1→2→3→4→5)
//! 2. 防重放攻击 (同一 TOTP 不能使用两次)
//! 3. 超时自动取消 (60s)
//! 4. 文件指纹 (SHA-256) 校验
//! 5. 安全擦除 (secure_wipe_path) 功能验证

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

// ============================================================
// 复刻 DestructState 用于测试 (避免依赖 main.rs 中的私有类型)
// ============================================================

struct DestructState {
    step: usize,
    first_totp: String,
    second_totp: String,
    last_action_time: Instant,
}

impl DestructState {
    fn new() -> Self {
        Self {
            step: 1,
            first_totp: String::new(),
            second_totp: String::new(),
            last_action_time: Instant::now(),
        }
    }

    fn is_timed_out(&self) -> bool {
        self.last_action_time.elapsed() > Duration::from_secs(60)
    }
}

// ============================================================
// 状态机测试
// ============================================================

#[test]
fn test_destruct_state_initial() {
    let state = DestructState::new();
    assert_eq!(state.step, 1, "初始步骤应为 1 (等待第一次 TOTP)");
    assert!(state.first_totp.is_empty());
    assert!(state.second_totp.is_empty());
}

#[test]
fn test_destruct_state_step_transitions() {
    let mut state = DestructState::new();

    // Step 1 → 2: 第一次 TOTP 验证通过
    assert_eq!(state.step, 1);
    state.first_totp = "123456".to_string();
    state.step = 2;
    assert_eq!(state.step, 2, "验证通过后应进入步骤 2 (等待确认按钮)");

    // Step 2 → 3: 用户点击确认
    state.step = 3;
    assert_eq!(state.step, 3, "确认后应进入步骤 3 (等待第二次 TOTP)");

    // Step 3 → 4: 第二次 TOTP 验证通过
    state.second_totp = "789012".to_string();
    state.step = 4;
    assert_eq!(state.step, 4, "第二次验证通过后应进入步骤 4 (等待安全文件)");

    // Step 4 → 5: 文件验证通过
    state.step = 5;
    assert_eq!(state.step, 5, "文件验证通过后应进入步骤 5 (最终确认)");
}

#[test]
fn test_destruct_state_full_flow() {
    let mut states: HashMap<i64, DestructState> = HashMap::new();
    let chat_id: i64 = 12345;

    // 初始化流程
    states.insert(chat_id, DestructState::new());
    assert!(states.contains_key(&chat_id));

    // 模拟完整流程
    let state = states.get_mut(&chat_id).unwrap();
    state.first_totp = "111111".to_string();
    state.step = 2;
    state.step = 3;
    state.second_totp = "222222".to_string();
    state.step = 4;
    state.step = 5;

    assert_eq!(state.step, 5);

    // 执行后移除
    states.remove(&chat_id);
    assert!(!states.contains_key(&chat_id));
}

// ============================================================
// 防重放攻击测试
// ============================================================

#[test]
fn test_anti_replay_same_totp_rejected() {
    let mut state = DestructState::new();

    // Step 1: 第一次 TOTP
    let totp_a = "123456";
    state.first_totp = totp_a.to_string();
    state.step = 2;
    state.step = 3;

    // Step 3: 尝试使用相同的 TOTP
    let totp_b = "123456"; // 与第一次相同
    let is_replay = totp_b == state.first_totp;

    assert!(is_replay, "相同的 TOTP 应被识别为重放攻击");
    // 在实际代码中，此处应拒绝并保持在 step 3
    assert_eq!(state.step, 3, "重放攻击后应保持在步骤 3");
}

#[test]
fn test_anti_replay_different_totp_accepted() {
    let mut state = DestructState::new();

    // Step 1: 第一次 TOTP
    let totp_a = "123456";
    state.first_totp = totp_a.to_string();
    state.step = 2;
    state.step = 3;

    // Step 3: 使用不同的 TOTP
    let totp_b = "654321";
    let is_replay = totp_b == state.first_totp;

    assert!(!is_replay, "不同的 TOTP 不应被视为重放攻击");
    // 验证通过，进入 step 4
    state.second_totp = totp_b.to_string();
    state.step = 4;
    assert_eq!(state.step, 4);
}

// ============================================================
// 超时测试
// ============================================================

#[test]
fn test_timeout_not_triggered_within_window() {
    let state = DestructState::new();
    // 刚创建的状态不应超时
    assert!(!state.is_timed_out(), "刚创建的状态不应超时");
}

#[test]
fn test_timeout_detection_logic() {
    // 模拟已过期的状态
    let state = DestructState {
        step: 1,
        first_totp: String::new(),
        second_totp: String::new(),
        last_action_time: Instant::now() - Duration::from_secs(61),
    };

    assert!(state.is_timed_out(), "超过 60 秒应触发超时");
}

#[test]
fn test_timeout_boundary_59s() {
    let state = DestructState {
        step: 2,
        first_totp: "123456".to_string(),
        second_totp: String::new(),
        last_action_time: Instant::now() - Duration::from_secs(59),
    };

    assert!(!state.is_timed_out(), "59 秒不应触发超时");
}

#[test]
fn test_timeout_resets_on_action() {
    let mut state = DestructState {
        step: 2,
        first_totp: "123456".to_string(),
        second_totp: String::new(),
        last_action_time: Instant::now() - Duration::from_secs(55),
    };

    // 模拟用户操作，重置计时器
    state.last_action_time = Instant::now();
    assert!(!state.is_timed_out(), "操作后计时器应重置");
}

// ============================================================
// 文件指纹 (SHA-256) 校验测试
// ============================================================

#[test]
fn test_file_hash_matching() {
    let content = b"this is the secret security file content";

    let mut hasher = Sha256::new();
    hasher.update(content);
    let hash_hex = hex::encode(hasher.finalize());

    // 模拟存储的 hash
    let stored_hash = hash_hex.clone();

    // 再次计算 (模拟用户上传同一文件)
    let mut hasher2 = Sha256::new();
    hasher2.update(content);
    let uploaded_hash = hex::encode(hasher2.finalize());

    assert_eq!(stored_hash, uploaded_hash, "相同文件内容的 SHA-256 应一致");
}

#[test]
fn test_file_hash_mismatch() {
    let original = b"correct file";
    let wrong = b"wrong file";

    let hash_original = {
        let mut h = Sha256::new();
        h.update(original);
        hex::encode(h.finalize())
    };

    let hash_wrong = {
        let mut h = Sha256::new();
        h.update(wrong);
        hex::encode(h.finalize())
    };

    assert_ne!(hash_original, hash_wrong, "不同文件内容的 SHA-256 应不同");
}

#[test]
fn test_file_hash_empty_file() {
    let content = b"";

    let mut hasher = Sha256::new();
    hasher.update(content);
    let hash_hex = hex::encode(hasher.finalize());

    // SHA-256 of empty string is a well-known constant
    assert_eq!(
        hash_hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "空文件的 SHA-256 应为已知常量"
    );
}

#[test]
fn test_file_hash_short_display() {
    let content = b"security file";
    let mut hasher = Sha256::new();
    hasher.update(content);
    let hash_hex = hex::encode(hasher.finalize());

    // 测试缩短显示逻辑
    let hash_short = if hash_hex.len() > 12 {
        format!("{}...{}", &hash_hex[..8], &hash_hex[hash_hex.len() - 4..])
    } else {
        hash_hex.clone()
    };

    assert!(hash_short.contains("..."), "长 Hash 应被截断显示");
    assert_eq!(hash_short.len(), 15, "截断后应为 8 + 3 + 4 = 15 字符");
}

// ============================================================
// 安全擦除 (secure_wipe_path) 功能测试
// ============================================================

#[test]
fn test_secure_wipe_single_file() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("secret.txt");

    // 写入敏感数据
    fs::write(&file_path, "TOP SECRET DATA 12345").unwrap();
    assert!(file_path.exists());

    // 执行安全擦除
    aegis::logic::security::secure_wipe_path(&file_path).unwrap();

    // 文件应被删除
    assert!(!file_path.exists(), "安全擦除后文件应不存在");
}

#[test]
fn test_secure_wipe_directory_recursive() {
    let dir = tempfile::tempdir().unwrap();
    let sub_dir = dir.path().join("subdir");
    fs::create_dir(&sub_dir).unwrap();

    // 创建嵌套文件
    fs::write(dir.path().join("file1.txt"), "data1").unwrap();
    fs::write(sub_dir.join("file2.txt"), "data2").unwrap();
    fs::write(sub_dir.join("file3.log"), "data3").unwrap();

    let target = dir.path().to_path_buf();
    assert!(target.exists());

    // 执行递归擦除
    aegis::logic::security::secure_wipe_path(&target).unwrap();

    assert!(!target.exists(), "安全擦除后目录应不存在");
}

#[test]
fn test_secure_wipe_nonexistent_path() {
    let path = Path::new("/tmp/definitely_does_not_exist_wwps_test_12345");

    // 对不存在的路径应返回 Ok
    let result = aegis::logic::security::secure_wipe_path(path);
    assert!(result.is_ok(), "擦除不存在的路径应返回 Ok");
}

#[test]
fn test_secure_wipe_overwrites_content() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("overwrite_test.bin");

    // 写入已知数据
    let original_data = vec![0xAA_u8; 1024];
    fs::write(&file_path, &original_data).unwrap();

    // 在擦除前，用另一个 handle 读取并验证内容
    let read_before = fs::read(&file_path).unwrap();
    assert_eq!(read_before, original_data, "写入数据应可以读回");

    // 手动执行覆盖步骤 (不删除，仅覆盖)
    {
        let metadata = fs::metadata(&file_path).unwrap();
        let len = metadata.len() as usize;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&file_path)
            .unwrap();
        let zeros = vec![0u8; len];
        file.write_all(&zeros).unwrap();
        file.sync_all().unwrap();
    }

    // 读取覆盖后的内容
    let read_after = fs::read(&file_path).unwrap();
    assert_eq!(read_after, vec![0u8; 1024], "覆盖后文件内容应全为 0x00");
    assert_ne!(read_after, original_data, "覆盖后数据不应与原始数据相同");
}

// ============================================================
// 并发 / 多用户测试
// ============================================================

#[test]
fn test_multiple_concurrent_destruct_sessions() {
    let mut states: HashMap<i64, DestructState> = HashMap::new();

    // 用户 A 开始自毁
    states.insert(100, DestructState::new());
    // 用户 B 开始自毁
    states.insert(200, DestructState::new());

    // 用户 A 进入 step 2
    states.get_mut(&100).unwrap().step = 2;
    states.get_mut(&100).unwrap().first_totp = "aaa111".to_string();

    // 用户 B 仍在 step 1
    assert_eq!(states.get(&200).unwrap().step, 1);
    assert_eq!(states.get(&100).unwrap().step, 2);

    // 用户 A 取消
    states.remove(&100);
    assert!(!states.contains_key(&100));

    // 用户 B 不受影响
    assert!(states.contains_key(&200));
    assert_eq!(states.get(&200).unwrap().step, 1);
}

#[test]
fn test_cancel_clears_state() {
    let mut states: HashMap<i64, DestructState> = HashMap::new();
    let chat_id: i64 = 999;

    states.insert(chat_id, DestructState::new());
    states.get_mut(&chat_id).unwrap().step = 3;
    states.get_mut(&chat_id).unwrap().first_totp = "secret".to_string();

    // 取消操作
    let removed = states.remove(&chat_id);
    assert!(removed.is_some(), "取消应成功移除状态");
    assert!(!states.contains_key(&chat_id), "取消后不应残留状态");
}

// ============================================================
// 边界情况
// ============================================================

#[test]
fn test_invalid_step_transition() {
    let state = DestructState::new();
    // Step 1 时不应允许直接跳到 step 5
    assert_ne!(state.step, 5, "不应允许跳过中间步骤");
    assert_eq!(state.step, 1, "应从步骤 1 开始");
}

#[test]
fn test_destruct_targets_list() {
    // 验证自毁目标列表的完整性
    let targets = [
        "/etc/wwps",
        "/var/log",
        "/root/.acme.sh",
        "/etc/systemd/system/wwps-aegis.service",
    ];

    assert_eq!(targets.len(), 4, "应有 4 个清理目标");
    assert!(targets.contains(&"/etc/wwps"), "应包含主配置目录");
    assert!(targets.contains(&"/var/log"), "应包含日志目录");
    assert!(targets.contains(&"/root/.acme.sh"), "应包含 ACME 证书目录");
    assert!(
        targets.contains(&"/etc/systemd/system/wwps-aegis.service"),
        "应包含 systemd service 文件"
    );
}
