use std::fs;
use std::path::Path;
use std::sync::Arc;

use aegis::adapters::common::BotAdapter;
use aegis::adapters::matrix::MatrixAdapter;
use aegis::core::security::SecurityManager;
use aegis::core::security::secure_fs::atomic_write_sensitive;
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
        client
            .matrix_auth()
            .login_username(&matrix_username, &matrix_pwd)
            .initial_device_display_name("Aegis Matrix Bot")
            .send()
            .await?;
        println!("✅ Matrix 登录成功: {matrix_username}");

        // Save session for future restarts
        if let Some(session) = client.matrix_auth().session() {
            let session_json = serde_json::to_string(&session)?;
            atomic_write_sensitive(&session_path, session_json.as_bytes())?;
            println!("✅ Matrix 会话已保存");
        }
    }

    // P2: Bootstrap cross-signing (best-effort)
    if client
        .encryption()
        .bootstrap_cross_signing_if_needed(None)
        .await
        .is_err()
    {
        let _ = client
            .encryption()
            .bootstrap_cross_signing_if_needed(Some(AuthData::Password(Password::new(
                UserIdentifier::Matrix(MatrixUserIdentifier::new(matrix_username.clone())),
                matrix_pwd.clone(),
            ))))
            .await;
    }

    // P1: Wait for E2EE initialization tasks
    client
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;

    // P1.5: Apply recovery key if cross-signing is incomplete
    {
        let status = client.encryption().cross_signing_status().await;
        if status.is_some_and(|s| s.is_complete()) {
            println!("✅ 交叉签名状态完整");
        } else {
            println!("⚠ 交叉签名状态不完整，尝试恢复密钥导入");
            let rk_encrypted = encrypted_config.matrix_recovery_key.as_ref().context(
                "远端已有交叉签名身份，本设备缺少私钥。请在配置中提供 matrix_recovery_key",
            )?;
            let rk_decrypted = security
                .decrypt(rk_encrypted)
                .context("解密 matrix_recovery_key 失败")?;
            let rk_str = String::from_utf8(rk_decrypted.expose_secret().to_vec())
                .map_err(|e| anyhow::anyhow!("matrix_recovery_key 包含无效的 UTF-8: {}", e))?
                .trim()
                .to_string();
            let rk = SecretString::from(rk_str);

            let recovery = client.encryption().recovery();
            match recovery.recover(rk.expose_secret()).await {
                Ok(_) => {}
                Err(matrix_sdk::encryption::recovery::RecoveryError::BackupExistsOnServer) => {
                    recovery.recover_and_fix_backup(rk.expose_secret()).await?;
                    println!("✅ 恢复密钥 + 修复 backup 成功");
                }
                Err(e) => anyhow::bail!("恢复密钥导入失败: {e}"),
            }

            let status = client
                .encryption()
                .cross_signing_status()
                .await
                .context("recover 后 cross_signing_status 返回 None")?;
            anyhow::ensure!(
                status.is_complete(),
                "恢复密钥导入后交叉签名状态仍不完整: master={}, self={}, user={}",
                status.has_master,
                status.has_self_signing,
                status.has_user_signing,
            );
            println!("✅ 恢复密钥导入成功，设备已加入信任链");

            // 用完即焚 — atomic clear
            crate::bootstrap::clear_matrix_recovery_key(config_dir)?;
            // rk (SecretString) zeroize happens on drop
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
            token: vec![],
            admin_id: vec![],
            totp_secret: vec![],
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
            token: vec![],
            admin_id: vec![],
            totp_secret: vec![],
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
            token: vec![],
            admin_id: vec![],
            totp_secret: vec![],
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
}
