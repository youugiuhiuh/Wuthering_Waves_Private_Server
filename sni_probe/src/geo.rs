//! IPinfo Lite / MaxMind MMDB 国家与 ASN 查询，以及审查管辖国排除。
//!
//! 同一个 `ipinfo_lite.mmdb` 同时提供 country 与 asn 字段，因此只维护一个读取器。

use std::net::IpAddr;
use std::path::Path;

use maxminddb::Reader;

/// 严禁作为 REALITY 伪装目标的审查管辖国 / 地区。
///
/// 借用审查方境内的目标会造成流量反向异常（审查系统看到"出境到 VPS，却带着指向境内的 SNI"），
/// 并且会让目标暴露在旁路验证之下。
const BLOCKED_COUNTRIES: &[&str] = &["CN", "HK", "MO", "IR", "RU", "KP"];

/// 判断 ISO 国家码是否属于审查管辖国。
pub fn is_blocked_country(code: &str) -> bool {
    BLOCKED_COUNTRIES.contains(&code.to_uppercase().as_str())
}

/// MMDB 记录：兼容 IPinfo Lite 扁平 schema 与 MaxMind 嵌套 schema。
#[derive(serde::Deserialize, Default)]
#[allow(dead_code)] // ASN 字段供后续 badger 等价存储使用
struct GeoRecord {
    // IPinfo Lite
    country: Option<String>,
    country_code: Option<String>,
    asn: Option<String>,
    as_name: Option<String>,
    as_domain: Option<String>,
    // MaxMind GeoLite2
    maxmind_country: Option<MaxMindCountry>,
    maxmind_registered_country: Option<MaxMindCountry>,
    autonomous_system_number: Option<u32>,
    autonomous_system_organization: Option<String>,
}

#[derive(serde::Deserialize)]
struct MaxMindCountry {
    iso_code: Option<String>,
}

#[allow(dead_code)]
impl GeoRecord {
    fn country(&self) -> String {
        self.country_code
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_uppercase)
            .or_else(|| {
                self.country
                    .as_deref()
                    .filter(|s| s.len() == 2)
                    .map(str::to_uppercase)
            })
            .or_else(|| {
                self.maxmind_country
                    .as_ref()
                    .and_then(|c| c.iso_code.as_deref())
                    .filter(|s| !s.is_empty())
                    .map(str::to_uppercase)
            })
            .or_else(|| {
                self.maxmind_registered_country
                    .as_ref()
                    .and_then(|c| c.iso_code.as_deref())
                    .filter(|s| !s.is_empty())
                    .map(str::to_uppercase)
            })
            .unwrap_or_else(|| "UNKNOWN".to_string())
    }

    fn asn(&self) -> Option<(u32, String)> {
        if let Some(raw) = self.asn.as_deref().filter(|s| !s.is_empty()) {
            let clean = raw.trim_start_matches("AS").trim_start_matches("as");
            if let Ok(n) = clean.parse::<u32>() {
                let org = self
                    .as_name
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| self.as_domain.clone().filter(|s| !s.is_empty()))
                    .unwrap_or_default();
                return Some((n, org));
            }
        }
        self.autonomous_system_number.map(|n| {
            (
                n,
                self.autonomous_system_organization
                    .clone()
                    .unwrap_or_default(),
            )
        })
    }
}

/// MMDB 查询器。
pub struct GeoDb {
    reader: Reader<Vec<u8>>,
}

impl GeoDb {
    /// 打开 MMDB 文件（ipinfo_lite.mmdb 或 GeoLite2-Country/ASN）。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self {
            reader: Reader::open_readfile(path)?,
        })
    }

    /// 一次性解码 IP 的 MMDB 记录；未命中或解码失败返回 `None`。
    fn record(&self, ip: IpAddr) -> Option<GeoRecord> {
        self.reader.lookup(ip).ok()?.decode().ok().flatten()
    }

    /// 查询国家码，未命中返回 `UNKNOWN`。
    pub fn country(&self, ip: IpAddr) -> String {
        self.record(ip)
            .map(|r| r.country())
            .unwrap_or_else(|| "UNKNOWN".to_string())
    }

    /// 查询 ASN 与组织名。
    #[allow(dead_code)]
    pub fn asn(&self, ip: IpAddr) -> Option<(u32, String)> {
        self.record(ip)?.asn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_censoring_jurisdictions() {
        for c in ["CN", "HK", "MO", "IR", "RU", "KP", "cn", "ir"] {
            assert!(is_blocked_country(c), "应拦截 {c}");
        }
    }

    #[test]
    fn allows_free_countries() {
        for c in ["US", "DE", "JP", "SG", "TW", "GB", "UNKNOWN", ""] {
            assert!(!is_blocked_country(c), "不应拦截 {c}");
        }
    }

    #[test]
    fn geo_record_extracts_ipinfo_schema() {
        let rec: GeoRecord = serde_json::from_str(
            r#"{"country":"US","country_code":"US","asn":"AS15169","as_name":"Google LLC"}"#,
        )
        .unwrap();
        assert_eq!(rec.country(), "US");
        assert_eq!(rec.asn(), Some((15169, "Google LLC".to_string())));
    }

    #[test]
    fn geo_record_extracts_maxmind_schema() {
        let rec: GeoRecord = serde_json::from_str(
            r#"{"maxmind_country":{"iso_code":"DE"},"autonomous_system_number":24940,
                "autonomous_system_organization":"Hetzner Online GmbH"}"#,
        )
        .unwrap();
        assert_eq!(rec.country(), "DE");
        assert_eq!(rec.asn(), Some((24940, "Hetzner Online GmbH".to_string())));
    }

    #[test]
    fn geo_record_defaults_to_unknown() {
        let rec: GeoRecord = serde_json::from_str("{}").unwrap();
        assert_eq!(rec.country(), "UNKNOWN");
        assert_eq!(rec.asn(), None);
    }
}
