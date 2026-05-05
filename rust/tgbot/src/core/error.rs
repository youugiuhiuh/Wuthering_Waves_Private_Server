//! 统一错误类型定义

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("服务错误: {0}")]
    Service(String),

    #[error("网络错误: {0}")]
    Network(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 解析错误: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0} 未安装")]
    NotInstalled(String),

    #[error("端口 {0} 不可用")]
    PortUnavailable(u16),

    #[error("无效参数: {0}")]
    InvalidParameter(String),

    #[error("操作超时")]
    Timeout,

    #[error("未知错误: {0}")]
    Unknown(String),
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Unknown(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use serde_json;

    #[test]
    fn test_app_error_config() {
        let err = AppError::Config("invalid setting".to_string());
        assert_eq!(err.to_string(), "配置错误: invalid setting");
    }

    #[test]
    fn test_app_error_service() {
        let err = AppError::Service("service unavailable".to_string());
        assert_eq!(err.to_string(), "服务错误: service unavailable");
    }

    #[test]
    fn test_app_error_network() {
        let err = AppError::Network("connection failed".to_string());
        assert_eq!(err.to_string(), "网络错误: connection failed");
    }

    #[test]
    fn test_app_error_not_installed() {
        let err = AppError::NotInstalled("nginx".to_string());
        assert_eq!(err.to_string(), "nginx 未安装");
    }

    #[test]
    fn test_app_error_port_unavailable() {
        let err = AppError::PortUnavailable(8080);
        assert_eq!(err.to_string(), "端口 8080 不可用");
    }

    #[test]
    fn test_app_error_invalid_parameter() {
        let err = AppError::InvalidParameter("empty value".to_string());
        assert_eq!(err.to_string(), "无效参数: empty value");
    }

    #[test]
    fn test_app_error_timeout() {
        let err = AppError::Timeout;
        assert_eq!(err.to_string(), "操作超时");
    }

    #[test]
    fn test_app_error_unknown() {
        let err = AppError::Unknown("something went wrong".to_string());
        assert_eq!(err.to_string(), "未知错误: something went wrong");
    }

    #[test]
    fn test_app_error_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: AppError = io_err.into();
        assert_eq!(err.to_string(), "IO 错误: file not found");
    }

    #[test]
    fn test_app_error_from_json_error() {
        let json_err = serde_json::from_str::<u32>("invalid").unwrap_err();
        let err: AppError = json_err.into();
        assert!(err.to_string().starts_with("JSON 解析错误:"));
    }

    #[test]
    fn test_result_type_alias() {
        fn return_result() -> Result<u32> {
            Ok(42)
        }
        fn return_error() -> Result<u32> {
            Err(AppError::Config("test".to_string()))
        }
        assert_eq!(return_result().unwrap(), 42);
        assert!(return_error().is_err());
    }
}