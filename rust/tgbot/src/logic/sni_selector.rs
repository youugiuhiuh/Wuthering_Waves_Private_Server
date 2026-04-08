use once_cell::sync::Lazy;
use prost::Message;
use rand::seq::SliceRandom;
use rand::thread_rng;
use rust_embed::RustEmbed;

use crate::logic::sni_state::{SNIPersistence, SNIState};

pub mod sni_proto {
    include!(concat!(env!("OUT_DIR"), "/sni.rs"));
}

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

static LARGEST_PB: Lazy<Option<(String, usize)>> = Lazy::new(|| find_file_with_most_domains());

fn find_file_with_most_domains() -> Option<(String, usize)> {
    let mut best_file: Option<String> = None;
    let mut best_count: usize = 0;

    for file in SniAssets::iter() {
        let filename = file.as_ref();
        if !filename.ends_with(".pb") {
            continue;
        }

        if let Some(asset) = SniAssets::get(filename) {
            let data = asset.data.as_ref();
            if let Some(domains) = load_protobuf(data) {
                if domains.len() > best_count {
                    best_count = domains.len();
                    best_file = Some(filename.to_string());
                }
            }
        }
    }

    if let Some(ref f) = best_file {
        log::info!(
            "Found file with most domains: {} ({} domains)",
            f,
            best_count
        );
    }
    best_file.map(|f| (f, best_count))
}

fn load_protobuf(data: &[u8]) -> Option<Vec<String>> {
    sni_proto::DomainList::decode(data)
        .ok()
        .map(|dl| dl.domains)
}

pub struct SNISelector {
    domains: Vec<String>,
    shuffled_indices: Vec<usize>,
    used_count: usize,
    cache_key: String,
}

impl SNISelector {
    pub fn get_for_country(country_code: &str) -> Self {
        let upper = country_code.to_uppercase();
        let code = match upper.as_str() {
            "UK" => "GB",
            c => c,
        };

        let domains = Self::load_domains(&code);
        let state = SNIState::new(domains.clone());

        let cache_key = format!("sni_{}", code);

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

    fn load_domains(code: &str) -> Vec<String> {
        const MIN_DOMAINS: usize = 3;

        let code_upper = code.to_uppercase();
        let pb_file = format!("{}.pb", code_upper);

        let country_domains = Self::load_embedded(&pb_file);

        if let Some(domains) = country_domains {
            if domains.len() >= MIN_DOMAINS {
                return domains;
            }
            log::warn!(
                "SNI file for {} has only {} domains (< {}), falling back to file with most domains",
                code_upper,
                domains.len(),
                MIN_DOMAINS
            );
        }

        if let Some((filename, count)) = LARGEST_PB.as_ref() {
            log::info!("Using fallback file: {} ({} domains)", filename, count);
            if let Some(domains) = Self::load_embedded(filename) {
                return domains;
            }
        }

        vec![]
    }

    pub fn next(&mut self) -> String {
        if self.domains.is_empty() {
            return String::new();
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
        load_protobuf(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_for_country_returns_selector_with_domains() {
        let selector = SNISelector::get_for_country("US");
        let mut s = selector;
        let first = s.next();
        assert!(!first.is_empty());
        assert!(first.contains('.'));
    }

    #[test]
    fn get_for_country_unknown_falls_back_to_default() {
        let selector = SNISelector::get_for_country("XX");
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
        let selector_uk = SNISelector::get_for_country("UK");
        let selector_gb = SNISelector::get_for_country("GB");
        let mut s1 = selector_uk;
        let mut s2 = selector_gb;
        assert!(!s1.next().is_empty());
        assert!(!s2.next().is_empty());
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
    fn load_protobuf_decodes_valid_data() {
        let domains = vec!["example.com".to_string(), "test.com".to_string()];
        let list = sni_proto::DomainList { domains };
        let mut buf = Vec::new();
        list.encode(&mut buf).unwrap();
        let decoded = load_protobuf(&buf).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0], "example.com");
        assert_eq!(decoded[1], "test.com");
    }
}
