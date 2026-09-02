use std::fs;
use std::path::Path;
use std::sync::Arc;

use aegis::common::BotAdapter;
use aegis::core::security::SecurityManager;
use aegis::gateways::matrix::MatrixAdapter;
use anyhow::{Context, Result};
use matrix_sdk::{
    Client as MatrixClient, Room, RoomState,
    config::SyncSettings,
    ruma::api::client::uiaa::{AuthData, MatrixUserIdentifier, Password, UserIdentifier},
};
use secrecy::ExposeSecret;
use secrecy::SecretString;

use crate::bootstrap::EncryptedConfig;

pub type MatrixHandle = (MatrixClient, Room, Arc<dyn BotAdapter>);

pub fn has_matrix_config(encrypted_config: &EncryptedConfig, args: &[String]) -> bool {
    let explicit_matrix = args.iter().any(|a| *a == "--matrix" || *a == "--all");
    explicit_matrix
        || (encrypted_config.matrix_homeserver.is_some()
            && encrypted_config.matrix_username.is_some()
            && encrypted_config.matrix_password.is_some()
            && encrypted_config.matrix_room_id.is_some())
}

/// 本地交叉签名私钥不完整时的下一步动作（connect_matrix P2 分层策略）。
///
/// 修复背景：`bootstrap_cross_signing_if_needed` 在已登录设备上永远返回 Ok 但不执行
/// （设备自身 identity 已存在），导致全新/损坏身份永远无法自建。分层策略：
/// 1) 本地已完整 → 无需操作；
/// 2) 有恢复密钥 → 优先从 secret storage 恢复（不破坏远端信任链）；
/// 3) 无恢复密钥且远端无身份 → 新建身份（bootstrap）；
/// 4) 无恢复密钥但远端已有身份 → 报错（需要正确恢复密钥或重置加密）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentityAction {
    /// 无需任何操作（本地交叉签名已完整）。
    None,
    /// 尝试用配置中的恢复密钥从 secret storage 恢复。
    TryRecovery,
    /// 远端无交叉签名身份，创建全新身份。
    BootstrapNew,
    /// 远端已有身份但本地无法恢复，需用户介入（正确恢复密钥 / 重置加密）。
    ErrorRequiresReset,
}

/// 初次决策：本地交叉签名不完整时，先做什么。
#[must_use]
fn next_identity_action(
    local_complete: bool,
    recovery_key_present: bool,
    remote_has_identity: bool,
) -> IdentityAction {
    if local_complete {
        IdentityAction::None
    } else if recovery_key_present {
        IdentityAction::TryRecovery
    } else if remote_has_identity {
        IdentityAction::ErrorRequiresReset
    } else {
        IdentityAction::BootstrapNew
    }
}

/// 恢复尝试之后的决策。
#[must_use]
fn after_recovery_action(recovered: bool, remote_has_identity: bool) -> IdentityAction {
    if recovered {
        IdentityAction::None
    } else if remote_has_identity {
        IdentityAction::ErrorRequiresReset
    } else {
        IdentityAction::BootstrapNew
    }
}

/// 尝试用恢复密钥从 secret storage 恢复交叉签名私钥；返回是否恢复成功。
async fn try_recover_with_key(
    client: &MatrixClient,
    security: &SecurityManager,
    encrypted_config: &EncryptedConfig,
) -> bool {
    let Some(rk_encrypted) = encrypted_config.matrix_recovery_key.as_ref() else {
        return false; // 未配置恢复密钥——属正常路径，无需日志
    };
    let Ok(rk_decrypted) = security.decrypt(rk_encrypted) else {
        println!("⚠ 恢复密钥解密失败（config.enc 中 matrix_recovery_key 损坏）");
        return false;
    };
    let Some(rk_str) = String::from_utf8(rk_decrypted.expose_secret().to_vec())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        println!("⚠ 恢复密钥为空或包含无效 UTF-8");
        return false;
    };

    let rk = SecretString::from(rk_str);
    let recovery = client.encryption().recovery();
    let result = match recovery.recover(rk.expose_secret()).await {
        Ok(_) => Ok(()),
        Err(matrix_sdk::encryption::recovery::RecoveryError::BackupExistsOnServer) => {
            recovery.recover_and_fix_backup(rk.expose_secret()).await
        }
        Err(e) => Err(e),
    };
    match result {
        Ok(_) => true,
        Err(e) => {
            println!("⚠ 恢复密钥导入失败(尝试其他路径): {e}");
            false
        }
    }
}

