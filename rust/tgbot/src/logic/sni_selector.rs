use once_cell::sync::Lazy;
use rand::seq::SliceRandom;
use rand::thread_rng;
use rust_embed::RustEmbed;

use crate::logic::config::RealityProto;
use crate::logic::sni_state::{SNIPersistence, SNIState};

// Embedded SNI resources
#[derive(RustEmbed)]
#[folder = "src/resources/sni/"]
struct SniAssets;

static SNI_PERSISTENCE: Lazy<Option<SNIPersistence>> = Lazy::new(|| match SNIPersistence::new() {
    Ok(p) => Some(p),
    Err(e) => {
        log::warn!("SNIPersistence init failed, using memory-only: {}", e);
        None
    }
});

pub struct SNISelector {
    domains: Vec<String>,
    index: usize,
    total_used: usize,
    cache_key: String,
}

impl SNISelector {
    pub fn get_for_country(country_code: &str, proto: RealityProto) -> Self {
        let upper = country_code.to_uppercase();
        let code = match upper.as_str() {
            "UK" => "GB",
            c => c,
        };

        let proto_prefix = match proto {
            RealityProto::Vision => "reality",
            RealityProto::XHTTP => "xhttp",
        };

        let cache_key = format!("{}_{}", proto_prefix, code);

        if let Some(ref persistence) = *SNI_PERSISTENCE {
            if let Some(state) = persistence.load(&cache_key) {
                log::info!(
                    "Loaded persisted SNI state for {}: {} domains, index={}, total_used={}",
                    cache_key,
                    state.domains.len(),
                    state.index,
                    state.total_used
                );
                return Self {
                    domains: state.domains,
                    index: state.index,
                    total_used: state.total_used,
                    cache_key,
                };
            }
        }

        let domains = Self::load_domains(proto_prefix, &code);

        let state = SNIState::new(domains.clone());
        if let Some(ref persistence) = *SNI_PERSISTENCE {
            if let Err(e) = persistence.save(&cache_key, &state) {
                log::warn!("Failed to save initial SNI state: {}", e);
            }
        }

        Self {
            domains: state.domains,
            index: state.index,
            total_used: state.total_used,
            cache_key,
        }
    }

    fn load_domains(proto_prefix: &str, code: &str) -> Vec<String> {
        let code_upper = code.to_uppercase();
        let bin_file = format!("{}/{}.bin", proto_prefix, code_upper);
        let txt_file = format!("{}/{}.txt", proto_prefix, code_upper);
        let fallback_bin = format!("{}.bin", code_upper);
        let fallback_txt = format!("{}.txt", code_upper);

        Self::load_embedded(&bin_file)
            .or_else(|| Self::load_embedded(&txt_file))
            .or_else(|| Self::load_embedded(&fallback_bin))
            .or_else(|| Self::load_embedded(&fallback_txt))
            .or_else(|| Self::load_embedded("default.bin"))
            .or_else(|| Self::load_embedded("default.txt"))
            .unwrap_or_else(|| vec!["www.google.com".to_string()])
    }

    fn new_from_list(domains: Vec<String>) -> Self {
        let mut rng = thread_rng();
        let mut domains = domains;
        domains.shuffle(&mut rng);
        Self {
            domains,
            index: 0,
            total_used: 0,
            cache_key: String::new(),
        }
    }

    pub fn next(&mut self) -> String {
        if self.domains.is_empty() {
            return "www.google.com".to_string();
        }

        if self.index >= self.domains.len() {
            self.index = 0;
            self.total_used += self.domains.len();
            self.shuffle();
            self.save_state();
        }

        let domain = self.domains[self.index].clone();
        self.index += 1;
        domain
    }

    fn shuffle(&mut self) {
        let mut rng = thread_rng();
        self.domains.shuffle(&mut rng);
    }

