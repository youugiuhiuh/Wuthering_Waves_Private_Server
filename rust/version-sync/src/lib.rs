use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use toml::Value;

pub struct VersionSyncer {
    cargo_toml_path: String,
    go_installer_path: String,
}

impl VersionSyncer {
    pub fn new() -> Self {
        Self {
            cargo_toml_path: "rust/tgbot/Cargo.toml".to_string(),
            go_installer_path: "go/installer/main.go".to_string(),
        }
    }

    pub fn extract_version(&self) -> Result<String> {
        let content = fs::read_to_string(&self.cargo_toml_path)
            .with_context(|| format!("Failed to read {}", self.cargo_toml_path))?;

        let toml_value: Value =
            toml::from_str(&content).with_context(|| "Failed to parse Cargo.toml")?;

        let version = toml_value
            .get("package")
            .and_then(|p| p.get("version"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Version not found in Cargo.toml"))?;

        Ok(version.to_string())
    }

    pub fn update_go_installer(&self, version: &str) -> Result<()> {
        let content = fs::read_to_string(&self.go_installer_path)
            .with_context(|| format!("Failed to read {}", self.go_installer_path))?;

        // 匹配 Go 常量: version = "vX.Y.Z"
        let version_regex = Regex::new(r#"version\s*=\s*"v[0-9]+\.[0-9]+\.[0-9]+""#)
            .with_context(|| "Failed to compile Go version regex")?;

        let updated_content =
            version_regex.replace_all(&content, format!(r#"version     = "v{}""#, version));

        fs::write(&self.go_installer_path, updated_content.as_ref())
            .with_context(|| format!("Failed to write {}", self.go_installer_path))?;

        Ok(())
    }

    pub fn sync_cargo_lock(&self) -> Result<()> {
        let original_dir =
            std::env::current_dir().with_context(|| "Failed to get current directory")?;

        // 切换到 rust/tgbot 目录
        std::env::set_current_dir("rust/tgbot")
            .with_context(|| "Failed to change to rust/tgbot directory")?;

        let cargo_result = std::process::Command::new("cargo")
            .args(["check"])
            .output()
            .with_context(|| "Failed to run cargo check");

        std::env::set_current_dir(&original_dir)
            .with_context(|| "Failed to restore original directory")?;

        let output = cargo_result?;

        if !output.status.success() {
            anyhow::bail!(
                "Cargo check failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    pub fn sync_all(&self) -> Result<SyncResult> {
        let version = self.extract_version()?;

        let mut modified_files = Vec::new();

        // 检查并更新 go/installer/main.go
        let old_content = fs::read_to_string(&self.go_installer_path).unwrap_or_default();
        self.update_go_installer(&version)?;
        let new_content = fs::read_to_string(&self.go_installer_path).unwrap_or_default();
        if old_content != new_content {
            modified_files.push(self.go_installer_path.clone());
        }

        // 同步 Cargo.lock
        let old_lock_content = fs::read_to_string("rust/tgbot/Cargo.lock").unwrap_or_default();
        self.sync_cargo_lock()?;
        let new_lock_content = fs::read_to_string("rust/tgbot/Cargo.lock").unwrap_or_default();
        if old_lock_content != new_lock_content {
            modified_files.push("rust/tgbot/Cargo.lock".to_string());
        }

        Ok(SyncResult {
            version,
            modified_files,
        })
    }
}

pub struct SyncResult {
    pub version: String,
    pub modified_files: Vec<String>,
}
