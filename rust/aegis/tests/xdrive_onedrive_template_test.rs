//! 端到端集成测试：XDRIVE Microsoft OneDrive Template 模式
//!
//! 验证点：
//! 1. `XdriveConfig::new_onedrive` 预设构造器：
//!    - 自动装配公开 Client ID 与 Client Secret；
//!    - 设定经过验证的 OneDrive 平衡流控参数（64KB 分片、80ms 刷出、200ms 轮询、1200ms 退避、4 并发）；
//!    - Service 标记为 `XdriveService::OneDrive`。
//! 2. `generate_onedrive_template_value()` 输出的 Microsoft Graph API Template：
//!    - `flatten == true`（Xray template 模式协议扁平化传输）；
//!    - `auth.tokenUrl` 为 Microsoft OAuth2 v2.0 端点；
//!    - `list.namesRegex` 包含且仅包含 1 个捕获组，且能准确匹配 Graph children 目录项；
//!    - `retry.status` 包含 429 限流状态码。
//! 3. Xray Inbound JSON 序列化：
//!    - `XrayInbound::build_xdrive_inbound` 自动将 `service` 转译为 `"template"` 并注入完整的 `template` 对象；
//!    - Xray JSON 结构合法，可被完整解析反序列化。
//! 4. `extract_onedrive_tokens` 智能向导提取：
//!    - 对标准的 `rclone authorize "onedrive"` JSON 自动补齐内置公开凭证；
//!    - 保留自定义 Client ID / Client Secret。

use aegis::core::xray::config::XrayInbound;
use aegis::core::xray::xdrive::{
    ONEDRIVE_DEFAULT_CLIENT_ID, ONEDRIVE_DEFAULT_CLIENT_SECRET, XdriveConfig, XdriveService,
    extract_onedrive_tokens,
};
use regex::Regex;
use serde_json::Value;

#[test]
fn test_onedrive_config_default_initialization() {
    let refresh_token = "mock_refresh_token";
    let remote_folder = "xray_sync";

    let config = XdriveConfig::new_onedrive(refresh_token, remote_folder);

    // 1. service 为 XdriveService::OneDrive
    assert_eq!(
        config.service,
        XdriveService::OneDrive,
        "service 必须为 XdriveService::OneDrive"
    );

    // 2. remote_folder 规范化去除前后斜杠
    assert_eq!(config.remote_folder, "xray_sync");

    // 3. secrets 自动包含内置的 ONEDRIVE_DEFAULT_CLIENT_ID 与 ONEDRIVE_DEFAULT_CLIENT_SECRET
    assert_eq!(
        config.secrets,
        vec![
            ONEDRIVE_DEFAULT_CLIENT_ID.to_string(),
            ONEDRIVE_DEFAULT_CLIENT_SECRET.to_string(),
            refresh_token.to_string(),
        ],
        "secrets 数组必须依次包含 [default_client_id, default_client_secret, refresh_token]"
    );

    // 4. 流控参数校验
    assert_eq!(
        config.segment_bytes,
        Some(65536),
        "segment_bytes 必须为 65536 (64KB)"
    );
    assert_eq!(
        config.flush_interval_ms,
        Some(30),
        "flush_interval_ms 必须为 30ms"
    );
    assert_eq!(
        config.poll_interval_ms,
        Some(100),
        "poll_interval_ms 必须为 100ms"
    );
    assert_eq!(
        config.max_poll_interval_ms,
        Some(1000),
        "max_poll_interval_ms 必须为 1000ms"
    );
    assert_eq!(config.concurrency, Some(4), "concurrency 并发槽数必须为 4");
    assert_eq!(
        config.eager_window_ms,
        Some(2000),
        "eager_window_ms 积极轮询窗口必须为 2000ms"
    );

    // 5. 校验配置合法性
    assert!(
        config.validate().is_ok(),
        "XdriveConfig::new_onedrive 构造的配置必须通过合法性校验"
    );
}

