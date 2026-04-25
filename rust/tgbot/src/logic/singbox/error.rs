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