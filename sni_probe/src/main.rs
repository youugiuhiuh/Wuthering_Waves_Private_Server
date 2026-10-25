//! sni_probe — sni_tester 三阶段流水线的 Rust 实现（热路径）。
//!
//! ```text
//! resolve-map    海外 VPS        raw 域名          -> domain,ip 映射表
//! probe-survival 审查境内/代理    domain,ip         -> 存活名单（零 DNS、零 HTTP）
//! build-pools    海外 VPS        domain,ip 存活    -> 双池 .pb
//! ```

mod bind;
mod cloudflare;
mod connectivity;
mod dns;
mod geo;
mod geodl;
mod h2;
mod logging;
mod merge;
mod pb;
mod pipeline;
mod progress;
mod resource;
mod run;
mod shutdown;
mod state;
mod storage;
mod target;
mod tls;
mod upgrade;

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
    /// 打印逐域名明细日志
    #[arg(long, global = true)]
    debug: bool,
    /// 全部阶段完成后关机
    #[arg(long, global = true)]
    shutdown: bool,
    /// 历史库路径（记录成功/失败/ASN 黑名单）
    #[arg(long, global = true, default_value = "sni_probe_history.redb")]
    history: PathBuf,
    /// 失败记录保留天数（对应 Go -ttl）
    #[arg(long = "ttl", global = true, default_value_t = 7)]
    ttl_days: u32,
    /// 无视历史记录重新测试（对应 Go -force）
    #[arg(long, global = true)]
    force: bool,
    /// 清空全部历史后退出（对应 Go -reset）
    #[arg(long, global = true)]
    reset: bool,
    /// 跳过启动时的并发安全自检（不推荐）
    #[arg(long = "no-guard", global = true)]
    no_guard: bool,
    /// 出站绑定到网卡（如 wlp6s0）或本地 IP，隔离测试流量与主宽带
    #[arg(long = "iface", global = true)]
    iface: Option<String>,
    /// 列出本机所有网卡名后退出
    #[arg(long = "list-ifaces", global = true)]
    list_ifaces: bool,
    #[command(subcommand)]
    command: Option<Command>,
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
    /// 升级目录下所有存量 .pb 为双池格式（对应 Go -upgrade-dir）
    UpgradeDir {
        /// 源目录
        #[arg(short = 'd', long = "dir")]
        dir: PathBuf,
        /// 输出目录（缺省原地覆盖）
        #[arg(long = "out-dir")]
        out_dir: Option<PathBuf>,
        /// 源文件是 v1.9.9 单池格式（domains=1），全部按 classic 处理
        #[arg(long = "legacy")]
        legacy: bool,
        /// 把失败域名保留到 classic 池（对应 Go -keep-dead）
        #[arg(long = "keep-dead")]
        keep_dead: bool,
        #[arg(long = "geo", default_value = "ipinfo_lite.mmdb")]
        geo: PathBuf,
        #[arg(short = 'w', long, default_value_t = 30)]
        workers: usize,
        #[arg(long, default_value_t = 5)]
        timeout: u64,
    },
    /// 升级单个 .pb 为双池格式（对应 Go -upgrade-pb）
    UpgradePb {
        #[arg(short = 'f', long = "file")]
        file: PathBuf,
        #[arg(long = "out")]
        out: Option<PathBuf>,
        #[arg(long = "legacy")]
        legacy: bool,
        #[arg(long = "keep-dead")]
        keep_dead: bool,
        #[arg(long = "geo", default_value = "ipinfo_lite.mmdb")]
        geo: PathBuf,
        #[arg(short = 'w', long, default_value_t = 30)]
        workers: usize,
        #[arg(long, default_value_t = 5)]
        timeout: u64,
    },
    /// 把目录下所有 .pb 合并成一个去重总池（跨文件去重）
    Merge {
        /// 源目录
        #[arg(short = 'd', long = "dir")]
        dir: PathBuf,
        /// 输出文件
        #[arg(short = 'o', long = "out")]
        out: PathBuf,
        /// 源文件是 v1.9.9 单池格式
        #[arg(long = "legacy")]
        legacy: bool,
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
    logging::set_debug(cli.debug);

    if cli.list_ifaces {
        for name in bind::list_interfaces() {
            match bind::iface_ipv4(&name) {
                Some(ip) => println!("{name:20} {ip}"),
                None => println!("{name:20} (无 IPv4)"),
            }
        }
        return Ok(());
    }

    // -iface：设置出站绑定
    if let Some(target) = cli.iface.as_deref()
        && let Err(e) = bind::set_outbound(target)
    {
        return Err(format!("--iface {target} 设置失败: {e}").into());
    }
    if cli.iface.is_some()
        && let Some(w) = bind::validate_binding()
    {
        eprintln!("{w}");
    }

    // -reset：清空历史后立即退出
    if cli.reset {
        match storage::Storage::open(&cli.history, cli.ttl_days) {
            Ok(s) => {
                s.clear_all()?;
                println!("已清空历史库 {}", cli.history.display());
            }
            Err(e) => eprintln!("打开历史库失败（可能本就不存在）: {e}"),
        }
        return Ok(());
    }

    // 打开历史库（供各子命令复用）
    let history = match storage::Storage::open(&cli.history, cli.ttl_days) {
        Ok(s) => Some(std::sync::Arc::new(s)),
        Err(e) => {
            eprintln!("warning: 历史库不可用（{e}），本次不记录成功/失败历史");
            None
        }
    };
    let force = cli.force;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = rt.block_on(async {
        let Some(cmd) = cli.command else {
            eprintln!("缺少子命令，使用 --help 查看用法");
            return Ok(());
        };

        // 启动自检：-w 超限会导致假失败（并污染历史库）
        if !cli.no_guard {
            let (requested, stage) = command_workers(&cmd);
            let budget = resource::budget();
            if let Some(w) = resource::warn_text(&budget, requested, stage) {
                eprintln!("{w}");
            }
        }

        match cmd {
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
            Command::UpgradeDir {
                dir,
                out_dir,
                legacy,
                keep_dead,
                geo,
                workers,
                timeout,
            } => {
                geodl::ensure_mmdb(&geo, None, |m| println!("{m}"))
                    .await
                    .map_err(|e| format!("GeoDB 不可用：{e}"))?;
                let geodb = Some(Arc::new(geo::GeoDb::open(&geo)?));
                let format = if legacy {
                    upgrade::PbFormat::LegacySinglePool
                } else {
                    upgrade::PbFormat::DualPool
                };
                let stats = upgrade::upgrade_pb_dir(
                    &dir,
                    out_dir.as_deref(),
                    geodb,
                    upgrade::UpgradeOptions {
                        workers,
                        timeout: Duration::from_secs(timeout),
                        keep_dead,
                        storage: history.clone(),
                        force,
                        format,
                    },
                )
                .await?;
                let total: usize = stats.iter().map(|s| s.total).sum();
                let pq: usize = stats.iter().map(|s| s.pq_count).sum();
                let classic: usize = stats.iter().map(|s| s.classic_count).sum();
                println!(
                    "\n[upgrade-dir] {} 个文件 | 域名 {} | PQ {} | classic {}",
                    stats.len(),
                    total,
                    pq,
                    classic
                );
            }
            Command::UpgradePb {
                file,
                out,
                legacy,
                keep_dead,
                geo,
                workers,
                timeout,
            } => {
                geodl::ensure_mmdb(&geo, None, |m| println!("{m}"))
                    .await
                    .map_err(|e| format!("GeoDB 不可用：{e}"))?;
                let geodb = Some(Arc::new(geo::GeoDb::open(&geo)?));
                let format = if legacy {
                    upgrade::PbFormat::LegacySinglePool
                } else {
                    upgrade::PbFormat::DualPool
                };
                upgrade::upgrade_pb_file(
                    &file,
                    out.as_deref(),
                    geodb,
                    upgrade::UpgradeOptions {
                        workers,
                        timeout: Duration::from_secs(timeout),
                        keep_dead,
                        storage: history.clone(),
                        force,
                        format,
                    },
                )
                .await?;
            }
            Command::Merge { dir, out, legacy } => {
                let format = if legacy {
                    upgrade::PbFormat::LegacySinglePool
                } else {
                    upgrade::PbFormat::DualPool
                };
                let (npq, nclassic) = merge::merge_dir_into(&dir, &out, format)?;
                println!(
                    "[merge] {} -> {} | PQ={} classic={}",
                    dir.display(),
                    out.display(),
                    npq,
                    nclassic
                );
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
                let blocklist = history
                    .as_ref()
                    .map(|h| Arc::new(storage::AsnBlocklist::new(h, 3)));
                let stats = pipeline::run_build_pools_with_history(
                    &input,
                    &out_dir,
                    geodb,
                    workers,
                    max_lines,
                    Duration::from_secs(timeout),
                    history.clone(),
                    blocklist,
                    force,
                )
                .await?;
                report("build-pools", stats, &format!(" -> {}", out_dir.display()));
            }
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });

    if result.is_ok()
        && cli.shutdown
        && let Err(e) = shutdown::request_shutdown(false)
    {
        eprintln!("[shutdown] 关机失败: {e}");
    }
    result
}

/// 取出各子命令请求的并发数，用于启动自检。
fn command_workers(cmd: &Command) -> (u64, &'static str) {
    match cmd {
        Command::ResolveMap { workers, .. } => (*workers as u64, "resolve-map"),
        Command::ProbeSurvival { workers, .. } => (*workers as u64, "probe-survival"),
        Command::Run { workers, .. } => (*workers as u64, "run"),
        Command::BuildPools { workers, .. } => (*workers as u64, "build-pools"),
        Command::UpgradeDir { workers, .. } => (*workers as u64, "upgrade-dir"),
        Command::UpgradePb { workers, .. } => (*workers as u64, "upgrade-pb"),
        Command::Merge { .. } => (1, "merge"),
    }
}