#[test]
fn test_generate_onedrive_template_value_structure() {
    let config = XdriveConfig::new_onedrive("mock_refresh_token", "xray_sync");
    let template = config.generate_onedrive_template_value();

    // 1. "flatten" 为 true
    assert_eq!(
        template["flatten"].as_bool(),
        Some(true),
        "flatten 必须显式声明为 true"
    );

    // 2. "auth"."tokenUrl" 为 Microsoft OAuth2 v2.0 Token 端点
    assert_eq!(
        template["auth"]["tokenUrl"].as_str(),
        Some("https://login.microsoftonline.com/common/oauth2/v2.0/token"),
        "auth.tokenUrl 必须指向 Microsoft common oauth2 v2.0 token endpoint"
    );
    assert_eq!(template["auth"]["type"].as_str(), Some("oauth2"));
    assert_eq!(
        template["auth"]["form"]["client_id"].as_str(),
        Some("{secret0}")
    );
    assert_eq!(
        template["auth"]["form"]["client_secret"].as_str(),
        Some("{secret1}")
    );
    assert_eq!(
        template["auth"]["form"]["refresh_token"].as_str(),
        Some("{secret2}")
    );
    assert_eq!(
        template["auth"]["form"]["grant_type"].as_str(),
        Some("refresh_token")
    );

    // 3. "list"."namesRegex" 匹配包含单个捕获组
    let names_regex_str = template["list"]["namesRegex"]
        .as_str()
        .expect("list.namesRegex 必须为字符串");
    let regex = Regex::new(names_regex_str).expect("list.namesRegex 必须是有效的正则表达式");
    // captures_len 包含全量匹配(第0组)与捕获组，因此 1 个捕获组时 captures_len 为 2
    assert_eq!(
        regex.captures_len(),
        2,
        "list.namesRegex 必须恰好包含单个捕获组以提取文件名"
    );

    // 针对典型 Microsoft Graph children 响应进行正则匹配提取断言
    let mock_graph_response = r#"{
        "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#users('me')/drive/root/children",
        "value": [
            {
                "createdDateTime": "2026-03-30T10:00:00Z",
                "name": "xray_seg_0001.bin",
                "size": 65536
            },
            {
                "createdDateTime": "2026-03-30T10:00:01Z",
                "name": "xray_seg_0002.bin",
                "size": 65536
            }
        ]
    }"#;
    let extracted_names: Vec<String> = regex
        .captures_iter(mock_graph_response)
        .map(|cap| cap[1].to_string())
        .collect();
    assert_eq!(
        extracted_names,
        vec![
            "xray_seg_0001.bin".to_string(),
            "xray_seg_0002.bin".to_string()
        ],
        "namesRegex 必须能够正确捕获 Graph API JSON 中的文件名"
    );

    // 4. "retry"."status" 包含 429
    let retry_statuses = template["retry"]["status"]
        .as_array()
        .expect("retry.status 必须为数组");
    let has_429 = retry_statuses.iter().any(|v| v.as_i64() == Some(429));
    assert!(
        has_429,
        "retry.status 必须包含 429 (Too Many Requests) 限流重试状态码"
    );
}

#[test]
fn test_xray_inbound_xdrive_onedrive_serialization() {
    let config = XdriveConfig::new_onedrive("mock_refresh_token", "xray_sync");
    let tag = "inbound-vless-onedrive";
    let uuid = "01234567-89ab-cdef-0123-456789abcdef";
    let email = "onedrive-test@wwps.internal";
    let port = 10086;
    let decryption = "none";

    // 1. 调用 XrayInbound::build_xdrive_inbound 构建入站配置
    let inbound = XrayInbound::build_xdrive_inbound(tag, uuid, email, port, &config, decryption);

    // 2. 断言基础入站属性
    assert_eq!(inbound["tag"], tag);
    assert_eq!(inbound["listen"], "127.0.0.1");
    assert_eq!(inbound["port"], port);
    assert_eq!(inbound["protocol"], "vless");
    assert_eq!(inbound["settings"]["clients"][0]["id"], uuid);
    assert_eq!(inbound["settings"]["clients"][0]["email"], email);
    assert_eq!(inbound["settings"]["decryption"], decryption);
    assert_eq!(inbound["streamSettings"]["network"], "xdrive");

    // 3. 断言生成的 JSON 中 streamSettings.xdriveSettings.service 为 "template"
    let xdrive_settings = &inbound["streamSettings"]["xdriveSettings"];
    assert_eq!(
        xdrive_settings["service"], "template",
        "针对 OneDrive 后端，Xray streamSettings.xdriveSettings.service 必须自动转译为 'template'"
    );

    // 4. 断言生成的 streamSettings.xdriveSettings.template 为完整的 Object，且包含所需属性
    let template = &xdrive_settings["template"];
    assert!(
        template.is_object(),
        "streamSettings.xdriveSettings.template 必须是完整的 JSON Object"
    );
    assert_eq!(template["flatten"], true);
    assert_eq!(
        template["auth"]["tokenUrl"],
        "https://login.microsoftonline.com/common/oauth2/v2.0/token"
    );
    assert_eq!(
        template["put"]["url"],
        "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}:/content"
    );
    assert_eq!(
        template["get"]["url"],
        "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}:/content"
    );
    assert_eq!(
        template["delete"]["url"],
        "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}/{name}"
    );
    assert_eq!(
        template["list"]["url"],
        "https://graph.microsoft.com/v1.0/me/drive/root:/{folder}:/children?select=name"
    );
    assert!(
        template["retry"]["status"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(429))
    );

    // 5. 确保 Xray 配置结构能够正常反序列化/解析（JSON Roundtrip 解析无损）
    let serialized_json = serde_json::to_string_pretty(&inbound)
        .expect("Xray inbound 配置必须能够成功序列化为字符串");
    let deserialized: Value = serde_json::from_str(&serialized_json)
        .expect("Xray inbound 序列化字符串必须能够被标准 JSON 解析器正确解析");
    assert_eq!(
        deserialized, inbound,
        "反序列化后的 JSON 结构必须与原始入站配置完全一致"
    );
}

