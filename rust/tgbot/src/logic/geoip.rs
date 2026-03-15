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
