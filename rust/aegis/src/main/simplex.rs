use std::path::Path;
use std::sync::Arc;

use aegis::common::BotAdapter;
use aegis::core::security::SecurityManager;
use aegis::gateways::simplex::SimplexAdapter;
use anyhow::{Context, Result};
use secrecy::ExposeSecret;
use simploxide_client::prelude::AddressSettings;

use crate::bootstrap::EncryptedConfig;

/// SimpleX runtime handle: WebSocket Bot, event stream, Adapter.
pub struct SimplexHandle {
    /// 与 `adapter` 内部持有的句柄指向同一连接；保留以便后续直接调用 Bot API。
    #[expect(dead_code)]
    pub bot: simploxide_client::ws::Bot,
    pub events: simploxide_client::ws::EventStream,
    pub adapter: Arc<dyn BotAdapter>,
}

/// 以 0600 原子写入 bot 地址文件：写 tmp → fsync → rename。
///
/// 权限显式设置两次 —— 建 tmp 时的 `.mode()` 对**已存在**的 tmp 不生效，而
/// `truncate(true)` 会复用它；上次崩溃残留的 tmp 若是 0644，rename 出去就是 0644。
fn write_address_file(path: &Path, address: &str) -> std::io::Result<()> {
    use std::fs::Permissions;
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let tmp_path = path.with_extension("tmp");
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        file.set_permissions(Permissions::from_mode(0o600))?;
        writeln!(file, "{address}")?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)
}

/// 记录 bot 地址：打日志 + best-effort 落盘。
///
/// 任何失败只 warn，不返回错误 —— 地址拿不到不应让 bot 起不来。
fn record_address(address: &str, path: &Path) {
    log::info!("SimpleX bot 地址: {address}");
    if let Err(e) = write_address_file(path, address) {
        log::warn!("写入 SimpleX 地址文件失败（不影响 bot 运行）: {e}");
    }
}

/// 是否需要接入 SimpleX。
///
/// 无 flag 时的探测只看 `simplex_port`：管理员留空是**合法状态**（首次安装时 contactId
/// 尚未存在，见 `decode_port_and_admin` 与 `main.rs` 的形态判定），所以要求
/// `simplex_admin_id` 同时存在会把「已配端口但还没填管理员」错判成非 SimpleX 部署，
/// 进而退化到 Telegram 分支（无 token 时 `Bot::new` 直接 panic）。
pub fn has_simplex_config(encrypted_config: &EncryptedConfig, args: &[String]) -> bool {
    let explicit = args.iter().any(|a| a == "--simplex");
    explicit || encrypted_config.simplex_port.is_some()
}

/// 解析并解密 simplex_port / simplex_admin_id。
///
/// simplex_admin_id 允许缺失：首次安装时管理员尚未连接，无从得知自己的 contactId；
/// 缺省时 bot 照常启动，事件循环会把非管理员消息记日志并忽略（见 runtime.rs 的无管理员分支）。
/// 字段存在时仍严格校验 —— 畸形值（非整数或 <= 0）必须启动即失败。
fn decode_port_and_admin(
    security: &SecurityManager,
    encrypted_config: &EncryptedConfig,
) -> Result<(u16, Option<i64>)> {
    let decrypt = |field: &Option<Vec<u8>>, what: &str| -> Result<String> {
        let vec = security.decrypt(field.as_ref().with_context(|| format!("缺少 {what}"))?)?;
        Ok(String::from_utf8(vec.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("{what} 包含无效 UTF-8: {e}"))?
            .trim()
            .to_string())
    };

    let port: u16 = decrypt(&encrypted_config.simplex_port, "simplex_port")?
        .parse()
        .context("simplex_port 应为 1-65535 的整数")?;

    let admin_id = match &encrypted_config.simplex_admin_id {
        Some(_) => {
            let raw = decrypt(&encrypted_config.simplex_admin_id, "simplex_admin_id")?;
            let id: i64 = raw.parse().context("simplex_admin_id 应为整数 contactId")?;
            if id <= 0 {
                anyhow::bail!("simplex_admin_id 应为正整数 contactId，收到 {id}");
            }
            Some(id)
        }
        None => None,
    };

    Ok((port, admin_id))
}

