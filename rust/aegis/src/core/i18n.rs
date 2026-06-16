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

    pub fn from_str(s: &str) -> Self {
        match s {
            "en" => Lang::En,
            "ja" => Lang::Ja,
            _ => Lang::Zh,
        }
    }
}

pub fn set_lang(lang: Lang) {
    rust_i18n::set_locale(lang.as_str());
}

pub fn current_lang() -> Lang {
    Lang::from_str(&rust_i18n::locale())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(Lang::from_str("zh"), Lang::Zh);
        assert_eq!(Lang::from_str("en"), Lang::En);
        assert_eq!(Lang::from_str("ja"), Lang::Ja);
        assert_eq!(Lang::from_str("unknown"), Lang::Zh);
    }

    #[test]
    fn lang_as_str() {
        assert_eq!(Lang::Zh.as_str(), "zh");
        assert_eq!(Lang::En.as_str(), "en");
        assert_eq!(Lang::Ja.as_str(), "ja");
    }
}
