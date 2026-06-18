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
        assert_eq!(crate::core::i18n::lang_to_timezone(Lang::Zh), "Asia/Shanghai");
    }

    #[test]
    fn lang_to_timezone_en() {
        assert_eq!(crate::core::i18n::lang_to_timezone(Lang::En), "America/New_York");
    }

    #[test]
    fn lang_to_timezone_ja() {
        assert_eq!(crate::core::i18n::lang_to_timezone(Lang::Ja), "Asia/Tokyo");
    }
}