/// 关闭 bot 地址的自动接受。
///
/// `auto_accept_with()` 的目的是「保留/创建地址」，副作用是每次启动都把 autoAccept
/// 重新打开。故必须在 connect 之后显式关闭，否则「永久关闭」只成立于首次安装。
///
/// 用 `undocumented`（flatten 的 `serde_json::Value`）注入**显式** `null`：
/// `AddressSettings.auto_accept = None` 会被 serde 省略该键（= 不修改），而设计 E2
/// 已实证必须 `autoAccept: null` 才真正关闭。
pub(crate) fn settings_with_auto_accept_disabled() -> AddressSettings {
    AddressSettings {
        business_address: false,
        auto_accept: None,
        auto_reply: None,
        undocumented: serde_json::json!({ "autoAccept": null }),
    }
}

pub async fn connect_simplex(
    security: &SecurityManager,
    encrypted_config: &EncryptedConfig,
    config_dir: &Path,
) -> Result<SimplexHandle> {
    let (port, _) = decode_port_and_admin(security, encrypted_config)?;

    let (bot, events) = simploxide_client::ws::BotBuilder::new("Aegis", port)
        .auto_accept_with(rust_i18n::t!("simplex.welcome").to_string())
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("连接 SimpleX WebSocket 失败: {e}"))?;

    // P2：autoAccept 永久关闭。失败即启动失败（fail closed）—— 这个控制是
    // 「地址泄露近乎无用」的全部依据，静默继续等于把公开地址留在自动接受状态。
    bot.configure_address(settings_with_auto_accept_disabled())
        .await
        .map_err(|e| anyhow::anyhow!("关闭 SimpleX autoAccept 失败，拒绝以不安全状态启动: {e}"))?;

    // connect() 内部已走完 setup_auto_accept，因此此刻地址必然已存在。
    // 读不到只 warn：地址缺失不应阻止 bot 启动（管理员仍可从日志排查）。
    // 地址文件路径由传入的 config_dir 派生，文件名以 paths.rs 的常量为唯一来源 ——
    // AEGIS_CONFIG_DIR 测试缝隙对落盘同样生效，两侧不会漂移。
    let address_path = config_dir.join(
        Path::new(aegis::core::paths::bot::SIMPLEX_ADDRESS_FILE)
            .file_name()
            .context("SIMPLEX_ADDRESS_FILE 缺少文件名")?,
    );
    match bot.address().await {
        Ok(address) => record_address(&address, &address_path),
        Err(e) => log::warn!("读取 SimpleX bot 地址失败（不影响 bot 运行）: {e}"),
    }

    let adapter: Arc<dyn BotAdapter> = Arc::new(SimplexAdapter::new(bot.clone()));
    Ok(SimplexHandle {
        bot,
        events,
        adapter,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_config() -> EncryptedConfig {
        EncryptedConfig {
            token: None,
            admin_id: None,
            totp_secret: None,
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            lang: None,
            matrix_recovery_key: None,
            simplex_port: None,
            simplex_admin_id: None,
        }
    }

    #[test]
    fn returns_false_when_no_simplex_config() {
        assert!(!has_simplex_config(&empty_config(), &[]));
    }

    #[test]
    fn returns_true_when_flag_present() {
        assert!(has_simplex_config(
            &empty_config(),
            &["--simplex".to_string()]
        ));
    }

    #[test]
    fn returns_true_when_both_fields_present() {
        let mut cfg = empty_config();
        cfg.simplex_port = Some(vec![1]);
        cfg.simplex_admin_id = Some(vec![1]);
        assert!(has_simplex_config(&cfg, &[]));
    }

    /// 管理员留空是合法状态（首次安装时 contactId 尚未存在），因此「只配端口」必须
    /// 被识别为 SimpleX。否则会退化到 Telegram 分支，而无 token 时 `Bot::new` 直接 panic。
    #[test]
    fn returns_true_when_only_port_present() {
        let mut cfg = empty_config();
        cfg.simplex_port = Some(vec![1]);
        assert!(has_simplex_config(&cfg, &[]));
    }

    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn write_address_file_writes_address_with_newline_and_0600() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        write_address_file(&path, "simplex:/contact#/?v=2-7&smp=smp%3A%2F%2Fabcd").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "simplex:/contact#/?v=2-7&smp=smp%3A%2F%2Fabcd\n"
        );
        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn write_address_file_truncates_previous_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        write_address_file(&path, &"a".repeat(500)).unwrap();
        write_address_file(&path, "short").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "short\n");
    }

    /// 只靠建文件时的 mode() 不够：它对已存在的 tmp 不生效，而 truncate(true) 会复用该文件。
    #[test]
    fn write_address_file_overrides_permissions_of_stale_tmp() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        let stale_tmp = dir.path().join("simplex_address.tmp");
        std::fs::write(&stale_tmp, b"leftover").unwrap();
        std::fs::set_permissions(&stale_tmp, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_address_file(&path, "simplex:/x").unwrap();

        assert_eq!(mode_of(&path), 0o600, "陈旧 tmp 的 0644 不得被 rename 出去");
        assert!(!stale_tmp.exists(), "tmp 应已被 rename 消费掉");
    }

    #[test]
    fn write_address_file_reports_error_when_parent_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("no-such-dir").join("simplex_address");
        assert!(write_address_file(&path, "simplex:/x").is_err());
    }

    #[test]
    fn record_address_writes_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("simplex_address");
        record_address("simplex:/contact#/?v=2-7", &path);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "simplex:/contact#/?v=2-7\n"
        );
    }

    /// 地址落盘失败不得影响 bot 启动 —— record_address 必须吞掉错误只 warn。
    #[test]
    fn record_address_swallows_io_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("no-such-dir").join("simplex_address");
        record_address("simplex:/contact#/?v=2-7", &path);
        assert!(!path.exists());
    }

    fn config_with(port: Option<&str>, admin: Option<&str>) -> (SecurityManager, EncryptedConfig) {
        let dir = tempfile::TempDir::new().unwrap();
        let security = SecurityManager::new(&dir.path().join(".key")).unwrap();
        let mut cfg = empty_config();
        cfg.simplex_port = port.map(|p| security.encrypt(p.as_bytes()).unwrap());
        cfg.simplex_admin_id = admin.map(|a| security.encrypt(a.as_bytes()).unwrap());
        (security, cfg)
    }

    #[test]
    fn decode_port_and_admin_allows_absent_admin() {
        let (security, cfg) = config_with(Some("5225"), None);
        let (port, admin) = decode_port_and_admin(&security, &cfg).unwrap();
        assert_eq!(port, 5225);
        assert_eq!(admin, None, "首次安装无管理员时必须放行");
    }

    #[test]
    fn decode_port_and_admin_parses_admin() {
        let (security, cfg) = config_with(Some("5225"), Some("42"));
        let (port, admin) = decode_port_and_admin(&security, &cfg).unwrap();
        assert_eq!(port, 5225);
        assert_eq!(admin, Some(42));
    }

    #[test]
    fn decode_port_and_admin_rejects_non_numeric_admin() {
        let (security, cfg) = config_with(Some("5225"), Some("abc"));
        assert!(decode_port_and_admin(&security, &cfg).is_err());
    }

    #[test]
    fn decode_port_and_admin_rejects_zero_admin() {
        let (security, cfg) = config_with(Some("5225"), Some("0"));
        assert!(decode_port_and_admin(&security, &cfg).is_err());
    }

    /// E2 实证：关闭 autoAccept 必须发**显式** `null`。
    /// `AddressSettings.auto_accept` 是 `Option` 且 `skip_serializing_if = is_none`，
    /// 传 `None` 只会省略该键（不修改），因此靠 flatten 的 `undocumented` 注入 null。
    #[test]
    fn disabled_auto_accept_settings_serialize_explicit_null() {
        let json = serde_json::to_string(&settings_with_auto_accept_disabled()).unwrap();
        assert_eq!(json, r#"{"businessAddress":false,"autoAccept":null}"#);
    }

    #[test]
    fn decode_port_and_admin_requires_port() {
        let (security, cfg) = config_with(None, None);
        assert!(decode_port_and_admin(&security, &cfg).is_err());
    }
}
