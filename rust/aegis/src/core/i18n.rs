use std::str::FromStr;

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

pub fn set_lang(lang: Lang) {
    rust_i18n::set_locale(lang.as_str());
}

pub fn current_lang() -> Lang {
    rust_i18n::locale().parse().unwrap_or(Lang::Zh)
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
}
