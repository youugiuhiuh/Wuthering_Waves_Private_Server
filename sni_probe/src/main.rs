//! sni_probe — sni_tester 三阶段流水线的 Rust 实现（热路径）。
//!
//! ```text
//! resolve-map    海外 VPS        raw 域名          -> domain,ip 映射表
//! probe-survival 审查境内/代理    domain,ip         -> 存活名单（零 DNS、零 HTTP）
//! build-pools    海外 VPS        domain,ip 存活    -> 双池 .pb
//! ```

mod cloudflare;
mod connectivity;
mod dns;
mod geo;
mod geodl;
mod h2;
mod pb;
mod pipeline;
mod progress;
mod run;
mod state;
mod target;
mod tls;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "sni_probe",
    about = "Three-phase SNI probe engine (Rust hot-path port)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 阶段一：海外纯 Anycast DNS 预解析，输出 domain,ip
    ResolveMap {
        #[arg(short = 'f', long = "file")]
        input: PathBuf,
        #[arg(short = 'o', long = "out", default_value = "resolved_targets.csv")]
        output: PathBuf,
        #[arg(short = 'w', long, default_value_t = 300)]
        workers: usize,
        #[arg(long = "max", default_value_t = 0)]
        max_lines: usize,
    },
    /// 阶段二：审查境内存活探测（零 DNS，需输入含 IP）
    ProbeSurvival {
        #[arg(short = 'f', long = "file")]
        input: PathBuf,
        #[arg(short = 'o', long = "out", default_value = "survived_candidates.csv")]
        output: PathBuf,
        #[arg(short = 'w', long, default_value_t = 100)]
        workers: usize,
        #[arg(long, default_value_t = 3)]
        timeout: u64,
        #[arg(short = 'p', long)]
        proxy: Option<String>,
        #[arg(long = "max", default_value_t = 0)]
        max_lines: usize,
    },
    /// 一键三阶段：网络切换提示 + Google 连通性闸门 + 进度条 + 单文件状态
    Run {
        /// 原始域名清单（Tranco CSV 或纯域名）
        #[arg(short = 'f', long = "file")]
        input: PathBuf,
        /// 阶段间状态文件（替代三段 CSV 传参，支持断点续跑）
        #[arg(long = "state", default_value = "sni_probe_state.bin")]
        state_file: PathBuf,
        #[arg(long = "out-dir", default_value = "rust/aegis/src/resources/sni")]
        out_dir: PathBuf,
        #[arg(long = "geo", default_value = "ipinfo_lite.mmdb")]
        geo: PathBuf,
        #[arg(short = 'w', long, default_value_t = 100)]
        workers: usize,
        /// 单目标 TLS 超时（秒）
        #[arg(long, default_value_t = 5)]
        timeout: u64,
        #[arg(long = "max", default_value_t = 0)]
        max_lines: usize,
    },
    /// 阶段三：海外深度质检，生成双池 .pb（与磁盘已有内容增量合并）
    BuildPools {
        #[arg(short = 'f', long = "file", default_value = "survived_candidates.csv")]
        input: PathBuf,
        #[arg(long = "out-dir", default_value = "rust/aegis/src/resources/sni")]
        out_dir: PathBuf,
        #[arg(long = "geo", default_value = "ipinfo_lite.mmdb")]
        geo: PathBuf,
        #[arg(short = 'w', long, default_value_t = 100)]
        workers: usize,
        #[arg(long, default_value_t = 5)]
        timeout: u64,
        #[arg(long = "max", default_value_t = 0)]
        max_lines: usize,
    },
}

fn report(stage: &str, stats: pipeline::StageStats, extra: &str) {
    println!(
        "[{stage}] total={} kept={} dropped={} elapsed={:.1}s{extra}",
        stats.total,
        stats.kept,
        stats.dropped,
        stats.elapsed.as_secs_f64()
    );
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        match cli.command {
            Command::ResolveMap {
                input,
                output,
                workers,
                max_lines,
            } => {
                let stats = pipeline::run_resolve_map(&input, &output, workers, max_lines).await?;
                report("resolve-map", stats, &format!(" -> {}", output.display()));
            }
            Command::ProbeSurvival {
                input,
                output,
                workers,
                timeout,
                proxy,
                max_lines,
            } => {
                let stats = pipeline::run_probe_survival(
                    &input,
                    &output,
                    workers,
                    max_lines,
                    Duration::from_secs(timeout),
                    proxy.as_deref(),
                )
                .await?;
                report(
                    "probe-survival",
                    stats,
                    &format!(" -> {}", output.display()),
                );
            }
            Command::Run {
                input,
                state_file,
                out_dir,
                geo,
                workers,
                timeout,
                max_lines,
            } => {
                if !run::state_outside_output(&state_file, &out_dir) {
                    return Err(format!(
                        "状态文件 {} 不能放在输出目录 {} 内",
                        state_file.display(),
                        out_dir.display()
                    )
                    .into());
                }
                println!("流程：{}", run::FLOW_HINT);
                run::run(run::RunOptions {
                    input,
                    state_file,
                    out_dir,
                    geo,
                    workers,
                    timeout: Duration::from_secs(timeout),
                    max_lines,
                })
                .await?;
            }
            Command::BuildPools {
                input,
                out_dir,
                geo,
                workers,
                timeout,
                max_lines,
            } => {
                geodl::ensure_mmdb(&geo, None, |msg| println!("{msg}"))
                    .await
                    .map_err(|e| format!("GeoDB 不可用：{e}"))?;
                let geodb =
                    Some(Arc::new(geo::GeoDb::open(&geo).map_err(|e| {
                        format!("GeoDB 打开失败 {}: {e}", geo.display())
                    })?));
                let stats = pipeline::run_build_pools(
                    &input,
                    &out_dir,
                    geodb,
                    workers,
                    max_lines,
                    Duration::from_secs(timeout),
                )
                .await?;
                report("build-pools", stats, &format!(" -> {}", out_dir.display()));
            }
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })
}
