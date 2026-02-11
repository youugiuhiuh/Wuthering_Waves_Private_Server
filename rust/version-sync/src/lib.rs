use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use toml::Value;

pub struct VersionSyncer {
    cargo_toml_path: String,
    install_sh_path: String,
}

impl VersionSyncer {
    pub fn new() -> Self {
        Self {
            cargo_toml_path: "rust/tgbot/Cargo.toml".to_string(),
            install_sh_path: "install.sh".to_string(),
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

    pub fn update_install_sh(&self, version: &str) -> Result<()> {
        let content = fs::read_to_string(&self.install_sh_path)
            .with_context(|| format!("Failed to read {}", self.install_sh_path))?;

        // 使用正则表达式精确匹配和替换版本号
        let version_regex = Regex::new(r#"\|\| echo "v[0-9]+\.[0-9]+\.[0-9]+""#)
            .with_context(|| "Failed to compile version regex")?;
        let display_regex = Regex::new(r#"echoContent green "当前版本：v[0-9]+\.[0-9]+\.[0-9]+""#)
            .with_context(|| "Failed to compile display regex")?;

        // 使用正则表达式的replace_all方法
        let step1 = version_regex.replace_all(&content, format!(r#"|| echo "v{}""#, version));
        let updated_content = display_regex.replace_all(
            &step1,
            format!(r#"echoContent green "当前版本：v{}""#, version),
        );

        fs::write(&self.install_sh_path, updated_content.as_ref())
            .with_context(|| format!("Failed to write {}", self.install_sh_path))?;

        Ok(())
    }

    pub fn sync_cargo_lock(&self) -> Result<()> {
        let original_dir =
            std::env::current_dir().with_context(|| "Failed to get current directory")?;

        // 切换到 rust/tgbot 目录
        std::env::set_current_dir("rust/tgbot")
            .with_context(|| "Failed to change to rust/tgbot directory")?;

        let output = std::process::Command::new("cargo")
            .args(&["check"])
            .output()
            .with_context(|| "Failed to run cargo check")?;

        // 恢复原始目录
        std::env::set_current_dir(original_dir)
            .with_context(|| "Failed to restore original directory")?;

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

        // 检查并更新 install.sh
        let old_install_content = fs::read_to_string(&self.install_sh_path).unwrap_or_default();
        if self.update_install_sh(&version).is_ok() {
            let new_install_content = fs::read_to_string(&self.install_sh_path).unwrap_or_default();
            if old_install_content != new_install_content {
                modified_files.push(self.install_sh_path.clone());
            }
        }

        // 同步 Cargo.lock
        let old_lock_content = fs::read_to_string("rust/tgbot/Cargo.lock").unwrap_or_default();
        if self.sync_cargo_lock().is_ok() {
            let new_lock_content = fs::read_to_string("rust/tgbot/Cargo.lock").unwrap_or_default();
            if old_lock_content != new_lock_content {
                modified_files.push("rust/tgbot/Cargo.lock".to_string());
            }
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
