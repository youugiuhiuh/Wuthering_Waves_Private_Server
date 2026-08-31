use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    Zh,
    En,
    Ja,
}

impl Lang {
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::En => "en",
            Lang::Ja => "ja",
        }
    }
}

impl FromStr for Lang {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "en" => Ok(Lang::En),
            "ja" => Ok(Lang::Ja),
            "zh" => Ok(Lang::Zh),
            _ => Err(()),
        }
    }
}

static LANG_CONFIGURED: AtomicBool = AtomicBool::new(false);

pub fn is_lang_configured() -> bool {
    LANG_CONFIGURED.load(Ordering::Relaxed)
}

pub fn mark_lang_configured() {
    LANG_CONFIGURED.store(true, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn reset_lang_configured() {
    LANG_CONFIGURED.store(false, Ordering::Relaxed);
}

pub fn set_lang(lang: Lang) {
    rust_i18n::set_locale(lang.as_str());
}

pub fn current_lang() -> Lang {
    rust_i18n::locale().parse().unwrap_or(Lang::Zh)
}

pub fn lang_to_timezone(lang: Lang) -> &'static str {
    match lang {
        Lang::Zh => "Asia/Shanghai",
        Lang::En => "America/New_York",
        Lang::Ja => "Asia/Tokyo",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn default_lang_is_zh() {
        set_lang(Lang::Zh);
        assert_eq!(current_lang(), Lang::Zh);
    }

    #[test]
    fn set_and_get_lang() {
        set_lang(Lang::En);
        assert_eq!(current_lang(), Lang::En);
        set_lang(Lang::Ja);
        assert_eq!(current_lang(), Lang::Ja);
        set_lang(Lang::Zh);
    }

    #[test]
    fn lang_from_str() {
        assert_eq!("zh".parse::<Lang>().unwrap(), Lang::Zh);
        assert_eq!("en".parse::<Lang>().unwrap(), Lang::En);
        assert_eq!("ja".parse::<Lang>().unwrap(), Lang::Ja);
        assert!("unknown".parse::<Lang>().is_err());
    }

    #[test]
    fn lang_as_str() {
        assert_eq!(Lang::Zh.as_str(), "zh");
        assert_eq!(Lang::En.as_str(), "en");
        assert_eq!(Lang::Ja.as_str(), "ja");
    }

    #[serial]
    #[test]
    fn is_lang_configured_returns_false_initially() {
        crate::core::i18n::reset_lang_configured();
        assert!(!crate::core::i18n::is_lang_configured());
    }

    #[serial]
    #[test]
    fn is_lang_configured_returns_true_after_mark() {
        crate::core::i18n::reset_lang_configured();
        crate::core::i18n::mark_lang_configured();
        assert!(crate::core::i18n::is_lang_configured());
    }

    #[test]
    fn lang_to_timezone_zh() {
        assert_eq!(
            crate::core::i18n::lang_to_timezone(Lang::Zh),
            "Asia/Shanghai"
        );
    }

    #[test]
    fn lang_to_timezone_en() {
        assert_eq!(
            crate::core::i18n::lang_to_timezone(Lang::En),
            "America/New_York"
        );
    }

    #[test]
    fn lang_to_timezone_ja() {
        assert_eq!(crate::core::i18n::lang_to_timezone(Lang::Ja), "Asia/Tokyo");
    }

    #[serial]
    #[test]
    fn domain_translation_keys_exist() {
        let keys = [
            "domain.prompt",
            "domain.yes",
            "domain.no",
            "domain.input_prompt",
            "domain.input_timeout",
            "domain.input_empty",
            "domain.prov_title",
            "domain.prov_cf",
            "domain.prov_aws",
            "domain.cred_prompt",
            "domain.cred_invalid",
            "domain.installing_acme",
            "domain.acme_install_fail",
            "domain.issuing_cert",
            "domain.cert_success",
            "domain.cert_fail",
            "domain.cert_timeout",
            "domain.cert_renew",
            "domain.cert_skip",
            "domain.cred_fail",
            "domain.gen_progress",
            "domain.processing",
            "domain.flow_expired",
            "ops.deploy_step_xhttp_tls",
            "ops.deploy_created_xhttp_tls",
            "ops.deploy_created_xhttp_bonus",
            "ops.deploy_fail_xhttp_tls",
            "xray.tls_batch_title",
            "xray.tls_batch_done",
        ];

        for lang in [Lang::Zh, Lang::En, Lang::Ja] {
            set_lang(lang);
            for key in &keys {
                let value = rust_i18n::t!(*key);
                let value_str = value.to_string();
                assert!(
                    !value_str.is_empty() && value_str != *key,
                    "key '{}' missing or resolves to key itself for {:?}",
                    key,
                    lang
                );
            }
        }

        set_lang(Lang::Zh);
    }

    /// 防混淆回归测试：
    /// 机器端部署名为 `wwps-core` / `wwps-box`，但用户端显示必须用上游产品名
    /// **Xray-core** / **Sing-box**（见 `core/paths.rs` 模块注释的命名映射）。
    ///
    /// 这些键名虽然带 `wwps_core_*` 前缀（历史标识符，禁止重命名），
    /// 但它们的值不得再出现裸的部署名 `wwps-core`。
    #[serial]
    #[test]
    fn user_facing_core_display_name_is_xray_core_not_wwps_core() {
        let keys = [
            "menu.wwps_core_mgmt",
            "menu.wwps_core_restart",
            "menu.wwps_core_status",
            "menu.wwps_core_restart_success",
            "menu.wwps_core_restart_fail",
            "menu.wwps_core_status_text",
            "menu.wwps_core_status_fail",
            "menu.wwps_core_btn",
            "upgrade.core_checking",
            "upgrade.core_fetching",
            "upgrade.core_restarting",
            "upgrade.core_updated",
            "upgrade.core_download_info",
        ];

        for lang in [Lang::Zh, Lang::En, Lang::Ja] {
            set_lang(lang);
            for key in &keys {
                let value_str = rust_i18n::t!(*key).to_string();
                assert!(
                    value_str.contains("Xray-core"),
                    "[{:?}] {} 应显示上游产品名 Xray-core，实际: {}",
                    lang,
                    key,
                    value_str
                );
                assert!(
                    !value_str.contains("wwps-core"),
                    "[{:?}] {} 泄漏了部署名 wwps-core，实际: {}",
                    lang,
                    key,
                    value_str
                );
            }
        }

        set_lang(Lang::Zh);
    }

    /// 防混淆回归测试：sing-box 的用户端显示必须用上游产品名 **Sing-box**，
    /// 不得出现部署名 `wwps-box`。
    #[serial]
    #[test]
    fn user_facing_singbox_display_name_is_sing_box_not_wwps_box() {
        let keys = [
            "menu.singbox_mgmt_title",
            "menu.singbox_mgmt_btn",
            "menu.singbox_status",
            "menu.singbox_install",
            "menu.singbox_installing",
            "menu.singbox_install_success",
            "ops.singbox_restart",
            "ops.singbox_restart_success",
        ];

        for lang in [Lang::Zh, Lang::En, Lang::Ja] {
            set_lang(lang);
            for key in &keys {
                let value_str = rust_i18n::t!(*key).to_string();
                assert!(
                    value_str.contains("Sing-box"),
                    "[{:?}] {} 应显示上游产品名 Sing-box，实际: {}",
                    lang,
                    key,
                    value_str
                );
                assert!(
                    !value_str.contains("wwps-box"),
                    "[{:?}] {} 泄漏了部署名 wwps-box，实际: {}",
                    lang,
                    key,
                    value_str
                );
            }
        }

        set_lang(Lang::Zh);
    }

    /// 防混淆回归测试：ML-DSA-65 命令引用键**故意**包含真实命令 `wwps-core mldsa65`
    /// （机器端实际执行的命令），但必须同时标注其身份为 Xray-core，防止用户误解。
    #[serial]
    #[test]
    fn pq_command_reference_keeps_real_command_and_annotates_xray_core() {
        let keys = [
            "xray.pq_mgmt_title",
            "xray.pq_title",
            "xray.pq_init_success",
        ];

        for lang in [Lang::Zh, Lang::En, Lang::Ja] {
            set_lang(lang);
            for key in &keys {
                let value_str = rust_i18n::t!(*key).to_string();
                assert!(
                    value_str.contains("wwps-core mldsa65"),
                    "[{:?}] {} 应保留真实命令 wwps-core mldsa65，实际: {}",
                    lang,
                    key,
                    value_str
                );
                assert!(
                    value_str.contains("Xray-core"),
                    "[{:?}] {} 应标注命令身份为 Xray-core，实际: {}",
                    lang,
                    key,
                    value_str
                );
            }
        }

        set_lang(Lang::Zh);
    }
}
