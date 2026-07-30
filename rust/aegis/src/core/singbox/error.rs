use std::fmt;

#[derive(Debug)]
pub enum SingBoxError {
    NotInstalled,
    ConfigGenerationFailed(String),
    NoAvailablePort,
    ServiceReloadFailed(String),
    DownloadFailed(String),
    UnsupportedArch(String),
}

impl fmt::Display for SingBoxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SingBoxError::NotInstalled => write!(f, "Sing-box 未安装"),
            SingBoxError::ConfigGenerationFailed(msg) => write!(f, "配置生成失败: {}", msg),
            SingBoxError::NoAvailablePort => write!(f, "端口分配失败: 无可用端口"),
            SingBoxError::ServiceReloadFailed(msg) => write!(f, "服务重载失败: {}", msg),
            SingBoxError::DownloadFailed(msg) => write!(f, "下载失败: {}", msg),
            SingBoxError::UnsupportedArch(arch) => write!(f, "不支持的架构: {}", arch),
        }
    }
}

impl std::error::Error for SingBoxError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_singbox_error_not_installed() {
        let err = SingBoxError::NotInstalled;
        assert_eq!(err.to_string(), "Sing-box 未安装");
    }

    #[test]
    fn test_singbox_error_config_generation_failed() {
        let err = SingBoxError::ConfigGenerationFailed("invalid json".to_string());
        assert_eq!(err.to_string(), "配置生成失败: invalid json");
    }

    #[test]
    fn test_singbox_error_no_available_port() {
        let err = SingBoxError::NoAvailablePort;
        assert_eq!(err.to_string(), "端口分配失败: 无可用端口");
    }

    #[test]
    fn test_singbox_error_service_reload_failed() {
        let err = SingBoxError::ServiceReloadFailed("systemctl failed".to_string());
        assert_eq!(err.to_string(), "服务重载失败: systemctl failed");
    }

    #[test]
    fn test_singbox_error_download_failed() {
        let err = SingBoxError::DownloadFailed("connection timeout".to_string());
        assert_eq!(err.to_string(), "下载失败: connection timeout");
    }

    #[test]
    fn test_singbox_error_unsupported_arch() {
        let err = SingBoxError::UnsupportedArch("riscv64".to_string());
        assert_eq!(err.to_string(), "不支持的架构: riscv64");
    }
}
