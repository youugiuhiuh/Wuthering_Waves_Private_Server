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

pub type Result<T> = std::result::Result<T, AppError>;