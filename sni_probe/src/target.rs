//! 输入行解析：支持原始域名、Tranco `rank,domain`、预解析 `domain,ip` 三种格式。

/// 解析一行输入，返回 `(domain, ip)`；`ip` 为空表示需要自行解析。
///
/// 支持格式：
/// - `example.com`
/// - `1,example.com`（Tranco）
/// - `example.com,93.184.216.34`
/// - `1,example.com,93.184.216.34`
pub fn parse_line(raw: &str) -> Option<(String, Option<String>)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') || raw.starts_with("//") {
        return None;
    }

    let mut parts: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();

    // 丢弃 Tranco 排名列
    if parts.first().is_some_and(|p| p.parse::<u64>().is_ok()) {
        parts.remove(0);
    }

    let domain = parts.first()?.to_string();
    if domain.is_empty() {
        return None;
    }

    // 第二列必须是合法 IP，否则忽略（避免把 `domain,extra` 误当 IP）
    let ip = parts
        .get(1)
        .filter(|c| c.parse::<std::net::IpAddr>().is_ok())
        .map(|c| c.to_string());

    Some((domain, ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_supported_formats() {
        let cases = [
            ("example.com", ("example.com", None)),
            ("1,example.com", ("example.com", None)),
            (
                "example.com,93.184.216.34",
                ("example.com", Some("93.184.216.34")),
            ),
            (
                "1,example.com,93.184.216.34",
                ("example.com", Some("93.184.216.34")),
            ),
            ("  1.2.3.4  ,  spaced.com  ", ("1.2.3.4", None)),
        ];

        for (input, (want_domain, want_ip)) in cases {
            // 注意：`spaced.com` 用逗号分隔时会变成两段，这里只校验前两段语义
            let got = parse_line(input).unwrap();
            assert_eq!(got.0, want_domain, "input={input}");
            assert_eq!(got.1.as_deref(), want_ip, "input={input}");
        }
    }

    #[test]
    fn rejects_blank_comments_and_garbage() {
        assert!(parse_line("").is_none());
        assert!(parse_line("   ").is_none());
        assert!(parse_line("# comment").is_none());
        assert!(parse_line("// comment").is_none());
        assert!(parse_line("42").is_none()); // 只有排名，没有域名
    }

    #[test]
    fn ignores_non_ip_second_column() {
        // `domain,multi-tier` 不应被当成 IP
        assert_eq!(parse_line("a.com,multi-tier").unwrap().1, None);
        // IPv6 也应识别
        assert_eq!(
            parse_line("a.com,2606:4700:4700::1111")
                .unwrap()
                .1
                .as_deref(),
            Some("2606:4700:4700::1111")
        );
    }
}