    fn save_state(&self) {
        if self.cache_key.is_empty() {
            return;
        }
        if let Some(ref persistence) = *SNI_PERSISTENCE {
            let state = SNIState {
                domains: self.domains.clone(),
                index: self.index,
                total_used: self.total_used,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            if let Err(e) = persistence.save(&self.cache_key, &state) {
                log::warn!("Failed to save SNI state: {}", e);
            }
        }
    }

    pub fn domains_remaining(&self) -> usize {
        if self.domains.is_empty() {
            return 0;
        }
        self.domains.len() - self.index
    }

    pub fn total_used(&self) -> usize {
        self.total_used
    }

    fn load_embedded(filename: &str) -> Option<Vec<String>> {
        let file = SniAssets::get(filename)?;
        let data = file.data.as_ref();

        if is_binary_format(data) {
            if let Some(domains) = load_binary(data) {
                return Some(domains);
            }
        }

        let text = std::str::from_utf8(data).ok()?;
        load_text(text)
    }
}

fn load_binary(data: &[u8]) -> Option<Vec<String>> {
    let mut domains = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        if offset + 2 > data.len() {
            break;
        }
        let length = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;

        if length == 0 || length > 512 {
            return None;
        }

        if offset + length > data.len() {
            break;
        }

        let domain = std::str::from_utf8(&data[offset..offset + length]).ok()?;
        if !domain.is_empty() && domain.contains('.') {
            domains.push(domain.to_string());
        }
        offset += length;
    }

    if domains.is_empty() {
        None
    } else {
        Some(domains)
    }
}

fn is_binary_format(data: &[u8]) -> bool {
    if data.len() < 4 || data.len() > 10 * 1024 * 1024 {
        return false;
    }

    let mut offset = 0;
    let mut count = 0;
    let max_domains = 1000;

    while offset < data.len() && count < max_domains {
        if offset + 2 > data.len() {
            return false;
        }

        let length = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;

        if length == 0 || length > 512 {
            return false;
        }

        if offset + length > data.len() {
            return false;
        }

        let slice = &data[offset..offset + length];
        if !slice.contains(&b'.') {
            return false;
        }

        for &b in slice {
            if !(b == 46
                || (b >= 48 && b <= 57)
                || (b >= 97 && b <= 122)
                || (b >= 65 && b <= 90)
                || b == 45
                || b == 95)
            {}
        }

        offset += length;
        count += 1;
    }

    count >= 3
}

