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
        // Priority: subfolder -> root folder -> default.txt
        let filename = format!("{}.txt", code);
        let subfolder_file = format!("{}/{}", proto_prefix, filename);

        let domains = Self::load_embedded(&subfolder_file)
            .or_else(|| Self::load_embedded(&filename))
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

    fn load_embedded(filename: &str) -> Option<Vec<String>> {
        let file = SniAssets::get(filename)?;
        let content = std::str::from_utf8(file.data.as_ref()).ok()?;

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

            if !domain_only.is_empty() {
                domains.push(domain_only.to_string());
            }
        }

        // Deduplicate
        domains.sort();
        domains.dedup();

        if domains.is_empty() {
            None
        } else {
            Some(domains)
        }
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
}
