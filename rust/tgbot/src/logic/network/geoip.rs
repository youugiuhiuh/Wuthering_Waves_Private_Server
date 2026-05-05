use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct GeoIPLocation {
    pub ip: String,
    pub location: LocationInfo,
}

#[derive(Debug, Deserialize)]
pub struct LocationInfo {
    pub city: String,
    pub country_code: String,
    pub country_name: String,
}

impl GeoIPLocation {
    pub fn country_code(&self) -> &str {
        &self.location.country_code
    }
}

// Support for ip.sb format
#[derive(Debug, Deserialize)]
struct IpSbLocation {
    ip: String,
    country_code: String,
    #[serde(rename = "country")]
    country_name: String,
    city: String,
}

impl From<IpSbLocation> for GeoIPLocation {
    fn from(sb: IpSbLocation) -> Self {
        GeoIPLocation {
            ip: sb.ip,
            location: LocationInfo {
                city: sb.city,
                country_code: sb.country_code,
                country_name: sb.country_name,
            },
        }
    }
}

pub struct GeoIPService {
    client: Client,
}

impl GeoIPService {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent("Mozilla/5.0 (compatible; wwps/4.0)")
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    pub async fn fetch_location(&self) -> Result<GeoIPLocation> {
        // Try Primary API
        match self.fetch_primary().await {
            Ok(loc) => return Ok(loc),
            Err(e) => log::warn!("Primary GeoIP API failed: {}", e),
        }

        // Try Backup API
        match self.fetch_backup().await {
            Ok(loc) => return Ok(loc),
            Err(e) => log::warn!("Backup GeoIP API failed: {}", e),
        }

        anyhow::bail!("All GeoIP APIs failed")
    }

    async fn fetch_primary(&self) -> Result<GeoIPLocation> {
        let url = "https://api.myip.la/en?json";
        let resp = self.client.get(url).send().await?.error_for_status()?;
        let loc: GeoIPLocation = resp.json().await?;
        Ok(loc)
    }

    async fn fetch_backup(&self) -> Result<GeoIPLocation> {
        let url = "https://api.ip.sb/geoip";
        let resp = self.client.get(url).send().await?.error_for_status()?;
        let sb_loc: IpSbLocation = resp.json().await?;
        Ok(sb_loc.into())
    }

    pub async fn get_country_code(&self) -> String {
        match self.fetch_location().await {
            Ok(loc) => loc.country_code().to_string(),
            Err(_) => "US".to_string(), // Default fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_geoip_location_country_code() {
        let json = r#"{"ip":"1.1.1.1","location":{"city":"Test","country_code":"AU","country_name":"Australia"}}"#;
        let loc: GeoIPLocation = serde_json::from_str(json).unwrap();
        assert_eq!(loc.country_code(), "AU");
        assert_eq!(loc.ip, "1.1.1.1");
    }

    #[test]
    fn test_ip_sb_location_conversion() {
        let json = r#"{"ip":"8.8.8.8","country_code":"US","country":"United States","city":"Mountain View"}"#;
        let sb_loc: IpSbLocation = serde_json::from_str(json).unwrap();
        let loc: GeoIPLocation = sb_loc.into();
        assert_eq!(loc.ip, "8.8.8.8");
        assert_eq!(loc.location.country_code, "US");
        assert_eq!(loc.location.country_name, "United States");
        assert_eq!(loc.location.city, "Mountain View");
    }

    #[test]
    fn test_geoip_location_missing_fields() {
        let json = r#"{"ip":"127.0.0.1"}"#;
        let result: Result<GeoIPLocation, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
