use rand::Rng;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::subscription::aggregator;
use crate::core::subscription::server::proto::{ProxyConfig, SubscriptionToken};

#[derive(Debug, Serialize, Deserialize)]
struct TokenRow {
    token: String,
    label: String,
    config_ids: String,
    created_at: i64,
    expires_at: i64,
    revoked: bool,
}

#[derive(Clone)]
pub struct TokenManager {
    db: std::sync::Arc<Mutex<Connection>>,
    public_ip: Option<String>,
}

impl TokenManager {
    pub fn new(
        db_path: &str,
        public_ip: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS subscription_tokens (
                token TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                config_ids TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL DEFAULT 0,
                revoked INTEGER NOT NULL DEFAULT 0
            )",
        )?;
        Ok(TokenManager {
            db: std::sync::Arc::new(Mutex::new(conn)),
            public_ip,
        })
    }

    fn generate_token() -> String {
        let charset: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let mut rng = rand::thread_rng();
        (0..32)
            .map(|_| {
                let idx = rng.gen_range(0..charset.len());
                charset[idx] as char
            })
            .collect()
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }

    fn row_to_token(row: &TokenRow) -> Result<SubscriptionToken, Box<dyn std::error::Error>> {
        let config_ids: Vec<String> = serde_json::from_str(&row.config_ids)?;
        Ok(SubscriptionToken {
            token: row.token.clone(),
            label: row.label.clone(),
            config_ids,
            created_at: row.created_at,
            expires_at: row.expires_at,
            revoked: row.revoked,
        })
    }

    pub fn create_token(
        &self,
        label: &str,
        config_ids: &[String],
    ) -> Result<SubscriptionToken, Box<dyn std::error::Error>> {
        let token = Self::generate_token();
        let now = Self::now();
        let config_ids_json = serde_json::to_string(config_ids)?;
        let db = self.db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO subscription_tokens (token, label, config_ids, created_at, expires_at, revoked) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![token, label, config_ids_json, now, 0, false],
        )?;
        Ok(SubscriptionToken {
            token,
            label: label.to_string(),
            config_ids: config_ids.to_vec(),
            created_at: now,
            expires_at: 0,
            revoked: false,
        })
    }

    pub fn list_tokens(
        &self,
        page: i32,
        page_size: i32,
    ) -> Result<(Vec<SubscriptionToken>, i64), Box<dyn std::error::Error>> {
        let offset = ((page.saturating_sub(1)).max(0)) * page_size;
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let total: i64 = db.query_row("SELECT COUNT(*) FROM subscription_tokens", [], |row| {
            row.get(0)
        })?;
        let mut stmt = db.prepare(
            "SELECT token, label, config_ids, created_at, expires_at, revoked FROM subscription_tokens ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![page_size, offset], |row| {
            let config_ids_str: String = row.get(2)?;
            Ok(TokenRow {
                token: row.get(0)?,
                label: row.get(1)?,
                config_ids: config_ids_str,
                created_at: row.get(3)?,
                expires_at: row.get(4)?,
                revoked: row.get::<_, i32>(5)? != 0,
            })
        })?;
        let mut tokens = Vec::new();
        for row in rows {
            tokens.push(Self::row_to_token(&row?)?);
        }
        Ok((tokens, total))
    }

    pub fn revoke_token(&self, token: &str) -> Result<(), Box<dyn std::error::Error>> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "UPDATE subscription_tokens SET revoked = 1 WHERE token = ?1",
            params![token],
        )?;
        Ok(())
    }

    pub fn update_token(
        &self,
        token: &str,
        config_ids: &[String],
        expires_at: i64,
    ) -> Result<SubscriptionToken, Box<dyn std::error::Error>> {
        let config_ids_json = serde_json::to_string(config_ids)?;
        let db = self.db.lock().map_err(|e| e.to_string())?;
        db.execute(
            "UPDATE subscription_tokens SET config_ids = ?1, expires_at = ?2 WHERE token = ?3",
            params![config_ids_json, expires_at, token],
        )?;
        let mut stmt = db.prepare(
            "SELECT token, label, config_ids, created_at, expires_at, revoked FROM subscription_tokens WHERE token = ?1",
        )?;
        let row = stmt.query_row(params![token], |row| {
            let config_ids_str: String = row.get(2)?;
            Ok(TokenRow {
                token: row.get(0)?,
                label: row.get(1)?,
                config_ids: config_ids_str,
                created_at: row.get(3)?,
                expires_at: row.get(4)?,
                revoked: row.get::<_, i32>(5)? != 0,
            })
        })?;
        Self::row_to_token(&row)
    }

    pub fn get_token_info(
        &self,
        token: &str,
    ) -> Result<(SubscriptionToken, usize), Box<dyn std::error::Error>> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let mut stmt = db.prepare(
            "SELECT token, label, config_ids, created_at, expires_at, revoked FROM subscription_tokens WHERE token = ?1",
        )?;
        let row = stmt.query_row(params![token], |row| {
            let config_ids_str: String = row.get(2)?;
            Ok(TokenRow {
                token: row.get(0)?,
                label: row.get(1)?,
                config_ids: config_ids_str,
                created_at: row.get(3)?,
                expires_at: row.get(4)?,
                revoked: row.get::<_, i32>(5)? != 0,
            })
        })?;
        let st = Self::row_to_token(&row)?;
        let count = st.config_ids.len();
        Ok((st, count))
    }

    pub fn get_configs_for_token(
        &self,
        token: &str,
    ) -> Result<Vec<ProxyConfig>, Box<dyn std::error::Error>> {
        let db = self.db.lock().map_err(|e| e.to_string())?;
        let (revoked, expires_at, config_ids_str): (bool, i64, String) = db
            .query_row(
                "SELECT revoked, expires_at, config_ids FROM subscription_tokens WHERE token = ?1",
                params![token],
                |row| {
                    Ok((
                        row.get::<_, i32>(0)? != 0,
                        row.get(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|e| -> Box<dyn std::error::Error> {
                match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        format!("token not found: {token}").into()
                    }
                    other => other.into(),
                }
            })?;
        if revoked {
            return Err("token revoked".into());
        }
        if expires_at > 0 {
            let now = Self::now();
            if now > expires_at {
                return Err("token expired".into());
            }
        }
        let allowed_ids: Vec<String> = serde_json::from_str(&config_ids_str)?;
        let allowed: Option<&[String]> = if allowed_ids.is_empty() {
            None
        } else {
            Some(&allowed_ids)
        };
        Ok(aggregator::aggregate_all(
            self.public_ip.as_deref(),
            allowed,
        ))
    }
}
