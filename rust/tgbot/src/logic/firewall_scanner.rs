use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tokio::fs;

static PORT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#""(?:port|listen_port)"\s*:\s*(\d+)"#).unwrap());
static LISTEN_ADDR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#""listen"\s*:\s*"(?:127\.0\.0\.1|localhost)""#).unwrap());

pub struct FirewallScanner;

impl FirewallScanner {
    /// 综合扫描所有需要放行的端口
    pub async fn scan_all_ports() -> Result<HashSet<u16>> {
        let mut ports = HashSet::new();

        // 1. SSH 端口
        if let Ok(ssh_port) = Self::detect_ssh_port().await {
            ports.insert(ssh_port);
        } else {
            ports.insert(22); // 兜底
        }

        // 2. Web 端口 (80/443) 如果 Nginx 存在
        if Self::is_web_service_active().await {
            ports.insert(80);
            ports.insert(443);
        }

        // 3. wwps-core 端口
        let wwps_core_dirs = vec!["/etc/wwps/wwps-core/conf"];
        for dir in wwps_core_dirs {
            if let Ok(p) = Self::scan_dir_for_ports(dir).await {
                ports.extend(p);
            }
        }

        // 4. wwps-box 端口
        let sb_dirs = vec!["/etc/wwps/wwps-box/conf/config"];
        for dir in sb_dirs {
            if let Ok(p) = Self::scan_dir_for_ports(dir).await {
                ports.extend(p);
            }
        }

        // 5. 系统级活跃监听端口 (综合扫描查漏补缺)
        if let Ok(active_ports) = Self::scan_active_listening_ports().await {
            ports.extend(active_ports);
        }

        Ok(ports)
    }

    /// 扫描系统当前正在监听的公网端口 (优先使用 ss，兜底使用 netstat)
    pub async fn scan_active_listening_ports() -> Result<HashSet<u16>> {
        let ports = HashSet::new();

        // 1. 尝试使用 ss (现代系统首选)
        match Self::scan_with_ss().await {
            Ok(p) => {
                if !p.is_empty() {
                    return Ok(p);
                }
            }
            Err(_) => { /* 继续尝试 netstat */ }
        }

        // 2. 尝试使用 netstat (旧系统或极简环境)
        match Self::scan_with_netstat().await {
            Ok(p) => Ok(p),
            Err(e) => {
                // 如果两个都失败了，至少返回空集合而不是报错
                eprintln!("警告: ss 和 netstat 端口扫描均失败: {}", e);
                Ok(ports)
            }
        }
    }

    async fn scan_with_ss() -> Result<HashSet<u16>> {
        let mut ports = HashSet::new();
        for proto_flag in &["-t", "-u"] {
            let output = tokio::process::Command::new("ss")
                .args(&["-H", proto_flag, "-nl"])
                .output()
                .await?;

            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 4 {
                        let local_addr = parts[parts.len() - 2]; // 倒数第二列通常是 Local Address
                        if !local_addr.contains("127.0.0.1") && !local_addr.contains("[::1]") {
                            if let Some(port_str) = local_addr.split(':').last() {
                                if let Ok(port) = port_str.parse::<u16>() {
                                    ports.insert(port);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(ports)
    }

    async fn scan_with_netstat() -> Result<HashSet<u16>> {
        let mut ports = HashSet::new();
        let output = tokio::process::Command::new("netstat")
            .args(&["-tunl"])
            .output()
            .await?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                // netstat 输出示例:
                // tcp        0      0 0.0.0.0:22              0.0.0.0:*               LISTEN
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 && (line.starts_with("tcp") || line.starts_with("udp")) {
                    let local_addr = parts[3];
                    if !local_addr.contains("127.0.0.1") && !local_addr.contains("::1") {
                        if let Some(port_str) = local_addr.split(':').last() {
                            if let Ok(port) = port_str.parse::<u16>() {
                                ports.insert(port);
                            }
                        }
                    }
                }
            }
        }
        Ok(ports)
    }

    /// 检测 SSH 端口
    pub async fn detect_ssh_port() -> Result<u16> {
        let config = fs::read_to_string("/etc/ssh/sshd_config")
            .await
            .context("读取 sshd_config 失败")?;
        for line in config.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if line.to_lowercase().starts_with("port ") {
                if let Some(port_str) = line.split_whitespace().nth(1) {
                    if let Ok(port) = port_str.parse::<u16>() {
                        return Ok(port);
                    }
                }
            }
        }
        Ok(22)
    }

    /// 扫描目录下的所有 .json 文件提取端口
    async fn scan_dir_for_ports<P: AsRef<Path>>(dir: P) -> Result<HashSet<u16>> {
        let mut ports = HashSet::new();
        let dir = dir.as_ref();
        if !fs::try_exists(dir).await.unwrap_or(false) {
            return Ok(ports);
        }

        let mut entries = fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                if let Ok(file_ports) = Self::extract_ports_from_file(&path).await {
                    ports.extend(file_ports);
                }
            }
        }
        Ok(ports)
    }

    /// 从单个 JSON 文件中提取端口 (排除 127.0.0.1 监听)
    async fn extract_ports_from_file(path: &PathBuf) -> Result<HashSet<u16>> {
        let content = fs::read_to_string(path).await?;
        let mut ports = HashSet::new();

        // 改进后的逻辑：扫描文件中的每个入站配置块（粗略定位）
        // 寻找 "port": 1234 模式，并检查其附近的 "listen" 字段

        // 我们按行处理，但维护一个简单的状态：是否在 127.0.0.1 的上下文中
        // wwps-core 的入站通常是 { "listen": "...", "port": ... }
        let mut is_local_context = false;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // 如果看到新的对象开始，重置 context (这只是简单近似)
            if line.contains('{') {
                is_local_context = false;
            }

            // 检测监听地址
            if LISTEN_ADDR_RE.is_match(line) {
                // 如果匹配到 127.0.0.1 或 localhost，则标记当前上下文为 local
                is_local_context = true;
            } else if line.contains("\"listen\"")
                && (line.contains("\"::\"") || line.contains("\"0.0.0.0\""))
            {
                // 如果明确是公网地址，重置 local 标记
                is_local_context = false;
            }

            // 提取端口
            if let Some(caps) = PORT_RE.captures(line) {
                if !is_local_context {
                    if let Some(port_match) = caps.get(1) {
                        if let Ok(port) = port_match.as_str().parse::<u16>() {
                            ports.insert(port);
                        }
                    }
                }
            }

            // 如果看到对象结束，也可以重置 (简单近似)
            if line.contains('}') {
                is_local_context = false;
            }
        }

        Ok(ports)
    }

    /// 检测是否存在 Web 服务器 (Nginx)
    async fn is_web_service_active() -> bool {
        // 简单通过文件路径判断或 pgrep (这里用路径，更通用)
        fs::try_exists("/etc/nginx").await.unwrap_or(false)
            || fs::try_exists("/etc/apache2").await.unwrap_or(false)
    }
}