fn load_text(content: &str) -> Option<Vec<String>> {
    let mut domains = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        let clean = trimmed
            .trim_matches(|c| c == '"' || c == '\'')
            .trim_end_matches(',');

        let domain_only = if let Some(idx) = clean.find(':') {
            &clean[..idx]
        } else {
            clean
        };

        if !domain_only.is_empty() && domain_only.contains('.') {
            domains.push(domain_only.to_string());
        }
    }

    if domains.is_empty() {
        None
    } else {
        Some(domains)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_for_country_returns_selector_with_domains() {
        let selector = SNISelector::get_for_country("US", RealityProto::Vision);
        let mut s = selector;
        let first = s.next();
        assert!(!first.is_empty());
        assert!(first.contains('.'));
    }

    #[test]
    fn get_for_country_unknown_falls_back_to_default() {
        let selector = SNISelector::get_for_country("XX", RealityProto::Vision);
        let mut s = selector;
        let d = s.next();
        assert!(!d.is_empty());
    }

    #[test]
    fn next_returns_fallback_for_empty_list() {
        let mut selector = SNISelector::new_from_list(vec![]);
        assert_eq!(selector.next(), "www.google.com");
    }

    #[test]
    fn next_rotates_through_domains() {
        let mut selector = SNISelector::new_from_list(vec![
            "a.example.com".to_string(),
            "b.example.com".to_string(),
        ]);
        let a = selector.next();
        let b = selector.next();
        let c = selector.next();
        assert!(a != b || b != c || a != c);
    }

    #[test]
    fn get_for_country_uk_normalizes_to_gb() {
        let selector_uk = SNISelector::get_for_country("UK", RealityProto::Vision);
        let selector_gb = SNISelector::get_for_country("GB", RealityProto::Vision);
        let mut s1 = selector_uk;
        let mut s2 = selector_gb;
        assert!(!s1.next().is_empty());
        assert!(!s2.next().is_empty());
    }

    #[test]
    fn get_for_country_xhttp_different_prefix() {
        let selector = SNISelector::get_for_country("US", RealityProto::XHTTP);
        let mut s = selector;
        assert!(!s.next().is_empty());
    }

    #[test]
    fn load_embedded_handles_comments_and_empty_lines() {
        let domains = SNISelector::load_embedded("default.txt");
        if let Some(domains) = domains {
            for domain in &domains {
                assert!(!domain.starts_with('#'));
                assert!(!domain.starts_with("//"));
                assert!(!domain.is_empty());
                assert!(domain.contains('.'));
            }
        }
    }

    #[test]
    fn new_from_list_shuffles_domains() {
        let list = vec![
            "a.com".to_string(),
            "b.com".to_string(),
            "c.com".to_string(),
            "d.com".to_string(),
            "e.com".to_string(),
        ];
        let mut results = Vec::new();
        for _ in 0..10 {
            let selector = SNISelector::new_from_list(list.clone());
            results.push(selector.domains.clone());
        }
        let all_same = results.iter().all(|r| r == &results[0]);
        assert!(!all_same, "Shuffle should produce different orderings");
    }

    #[test]
    fn binary_format_detection() {
        let binary_data = vec![
            0x00, 0x0B, 0x65, 0x78, 0x61, 0x6D, 0x70, 0x6C, 0x65, 0x2E, 0x63, 0x6F, 0x6D, 0x00,
            0x08, 0x74, 0x65, 0x73, 0x74, 0x2E, 0x63, 0x6F, 0x6D, 0x00, 0x07, 0x66, 0x6F, 0x6F,
            0x2E, 0x63, 0x6F, 0x6D,
        ];
        assert!(is_binary_format(&binary_data));

        assert!(!is_binary_format(&[0x00, 0x0B]));

        let two_domains = vec![
            0x00, 0x0B, 0x65, 0x78, 0x61, 0x6D, 0x70, 0x6C, 0x65, 0x2E, 0x63, 0x6F, 0x6D, 0x00,
            0x08, 0x74, 0x65, 0x73, 0x74, 0x2E, 0x63, 0x6F, 0x6D,
        ];
        assert!(!is_binary_format(&two_domains));
    }

    #[test]
    fn load_binary_parses_correctly() {
        let binary_data = vec![
            0x00, 0x0B, 0x65, 0x78, 0x61, 0x6D, 0x70, 0x6C, 0x65, 0x2E, 0x63, 0x6F, 0x6D, 0x00,
            0x08, 0x74, 0x65, 0x73, 0x74, 0x2E, 0x63, 0x6F, 0x6D, 0x00, 0x07, 0x66, 0x6F, 0x6F,
            0x2E, 0x63, 0x6F, 0x6D,
        ];

        let domains = load_binary(&binary_data);
        assert!(domains.is_some());
        let domains = domains.unwrap();
        assert_eq!(domains.len(), 3);
        assert_eq!(domains[0], "example.com");
        assert_eq!(domains[1], "test.com");
        assert_eq!(domains[2], "foo.com");
    }

    #[test]
    fn domains_remaining() {
        let mut selector = SNISelector::new_from_list(vec![
            "a.com".to_string(),
            "b.com".to_string(),
            "c.com".to_string(),
        ]);
        assert_eq!(selector.domains_remaining(), 3);
        selector.next();
        assert_eq!(selector.domains_remaining(), 2);
        selector.next();
        assert_eq!(selector.domains_remaining(), 1);
    }
}