/// 远端无交叉签名身份时，创建全新身份（需要密码 UIA；若服务器不强制 UIA 亦可）。
///
/// 注意：不能使用 `bootstrap_cross_signing_if_needed`——它在已登录设备上永远跳过
/// （本机身份已存在即返回 Ok），这正是历史上“bootstrap 成功但什么都没做”的根因。
///
/// TODO: bootstrap 只创建 cross-signing 私钥，不创建 secret storage/恢复密钥备份；
/// 若 matrix_store 丢失将无法用恢复密钥重建（会落入 ErrorRequiresReset 死路）。
/// 后续可在此处 enable recovery 并打印一次新恢复密钥（review M7）。
async fn bootstrap_new_identity(
    client: &MatrixClient,
    matrix_username: &str,
    matrix_pwd: &str,
) -> Result<()> {
    client
        .encryption()
        .bootstrap_cross_signing(Some(AuthData::Password(Password::new(
            UserIdentifier::Matrix(MatrixUserIdentifier::new(matrix_username.to_owned())),
            matrix_pwd.to_owned(),
        ))))
        .await
        .context("bootstrap cross-signing 失败")?;
    let st = client.encryption().cross_signing_status().await;
    anyhow::ensure!(
        st.as_ref().is_some_and(|s| s.is_complete()),
        "bootstrap 后交叉签名仍不完整"
    );
    println!("✅ 全新交叉签名身份创建完成");
    Ok(())
}

/// Matrix 设备显示名：带机房归属城市前缀，如 `San Jose Aegis Matrix Bot`。
/// 城市不可用/为空时回退到默认名。
fn matrix_device_display_name(city: Option<&str>) -> String {
    match city.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("{c} Aegis Matrix Bot"),
        None => "Aegis Matrix Bot".to_string(),
    }
}

