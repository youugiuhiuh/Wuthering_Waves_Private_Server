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

static LARGEST_BIN_REALITY: Lazy<Option<String>> =
    Lazy::new(|| find_largest_bin_in_protocol("reality"));

static LARGEST_BIN_XHTTP: Lazy<Option<String>> =
    Lazy::new(|| find_largest_bin_in_protocol("xhttp"));

fn find_largest_bin_in_protocol(proto_prefix: &str) -> Option<String> {
    let prefix_path = format!("{}/", proto_prefix);
    let mut largest_file: Option<String> = None;
    let mut largest_size: usize = 0;

    for file in SniAssets::iter() {
        let filename = file.as_ref();
        if !filename.starts_with(&prefix_path) {
            continue;
        }
        if !filename.ends_with(".bin") {
            continue;
        }

        if let Some(asset) = SniAssets::get(filename) {
            let size = asset.data.as_ref().len();
            if size > largest_size {
                largest_size = size;
                largest_file = Some(filename.to_string());
            }
        }
    }

    if let Some(ref f) = largest_file {
        log::info!(
            "Found largest .bin for {}: {} ({} bytes)",
            proto_prefix,
            f,
            largest_size
        );
    }
    largest_file
}

pub struct SNISelector {
    domains: Vec<String>,
    shuffled_indices: Vec<usize>,
    used_count: usize,
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
                    "Loaded persisted SNI state for {}: {} domains, remaining={}, used={}",
                    cache_key,
                    state.domains.len(),
                    state.shuffled_indices.len(),
                    state.used_count
                );
                return Self {
                    domains: state.domains,
                    shuffled_indices: state.shuffled_indices,
                    used_count: state.used_count,
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
            shuffled_indices: state.shuffled_indices,
            used_count: state.used_count,
            cache_key,
        }
    }

    fn load_domains(proto_prefix: &str, code: &str) -> Vec<String> {
        let code_upper = code.to_uppercase();
        let bin_file = format!("{}/{}.bin", proto_prefix, code_upper);
        let txt_file = format!("{}/{}.txt", proto_prefix, code_upper);
        let fallback_bin = format!("{}.bin", code_upper);
        let fallback_txt = format!("{}.txt", code_upper);

        let largest_cache: &Option<String> = match proto_prefix {
            "reality" => &LARGEST_BIN_REALITY,
            "xhttp" => &LARGEST_BIN_XHTTP,
            _ => &None,
        };

        Self::load_embedded(&bin_file)
            .or_else(|| Self::load_embedded(&txt_file))
            .or_else(|| Self::load_embedded(&fallback_bin))
            .or_else(|| Self::load_embedded(&fallback_txt))
            .or_else(|| largest_cache.as_ref().and_then(|f| Self::load_embedded(f)))
            .or_else(|| Self::load_embedded("default.bin"))
            .or_else(|| Self::load_embedded("default.txt"))
            .unwrap_or_else(|| vec!["www.google.com".to_string()])
    }

    pub fn next(&mut self) -> String {
        if self.domains.is_empty() {
            return "www.google.com".to_string();
        }

        if self.shuffled_indices.is_empty() {
            self.reset_shuffled_indices();
            self.save_state();
        }

        let idx = self.shuffled_indices.pop().unwrap();
        self.used_count += 1;
        self.save_state();

        self.domains[idx].clone()
    }

    fn reset_shuffled_indices(&mut self) {
        let mut indices: Vec<usize> = (0..self.domains.len()).collect();
        let mut rng = thread_rng();
        indices.shuffle(&mut rng);
        self.shuffled_indices = indices;
    }

    fn save_state(&self) {
        if self.cache_key.is_empty() {
            return;
        }
        if let Some(ref persistence) = *SNI_PERSISTENCE {
            let state = SNIState {
                domains: self.domains.clone(),
                shuffled_indices: self.shuffled_indices.clone(),
                used_count: self.used_count,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            if let Err(e) = persistence.save(&self.cache_key, &state) {
                log::warn!("Failed to save SNI state: {}", e);
            }
        }
    }

    pub fn remaining(&self) -> usize {
        self.shuffled_indices.len()
    }

    pub fn total_used(&self) -> usize {
        self.used_count
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
    fn next_random_no_repeat() {
        let mut selector = SNISelector {
            domains: vec![
                "a.com".to_string(),
                "b.com".to_string(),
                "c.com".to_string(),
                "d.com".to_string(),
                "e.com".to_string(),
            ],
            shuffled_indices: vec![0, 1, 2, 3, 4],
            used_count: 0,
            cache_key: String::new(),
        };

        let mut results = Vec::new();
        for _ in 0..5 {
            results.push(selector.next());
        }

        let unique: std::collections::HashSet<_> = results.iter().collect();
        assert_eq!(unique.len(), 5, "Should have 5 unique domains");
        assert_eq!(selector.remaining(), 0);
    }

    #[test]
    fn next_resets_when_exhausted() {
        let mut selector = SNISelector {
            domains: vec!["a.com".to_string(), "b.com".to_string()],
            shuffled_indices: vec![0, 1],
            used_count: 0,
            cache_key: String::new(),
        };

        selector.next();
        selector.next();
        assert_eq!(selector.remaining(), 0);

        selector.next();
        assert_eq!(selector.remaining(), 1);
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
    fn remaining_count() {
        let mut selector = SNISelector {
            domains: vec![
                "a.com".to_string(),
                "b.com".to_string(),
                "c.com".to_string(),
            ],
            shuffled_indices: vec![0, 1, 2],
            used_count: 0,
            cache_key: String::new(),
        };

        assert_eq!(selector.remaining(), 3);
        selector.next();
        assert_eq!(selector.remaining(), 2);
        selector.next();
        assert_eq!(selector.remaining(), 1);
    }

    #[test]
    fn find_largest_bin_finds_us_file() {
        let result = find_largest_bin_in_protocol("reality");
        assert!(result.is_some());
        let filename = result.unwrap();
        assert!(
            filename.ends_with("US.bin"),
            "Expected US.bin, got {}",
            filename
        );

        let result = find_largest_bin_in_protocol("xhttp");
        assert!(result.is_some());
        let filename = result.unwrap();
        assert!(
            filename.ends_with("US.bin"),
            "Expected US.bin, got {}",
            filename
        );
    }

    #[test]
    fn get_for_country_unknown_uses_largest_bin_fallback() {
        let selector = SNISelector::get_for_country("XXXUNKNOWN", RealityProto::Vision);
        assert!(
            !selector.domains.is_empty(),
            "Domains should not be empty for unknown country"
        );
        assert!(
            selector.domains.iter().all(|d| d.contains('.')),
            "All domains should contain dots"
        );

        let selector = SNISelector::get_for_country("ZZZ999", RealityProto::XHTTP);
        assert!(
            !selector.domains.is_empty(),
            "Domains should not be empty for unknown country (xhttp)"
        );
    }

    #[test]
    fn lazy_cache_is_initialized() {
        assert!(
            LARGEST_BIN_REALITY.is_some(),
            "LARGEST_BIN_REALITY should be initialized"
        );
        assert!(
            LARGEST_BIN_XHTTP.is_some(),
            "LARGEST_BIN_XHTTP should be initialized"
        );
    }
}
