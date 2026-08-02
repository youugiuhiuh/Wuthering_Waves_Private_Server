use std::str::FromStr;

use anyhow::Result;
use async_trait::async_trait;

use crate::core::i18n;
use crate::core::i18n::Lang;
use crate::core::system::operations::Operations;
use crate::core::system::scheduler::{ScheduledTask, TaskType, get_manager};

#[async_trait]
pub trait HostSettings: Send + Sync {
    async fn apply_locale(&self, locale: &str) -> Result<()>;
}

pub struct SystemHostSettings;

#[async_trait]
impl HostSettings for SystemHostSettings {
    async fn apply_locale(&self, locale: &str) -> Result<()> {
        let lang =
            Lang::from_str(locale).map_err(|_| anyhow::anyhow!("unknown locale: {locale}"))?;
        let tz = i18n::lang_to_timezone(lang);

        match tokio::process::Command::new("timedatectl")
            .args(["set-timezone", tz])
            .output()
            .await
        {
            Ok(o) if !o.status.success() => {
                log::warn!("设置系统时区 {} 失败: exit {:?}", tz, o.status.code());
            }
            Err(e) => log::warn!("设置系统时区 {} 失败: {}", tz, e),
            _ => {}
        }

        if let Err(e) = Operations::set_apt_daily_timer().await {
            log::warn!("覆盖 apt-daily timer 失败: {}", e);
        }

        if let Err(e) =
            Operations::perform_maintenance_with_reboot_time(Operations::DEFAULT_REBOOT_TIME).await
        {
            log::warn!("安全更新初始化失败: {}", e);
        }

        if let Some(manager) = get_manager().await {
            let geo_task = ScheduledTask::new_with_timezone(TaskType::GeoUpdate, "0 1 * * 1", tz);
            let _ = manager.add_new_task(geo_task).await;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn system_host_settings_rejects_unknown_locale() {
        let host = SystemHostSettings;
        assert!(host.apply_locale("xx").await.is_err());
    }
}