pub async fn connect_matrix(
    security: &SecurityManager,
    encrypted_config: &EncryptedConfig,
    config_dir: &Path,
) -> Result<MatrixHandle> {
    let decrypt_matrix = |field: &Option<Vec<u8>>| -> Result<String> {
        let vec = security.decrypt(field.as_ref().with_context(|| "缺少 Matrix 配置项")?)?;
        Ok(String::from_utf8(vec.expose_secret().to_vec())
            .map_err(|e| anyhow::anyhow!("Matrix 字段包含无效的 UTF-8: {}", e))?
            .trim()
            .to_string())
    };

    let matrix_homeserver = decrypt_matrix(&encrypted_config.matrix_homeserver)?;
    let matrix_username = decrypt_matrix(&encrypted_config.matrix_username)?;
    let matrix_pwd = decrypt_matrix(&encrypted_config.matrix_password)?;
    let matrix_room_id_str = decrypt_matrix(&encrypted_config.matrix_room_id)?;
    let matrix_store_passphrase = decrypt_matrix(&encrypted_config.matrix_store_passphrase)?;

    let store_path = config_dir.join("matrix_store");
    let client = MatrixClient::builder()
        .homeserver_url(&matrix_homeserver)
        .sqlite_store(&store_path, Some(&matrix_store_passphrase))
        .build()
        .await?;

    // ── Session restore (P0) ──
    let session_path = config_dir.join("matrix_session.json");
    if session_path.exists() {
        let session_json = fs::read_to_string(&session_path)?;
        let session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_str(&session_json)?;
        client
            .matrix_auth()
            .restore_session(session, matrix_sdk::store::RoomLoadSettings::default())
            .await?;
        println!("✅ Matrix 会话恢复成功: {matrix_username}");
    } else {
        // 设备名带机房归属城市（best-effort，查询失败回退默认名）
        let city = aegis::core::network::GeoIPService::new()
            .fetch_location()
            .await
            .map(|loc| loc.location.city)
            .ok();
        client
            .matrix_auth()
            .login_username(&matrix_username, &matrix_pwd)
            .initial_device_display_name(&matrix_device_display_name(city.as_deref()))
            .send()
            .await?;
        println!("✅ Matrix 登录成功: {matrix_username}");

        // Save session for future restarts
        if let Some(session) = client.matrix_auth().session() {
            let session_json = serde_json::to_string(&session)?;
            fs::write(&session_path, session_json)?;
            println!("✅ Matrix 会话已保存");
        }
    }

    // P1: Wait for E2EE initialization tasks (OlmMachine/身份就绪)
    client
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;

    // ── P2: 确保本地持有完整交叉签名私钥 ──
    // 分层策略由纯函数 next_identity_action / after_recovery_action 驱动：
    //   1) 本地已完整 → 直接通过
    //   2) 不完整且有恢复密钥 → 先用恢复密钥从 secret storage 恢复（不破坏信任链）
    //   3) 仍不完整且远端无交叉签名身份 → bootstrap 创建全新身份
    //   4) 仍不完整且远端已有身份 → 明确报错（不破坏远端信任链）
    // 注意：远端身份判断必须服务器权威——get_user_identity 只读本地 store，在全新登录/
    // store 丢失后会假阴性，导致误 bootstrap 覆盖账号真实身份；request_user_identity
    // 发起 /keys/query 并保证最新。
    let matrix_uid = client
        .user_id()
        .context("未登录，无法获取会话用户")?
        .to_owned();

    let remote_has_identity = match client.encryption().get_user_identity(&matrix_uid).await {
        Ok(Some(_)) => true,
        _ => client
            .encryption()
            .request_user_identity(&matrix_uid)
            .await
            .ok()
            .flatten()
            .is_some(),
    };

    let local_complete_now = || async {
        client
            .encryption()
            .cross_signing_status()
            .await
            .as_ref()
            .is_some_and(|s| s.is_complete())
    };

    match next_identity_action(
        local_complete_now().await,
        encrypted_config.matrix_recovery_key.is_some(),
        remote_has_identity,
    ) {
        IdentityAction::None => {
            println!("✅ 交叉签名状态完整");
        }
        IdentityAction::TryRecovery => {
            println!("⚠ 本地交叉签名私钥不完整，尝试恢复密钥导入…");
            // I1: recovered 必须以 store 真值为准——recover() 返回 Ok 只代表打开了
            // secret storage 并导入了“能导入的”，未必包含完整交叉签名私钥。
            let _ = try_recover_with_key(&client, security, encrypted_config).await;
            let recovered = local_complete_now().await;
            match after_recovery_action(recovered, remote_has_identity) {
                IdentityAction::None => {
                    println!("✅ 恢复密钥导入成功，设备已加入信任链");
                    // 用完即焚 — atomic clear（失败时保留密钥，避免误烧）
                    if let Err(e) = crate::bootstrap::clear_matrix_recovery_key(config_dir) {
                        eprintln!("⚠ 清除已用恢复密钥失败(将保留在配置中): {e}");
                    }
                }
                IdentityAction::BootstrapNew => {
                    bootstrap_new_identity(&client, &matrix_username, &matrix_pwd).await?;
                }
                IdentityAction::ErrorRequiresReset => {
                    anyhow::bail!(
                        "远端已有交叉签名身份，但本地缺少私钥且恢复密钥无法恢复。\
                         请在 Element 用正确的恢复密钥恢复，或重置加密后重新配置 matrix_recovery_key"
                    );
                }
                IdentityAction::TryRecovery => unreachable!(),
            }
        }
        IdentityAction::BootstrapNew => {
            println!("⚠ 远端无交叉签名身份，创建全新身份…");
            bootstrap_new_identity(&client, &matrix_username, &matrix_pwd).await?;
        }
        IdentityAction::ErrorRequiresReset => {
            anyhow::bail!(
                "远端已有交叉签名身份，但本地缺少私钥且未配置恢复密钥。\
                 请提供正确的 matrix_recovery_key，或在客户端重置加密身份后重新配置"
            );
        }
    }

    client.sync_once(SyncSettings::default()).await?;

    let room_id: matrix_sdk::ruma::OwnedRoomId = matrix_room_id_str.parse()?;

    let client_inv = client.clone();
    client.add_event_handler(
        move |_: matrix_sdk::ruma::events::room::member::OriginalSyncRoomMemberEvent,
              room: Room| {
            let c = client_inv.clone();
            async move {
                if room.state() == RoomState::Invited {
                    let _ = c.join_room_by_id(room.room_id()).await;
                }
            }
        },
    );

    let room = client
        .get_room(&room_id)
        .context("未找到 Matrix 房间，请先邀请机器人到房间")?;

    let matrix_adapter: Arc<dyn BotAdapter> = Arc::new(MatrixAdapter::new(room.clone()));
    Ok((client, room, matrix_adapter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::EncryptedConfig;

    fn empty_config() -> EncryptedConfig {
        EncryptedConfig {
            token: Some(vec![]),
            admin_id: Some(vec![]),
            totp_secret: Some(vec![]),
            self_destruct_key_hash: None,
            matrix_homeserver: None,
            matrix_username: None,
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            discord_token: None,
            discord_admin_id: None,
            lang: None,
            matrix_recovery_key: None,
        }
    }

    #[test]
    fn returns_true_when_all_matrix_fields_present() {
        let config = EncryptedConfig {
            token: Some(vec![]),
            admin_id: Some(vec![]),
            totp_secret: Some(vec![]),
            self_destruct_key_hash: None,
            matrix_homeserver: Some(vec![1]),
            matrix_username: Some(vec![1]),
            matrix_password: Some(vec![1]),
            matrix_room_id: Some(vec![1]),
            matrix_store_passphrase: None,
            discord_token: None,
            discord_admin_id: None,
            lang: None,
            matrix_recovery_key: None,
        };
        assert!(has_matrix_config(&config, &[]));
    }

    #[test]
    fn returns_false_when_matrix_fields_missing() {
        let config = empty_config();
        assert!(!has_matrix_config(&config, &[]));
    }

    #[test]
    fn returns_true_when_flag_overrides_empty_fields() {
        let config = empty_config();
        assert!(has_matrix_config(&config, &["--matrix".to_string()]));
    }

    #[test]
    fn returns_true_when_all_flag_overrides_empty_fields() {
        let config = empty_config();
        assert!(has_matrix_config(&config, &["--all".to_string()]));
    }

    #[test]
    fn returns_false_when_some_fields_missing() {
        let config = EncryptedConfig {
            token: Some(vec![]),
            admin_id: Some(vec![]),
            totp_secret: Some(vec![]),
            self_destruct_key_hash: None,
            matrix_homeserver: Some(vec![1]),
            matrix_username: Some(vec![1]),
            matrix_password: None,
            matrix_room_id: None,
            matrix_store_passphrase: None,
            discord_token: None,
            discord_admin_id: None,
            lang: None,
            matrix_recovery_key: None,
        };
        assert!(!has_matrix_config(&config, &[]));
    }

    #[test]
    fn ignores_non_matrix_flags() {
        let config = empty_config();
        assert!(!has_matrix_config(&config, &["--tg-only".to_string()]));
    }

    // ── 交叉签名身份决策（connect_matrix P2 分层逻辑）──

    #[test]
    fn identity_action_none_when_local_complete() {
        assert_eq!(next_identity_action(true, true, true), IdentityAction::None);
        assert_eq!(
            next_identity_action(true, false, false),
            IdentityAction::None
        );
    }

    #[test]
    fn identity_action_tries_recovery_when_key_present() {
        assert_eq!(
            next_identity_action(false, true, true),
            IdentityAction::TryRecovery
        );
        assert_eq!(
            next_identity_action(false, true, false),
            IdentityAction::TryRecovery
        );
    }

    #[test]
    fn identity_action_requires_reset_when_remote_identity_and_no_key() {
        assert_eq!(
            next_identity_action(false, false, true),
            IdentityAction::ErrorRequiresReset
        );
    }

    #[test]
    fn identity_action_bootstraps_when_no_remote_identity_and_no_key() {
        assert_eq!(
            next_identity_action(false, false, false),
            IdentityAction::BootstrapNew
        );
    }

    #[test]
    fn after_recovery_none_when_recovered() {
        assert_eq!(after_recovery_action(true, true), IdentityAction::None);
        assert_eq!(after_recovery_action(true, false), IdentityAction::None);
    }

    #[test]
    fn after_recovery_requires_reset_when_remote_identity_present() {
        assert_eq!(
            after_recovery_action(false, true),
            IdentityAction::ErrorRequiresReset
        );
    }

    #[test]
    fn after_recovery_bootstraps_when_remote_empty() {
        assert_eq!(
            after_recovery_action(false, false),
            IdentityAction::BootstrapNew
        );
    }

    // ── Matrix 设备显示名（带机房归属城市前缀）──

    #[test]
    fn device_name_prepends_city() {
        assert_eq!(
            matrix_device_display_name(Some("San Jose")),
            "San Jose Aegis Matrix Bot"
        );
    }

    #[test]
    fn device_name_falls_back_without_city() {
        assert_eq!(matrix_device_display_name(None), "Aegis Matrix Bot");
    }

    #[test]
    fn device_name_ignores_blank_city() {
        assert_eq!(matrix_device_display_name(Some("   ")), "Aegis Matrix Bot");
    }

    #[test]
    fn device_name_trims_city() {
        assert_eq!(
            matrix_device_display_name(Some("  San Jose  ")),
            "San Jose Aegis Matrix Bot"
        );
    }
}