#[test]
fn test_extract_onedrive_tokens_smart_wizard() {
    // 1. 模拟 rclone authorize "onedrive" 输出的标准 JSON 字符串
    let rclone_json = r#"{
        "access_token": "mock_access_token_xyz",
        "token_type": "Bearer",
        "refresh_token": "mock_refresh_token_abc",
        "expiry": "2026-11-10T12:00:00Z"
    }"#;

    let (client_id, client_secret, refresh_token, folder) =
        extract_onedrive_tokens(rclone_json).expect("必须成功解析并提取标准 rclone authorize JSON");

    // 断言自动绑定内置 Client ID 与 Client Secret
    assert_eq!(
        client_id, ONEDRIVE_DEFAULT_CLIENT_ID,
        "未显式指定 client_id 时必须自动绑定内置 ONEDRIVE_DEFAULT_CLIENT_ID"
    );
    assert_eq!(
        client_secret, ONEDRIVE_DEFAULT_CLIENT_SECRET,
        "未显式指定 client_secret 时必须自动绑定内置 ONEDRIVE_DEFAULT_CLIENT_SECRET"
    );
    assert_eq!(refresh_token, "mock_refresh_token_abc");
    assert_eq!(folder, None);

    // 2. 模拟带 rclone 终端提示横幅的输出块
    let rclone_cli_banner = r#"
2026/03/30 12:00:00 NOTICE: If your browser doesn't open automatically go to the following link: http://127.0.0.1:53682/auth
2026/03/30 12:00:00 NOTICE: Log in and authorize rclone for access
2026/03/30 12:00:00 NOTICE: Waiting for code...
2026/03/30 12:00:00 NOTICE: Got code
Paste the following into your remote machine --->
{"access_token":"mock_access_token_cli","token_type":"Bearer","refresh_token":"mock_refresh_token_banner","expiry":"2026-11-10T12:00:00Z"}
<---End paste
"#;
    let (cid_b, sec_b, ref_tok_b, _) = extract_onedrive_tokens(rclone_cli_banner)
        .expect("必须能从包含前导与后置横幅的 rclone CLI 输出中提取凭证");
    assert_eq!(cid_b, ONEDRIVE_DEFAULT_CLIENT_ID);
    assert_eq!(sec_b, ONEDRIVE_DEFAULT_CLIENT_SECRET);
    assert_eq!(ref_tok_b, "mock_refresh_token_banner");

    // 3. 用户提供自定义 Client ID 与 Client Secret 的情况
    let custom_json = r#"{
        "client_id": "custom-onedrive-app-id",
        "client_secret": "custom-onedrive-app-secret",
        "refresh_token": "custom-refresh-token-val",
        "folder": "/custom_tunnel_folder"
    }"#;
    let (cid_c, sec_c, ref_tok_c, folder_c) =
        extract_onedrive_tokens(custom_json).expect("必须能正确提取自定义的 OneDrive 凭证");
    assert_eq!(cid_c, "custom-onedrive-app-id");
    assert_eq!(sec_c, "custom-onedrive-app-secret");
    assert_eq!(ref_tok_c, "custom-refresh-token-val");
    assert_eq!(folder_c.as_deref(), Some("/custom_tunnel_folder"));
}
