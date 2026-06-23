use std::fs;
use std::path::Path;
use std::sync::Arc;

use aegis::adapters::common::BotAdapter;
use aegis::adapters::matrix::MatrixAdapter;
use aegis::core::security::SecurityManager;
use anyhow::{Context, Result};
use matrix_sdk::{
    Client as MatrixClient, Room, RoomState,
    config::SyncSettings,
    ruma::api::client::uiaa::{AuthData, MatrixUserIdentifier, Password, UserIdentifier},
};
use secrecy::ExposeSecret;

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
            fs::write(&session_path, session_json)?;
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
