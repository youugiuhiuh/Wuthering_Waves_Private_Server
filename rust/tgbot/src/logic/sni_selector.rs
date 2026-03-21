use once_cell::sync::Lazy;
use rand::seq::SliceRandom;
use rand::thread_rng;
use rust_embed::RustEmbed;
use std::collections::HashMap;
use std::sync::RwLock;

use crate::logic::config::RealityProto;

// Embedded SNI resources
#[derive(RustEmbed)]
#[folder = "src/resources/sni/"]
struct SniAssets;

// Cache map: Country Code -> List of Domains
static SNI_CACHE: Lazy<RwLock<HashMap<String, Vec<String>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

pub struct SNISelector {
    domains: Vec<String>,
    index: usize,
}

impl SNISelector {
    /// Create a new selector for the given country code and protocol.
    pub fn get_for_country(country_code: &str, proto: RealityProto) -> Self {
        // Normalize country code (e.g., UK -> GB)
        let upper = country_code.to_uppercase();
        let code = match upper.as_str() {
            "UK" => "GB",
            c => c,
        };

        let proto_prefix = match proto {
            RealityProto::Vision => "reality",
            RealityProto::XHTTP => "xhttp",
        };

        let cache_key = format!("{}:{}", proto_prefix, code);

        // 1. Try Memory Cache
        {
            let cache = SNI_CACHE.read().unwrap();
            if let Some(domains) = cache.get(&cache_key) {
                if !domains.is_empty() {
                    return Self::new_from_list(domains.clone());
                }
            }
        }

        // 2. Try Load from Embedded Resource
        // Priority: subfolder -> root folder -> default
        let code_upper = code.to_uppercase();
        let bin_file = format!("{}/{}.bin", proto_prefix, code_upper);
        let txt_file = format!("{}/{}.txt", proto_prefix, code_upper);
        let fallback_bin = format!("{}.bin", code_upper);
        let fallback_txt = format!("{}.txt", code_upper);

        let domains = Self::load_embedded(&bin_file)
            .or_else(|| Self::load_embedded(&txt_file))
            .or_else(|| Self::load_embedded(&fallback_bin))
            .or_else(|| Self::load_embedded(&fallback_txt))
            .or_else(|| Self::load_embedded("default.bin"))
            .or_else(|| Self::load_embedded("default.txt"))
            .unwrap_or_else(|| vec!["www.google.com".to_string()]);

        // 3. Update Cache
        {
            let mut cache = SNI_CACHE.write().unwrap();
            cache.insert(cache_key, domains.clone());
        }

        Self::new_from_list(domains)
    }

    fn new_from_list(mut domains: Vec<String>) -> Self {
        let mut rng = thread_rng();
        domains.shuffle(&mut rng);
        Self { domains, index: 0 }
    }

    /// Get the next domain in the rotation.
    pub fn next(&mut self) -> String {
        if self.domains.is_empty() {
            return "www.google.com".to_string(); // Ultimate fallback
        }

        if self.index >= self.domains.len() {
            self.index = 0;
            self.shuffle();
        }

        let domain = self.domains[self.index].clone();
        self.index += 1;
        domain
    }

    fn shuffle(&mut self) {
        let mut rng = thread_rng();
        self.domains.shuffle(&mut rng);
    }

    /// Load embedded file with automatic format detection (Binary vs TXT)
    fn load_embedded(filename: &str) -> Option<Vec<String>> {
        let file = SniAssets::get(filename)?;
        let data = file.data.as_ref();

        // Try Binary format first
        if is_binary_format(data) {
            if let Some(domains) = load_binary(data) {
                return Some(domains);
            }
        }

        // Fallback to TXT format
        let text = std::str::from_utf8(data).ok()?;
        load_text(text)
    }
}

/// Load Binary format: [2-byte length (big-endian)] + [domain bytes]
fn load_binary(data: &[u8]) -> Option<Vec<String>> {
    let mut domains = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        if offset + 2 > data.len() {
            break;
        }
        // Read 2-byte length (big-endian)
        let length = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;

        if length == 0 || length > 512 {
            // Invalid length, not binary format
            return None;
        }

        if offset + length > data.len() {
            break;
        }

        // Read domain bytes
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

/// Detect if data is in Binary format
/// Binary format check: valid length prefix chain
fn is_binary_format(data: &[u8]) -> bool {
    // Minimum size: at least 4 bytes (2 domains minimum)
    if data.len() < 4 || data.len() > 10 * 1024 * 1024 {
        return false;
    }

    let mut offset = 0;
    let mut count = 0;
    let max_domains = 1000; // Safety limit

    while offset < data.len() && count < max_domains {
        if offset + 2 > data.len() {
            return false;
        }

        // Read length prefix
        let length = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;

        // Length validation
        if length == 0 || length > 512 {
            return false;
        }

        // Check if domain portion is valid UTF-8 and contains dot
        if offset + length > data.len() {
            return false;
        }

        // Check if it looks like a domain (contains at least one dot)
        let slice = &data[offset..offset + length];
        if !slice.contains(&b'.') {
            // Not a domain, might be text format
            return false;
        }

        // Check if it's printable ASCII
        for &b in slice {
            if !(b == 46
                || (b >= 48 && b <= 57)
                || (b >= 97 && b <= 122)
                || (b >= 65 && b <= 90)
                || b == 45
                || b == 95)
            {
                // Contains special chars other than dot, dash, underscore
                // This might still be valid, but be conservative
            }
        }

        offset += length;
        count += 1;
    }

    // Must have found at least 3 domains to be considered binary
    // (avoid false positive on short text files)
    count >= 3
}

/// Load TXT format (legacy): one domain per line
fn load_text(content: &str) -> Option<Vec<String>> {
    let mut domains = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        // Ignore comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        // Cleanup quotes and commas
        let clean = trimmed
            .trim_matches(|c| c == '"' || c == '\'')
            .trim_end_matches(',');

        // Normalize: remove port if present
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
        // Test with default.txt (should exist)
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
        // Valid binary data: "example.com" (11 bytes)
        let binary_data = vec![
            0x00, 0x0B, // length = 11
            0x65, 0x78, 0x61, 0x6D, 0x70, 0x6C, 0x65, 0x2E, 0x63, 0x6F, 0x6D, // "example.com"
        ];
        assert!(is_binary_format(&binary_data));

        // Invalid: too short
        assert!(!is_binary_format(&[0x00, 0x0B]));
    }

    #[test]
    fn load_binary_parses_correctly() {
        // "example.com" (11 bytes)
        let binary_data = vec![
            0x00, 0x0B, 0x65, 0x78, 0x61, 0x6D, 0x70, 0x6C, 0x65, 0x2E, 0x63, 0x6F, 0x6D,
            // "test.com" (9 bytes)
            0x00, 0x09, 0x74, 0x65, 0x73, 0x74, 0x2E, 0x63, 0x6F, 0x6D,
        ];

        let domains = load_binary(&binary_data);
        assert!(domains.is_some());
        let domains = domains.unwrap();
        assert_eq!(domains.len(), 2);
        assert_eq!(domains[0], "example.com");
        assert_eq!(domains[1], "test.com");
    }
}
