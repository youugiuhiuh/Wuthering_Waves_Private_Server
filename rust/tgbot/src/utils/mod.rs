use rust_i18n::t;

pub fn format_duration_human(secs: u64, lang: &str) -> String {
    if secs < 60 {
        t!("duration.seconds", locale = lang, count = secs).to_string()
    } else if secs < 3600 {
        t!("duration.minutes", locale = lang, count = secs / 60).to_string()
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            t!("duration.hours", locale = lang, count = h).to_string()
        } else {
            t!("duration.hours_minutes", locale = lang, h = h, m = m).to_string()
        }
    } else {
        let d = secs / 86400;
        let remaining = secs % 86400;
        if remaining >= 3600 {
            let h = remaining / 3600;
            t!("duration.days_hours", locale = lang, count = d, h = h).to_string()
        } else {
            t!("duration.days", locale = lang, count = d).to_string()
        }
    }
}

pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn validate_hash_prefix(prefix: &str) -> anyhow::Result<&str> {
    if prefix.is_empty() {
        anyhow::bail!("hash 前缀不能为空");
    }
    if prefix.len() > 8 {
        anyhow::bail!("hash 前缀过长: {} (最大 8)", prefix.len());
    }
    if !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("hash 前缀包含无效字符");
    }
    Ok(prefix)
}

pub fn validate_idx(idx: usize, max: usize, field_name: &str) -> anyhow::Result<()> {
    if idx >= max {
        anyhow::bail!(
            "{} 索引 {} 超出范围 (最大 {})",
            field_name,
            idx,
            max.saturating_sub(1)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(key: &str) -> String {
        rust_i18n::t!(key, locale = "zh-CN").to_string()
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration_human(0, "zh-CN"), "0秒");
        assert_eq!(format_duration_human(30, "zh-CN"), "30秒");
        assert_eq!(format_duration_human(59, "zh-CN"), "59秒");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration_human(60, "zh-CN"), "1分钟");
        assert_eq!(format_duration_human(90, "zh-CN"), "1分钟");
        assert_eq!(format_duration_human(120, "zh-CN"), "2分钟");
        assert_eq!(format_duration_human(3599, "zh-CN"), "59分钟");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration_human(3600, "zh-CN"), "1小时");
        assert_eq!(format_duration_human(3660, "zh-CN"), "1小时1分");
        assert_eq!(format_duration_human(7200, "zh-CN"), "2小时");
        assert_eq!(format_duration_human(7320, "zh-CN"), "2小时2分");
    }

    #[test]
    fn format_duration_days() {
        assert_eq!(format_duration_human(86400, "zh-CN"), "1天");
        assert_eq!(format_duration_human(172800, "zh-CN"), "2天");
        assert_eq!(format_duration_human(90000, "zh-CN"), "1天1小时");
    }

    #[test]
    fn escape_html_ampersand() {
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("&amp;"), "&amp;amp;");
    }

    #[test]
    fn escape_html_angle_brackets() {
        assert_eq!(escape_html("<div>"), "&lt;div&gt;");
        assert_eq!(escape_html("<"), "&lt;");
        assert_eq!(escape_html(">"), "&gt;");
    }

    #[test]
    fn escape_html_quotes() {
        assert_eq!(escape_html("say \"hello\""), "say &quot;hello&quot;");
    }

    #[test]
    fn escape_html_plain_text() {
        assert_eq!(escape_html("hello world"), "hello world");
        assert_eq!(escape_html(""), "");
    }

    #[test]
    fn validate_hash_prefix_valid() {
        assert_eq!(validate_hash_prefix("abc").unwrap(), "abc");
        assert_eq!(validate_hash_prefix("ABCDEF12").unwrap(), "ABCDEF12");
        assert_eq!(validate_hash_prefix("1").unwrap(), "1");
    }

    #[test]
    fn validate_hash_prefix_empty() {
        let result = validate_hash_prefix("");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "hash 前缀不能为空");
    }

    #[test]
    fn validate_hash_prefix_too_long() {
        let result = validate_hash_prefix("ABCDEF123");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("hash 前缀过长"));
    }

    #[test]
    fn validate_hash_prefix_invalid_chars() {
        let result = validate_hash_prefix("GG");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("无效字符"));
    }

    #[test]
    fn validate_idx_valid() {
        assert!(validate_idx(0, 5, "items").is_ok());
        assert!(validate_idx(4, 5, "items").is_ok());
    }

    #[test]
    fn validate_idx_out_of_bounds() {
        let result = validate_idx(5, 5, "items");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("超出范围"));
    }

    #[test]
    fn validate_idx_empty() {
        assert!(validate_idx(0, 0, "items").is_err());
        assert!(validate_idx(0, 1, "items").is_ok());
    }
}
