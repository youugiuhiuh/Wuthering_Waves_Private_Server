//! Imperative I/O Operations Examples / 指令式 I/O 操作示例
//!
//! This file demonstrates correct and incorrect usage of imperative paradigm
//! in rust/tgbot codebase.

use anyhow::Result;

// ============================================================================
// CORRECT EXAMPLES / 正确示例
// ============================================================================

/// ✅ Imperative: 顺序 I/O 操作，带进度回调
/// 场景: 安装 BBR3 并报告进度
pub async fn install_bbr3_with_progress<F>(callback: F) -> Result<()>
where
    F: Fn(usize, &str),
{
    // 步骤 1: 修复主机名解析
    callback(1, "🔧 修复主机名解析...");
    fix_local_hostname_resolution().await?;

    // 步骤 2: 检查并安装依赖
    callback(2, "📦 检查并安装依赖...");
    ensure_bbr3_dependencies().await?;

    // 步骤 3: 配置内核参数
    callback(3, "⚙️ 配置内核参数...");
    apply_sysctl_params().await?;

    // 步骤 4: 加载 BBR 模块
    callback(4, "🔄 加载 BBR 模块...");
    load_bbr_module().await?;

    // 步骤 5: 验证安装
    callback(5, "✅ 验证安装...");
    verify_bbr3_installation().await?;

    Ok(())
}

/// ✅ Imperative: 证书解析 - 状态机更适合指令式
/// 场景: 从 sing-box 输出中解析 TLS 证书和私钥
async fn parse_tls_certificates(output: &str) -> Result<(String, String)> {
    let lines: Vec<&str> = output.lines().collect();

    let mut key_content = String::new();
    let mut cert_content = String::new();
    let mut in_key = false;
    let mut in_cert = false;

    for line in lines {
        if line.contains("BEGIN PRIVATE KEY") {
            in_key = true;
            key_content.push_str(line);
            key_content.push('\n');
            continue;
        }
        if line.contains("END PRIVATE KEY") {
            key_content.push_str(line);
            key_content.push('\n');
            in_key = false;
            continue;
        }
        if line.contains("BEGIN CERTIFICATE") {
            in_cert = true;
            cert_content.push_str(line);
            cert_content.push('\n');
            continue;
        }
        if line.contains("END CERTIFICATE") {
            cert_content.push_str(line);
            cert_content.push('\n');
            in_cert = false;
            continue;
        }

        // 状态累积
        if in_key {
            key_content.push_str(line);
            key_content.push('\n');
        }
        if in_cert {
            cert_content.push_str(line);
            cert_content.push('\n');
        }
    }

    Ok((key_content, cert_content))
}

/// ✅ Imperative: 顺序步骤 - 清晰表达执行流程
/// 场景: 创建 Hysteria2 配置
pub async fn create_hysteria2_config(
    count: usize,
    enable_obfs: bool,
) -> Result<Vec<String>> {
    let mut links = Vec::new();
    let mut configs = Vec::new();

    for i in 0..count {
        // 分配端口 (I/O)
        let (main_port, hop_range) = allocate_port().await?;

        // 生成配置 (纯转换)
        let config = generate_config(main_port, enable_obfs);

        // 生成链接 (纯转换)
        let link = generate_link(&config, hop_range);

        links.push(link);
        configs.push(config);

        // 防火墙规则 (I/O)
        setup_firewall_rules(main_port, hop_range).await?;
    }

    // 保存配置 (I/O)
    save_config(&configs).await?;

    // 重载服务 (I/O)
    reload_service().await?;

    Ok(links)
}

/// ✅ Imperative: 错误处理使用 ? 操作符
/// 场景: 顺序执行多个可能失败的操作
async fn setup_service() -> Result<()> {
    // 每一步都清晰表达可能的失败
    check_prerequisites().await?;      // 失败则返回
    initialize_directories().await?;   // 失败则返回
    copy_binaries().await?;            // 失败则返回
    configure_service().await?;         // 失败则返回
    start_service().await?;            // 失败则返回

    Ok(())
}

// ============================================================================
// ANTI-PATTERNS / 反模式
// ============================================================================

/// ❌ Declarative: 不要把顺序 I/O 改成函数式链
/// 场景: 服务设置
async fn setup_service_bad() -> Result<()> {
    // ❌ 这掩盖了顺序依赖关系
    let steps = vec![
        check_prerequisites,
        initialize_directories,
        copy_binaries,
        configure_service,
        start_service,
    ];

    for step in steps {
        step().await?;  // 错误处理不清晰
    }

    Ok(())
}

/// ❌ Declarative: 不要用迭代器处理有顺序依赖的 I/O
async fn setup_service_worse() -> Result<()> {
    // ❌ 完全不合适
    let results: Vec<Result<()>> = futures::stream::iter([
        check_prerequisites(),
        initialize_directories(),
        copy_binaries(),
        configure_service(),
        start_service(),
    ])
    .buffer_unordered(1)  // 强制顺序
    .collect()
    .await;

    // 错误处理复杂
    for result in results {
        result?;
    }

    Ok(())
}

/// ❌ Mixed: 混用不同范式导致代码难以理解
async fn mixed_approach() -> Result<()> {
    let mut data = load_data().await?;

    // ❌ 一会儿函数式
    let processed: Vec<_> = data
        .iter()
        .map(|x| transform(x))
        .collect();

    // ❌ 一会儿指令式
    for item in &processed {
        save_item(item).await?;
    }

    Ok(())
}

// ============================================================================
// HELPER FUNCTIONS / 辅助函数
// ============================================================================

async fn fix_local_hostname_resolution() -> Result<()> {
    Ok(())
}

async fn ensure_bbr3_dependencies() -> Result<()> {
    Ok(())
}

async fn apply_sysctl_params() -> Result<()> {
    Ok(())
}

async fn load_bbr_module() -> Result<()> {
    Ok(())
}

async fn verify_bbr3_installation() -> Result<()> {
    Ok(())
}

async fn allocate_port() -> Result<(u16, (u16, u16))> {
    Ok((10000, (10001, 10099)))
}

fn generate_config(port: u16, enable_obfs: bool) -> Config {
    Config { port, enable_obfs }
}

fn generate_link(config: &Config, hop_range: (u16, u16)) -> String {
    format!("hysteria2://password@host:{},{}-{}?sni=example.com#test",
        config.port, hop_range.0, hop_range.1)
}

async fn setup_firewall_rules(main_port: u16, hop_range: (u16, u16)) -> Result<()> {
    Ok(())
}

async fn save_config(configs: &[Config]) -> Result<()> {
    Ok(())
}

async fn reload_service() -> Result<()> {
    Ok(())
}

async fn check_prerequisites() -> Result<()> {
    Ok(())
}

async fn initialize_directories() -> Result<()> {
    Ok(())
}

async fn copy_binaries() -> Result<()> {
    Ok(())
}

async fn configure_service() -> Result<()> {
    Ok(())
}

async fn start_service() -> Result<()> {
    Ok(())
}

fn transform(x: &str) -> String {
    x.to_string()
}

async fn load_data() -> Result<Vec<String>> {
    Ok(vec![])
}

async fn save_item(item: &str) -> Result<()> {
    Ok(())
}

#[derive(Debug)]
struct Config {
    port: u16,
    enable_obfs: bool,
}