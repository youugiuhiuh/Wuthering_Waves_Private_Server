//! 一键三阶段编排：网络切换提示 + Google 连通性闸门 + 进度条 + 单文件状态。
//!
//! 流程：
//!
//! ```text
//! 检测当前网络 ──不可访问──> 提示切到【海外网络】，回车后复检
//!       │可访问
//!       ▼
//! 阶段一 resolve-map（海外）──> 提示切回【境内网络】，回车后复检必须不可访问
//!       │
//!       ▼
//! 阶段二 probe-survival（境内，零 DNS）
//!       │
//!       ▼
//! 提示切回【海外网络】，回车后复检必须可访问
//!       │
//!       ▼
//! 阶段三 build-pools（海外）
//! ```
//!
//! 断电/中断靠 `state.bin` 里已完成的阶段位掩码续跑。

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::connectivity::{self, Connectivity};
use crate::dns;
use crate::geo;
use crate::pipeline;
use crate::state::{Entry, PHASE_BUILD, PHASE_RESOLVE, PHASE_SURVIVAL, State};
use crate::target;

/// 编排参数。
pub struct RunOptions {
    pub input: PathBuf,
    pub state_file: PathBuf,
    pub out_dir: PathBuf,
    pub geo: PathBuf,
    pub workers: usize,
    pub timeout: Duration,
    pub max_lines: usize,
}

/// 读取标准输入的一行；EOF 视作放弃。
fn read_line() -> Option<String> {
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line),
        Err(_) => None,
    }
}

/// 提示用户并等待回车确认；返回 `false` 表示用户放弃（EOF）。
fn wait_enter(prompt: &str) -> bool {
    println!("{prompt}");
    print!("  按 Enter 继续（Ctrl+C 中止）… ");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    match read_line() {
        Some(_) => {
            println!();
            true
        }
        None => {
            println!();
            false
        }
    }
}

/// 网络闸门：要求当前网络**能**直连 Google。
async fn gate_expect_reachable(timeout: Duration) -> Result<(), String> {
    // shortcut: SNI_PROBE_SKIP_GATES 仅供自动化测试绕过网络切换提示
    if std::env::var("SNI_PROBE_SKIP_GATES").is_ok() {
        println!("  → 已跳过网络闸门（SNI_PROBE_SKIP_GATES）");
        return Ok(());
    }
    loop {
        let c = connectivity::check(timeout).await;
        println!("  → {}", c.describe());
        if c.is_reachable() {
            return Ok(());
        }
        if !wait_enter("  ⚠ 当前网络无法访问 Google。请切换到【海外网络】，然后按 Enter 复检：")
        {
            return Err("用户中止：未能切换到可访问 Google 的网络".into());
        }
    }
}

/// 网络闸门：要求当前网络**不能**直连 Google（即已回到境内）。
async fn gate_expect_blocked(timeout: Duration) -> Result<(), String> {
    // shortcut: SNI_PROBE_SKIP_GATES 仅供自动化测试绕过网络切换提示
    if std::env::var("SNI_PROBE_SKIP_GATES").is_ok() {
        println!("  → 已跳过网络闸门（SNI_PROBE_SKIP_GATES）");
        return Ok(());
    }
    loop {
        let c = connectivity::check(timeout).await;
        println!("  → {}", c.describe());
        match c {
            // 境外 DNS 解析失败也算"境内/不可用"，允许继续（境内常见）
            Connectivity::DnsFailed(_) => return Ok(()),
            Connectivity::HandshakeFailed { .. } => return Ok(()),
            Connectivity::Reachable { .. } => {
                if !wait_enter(
                    "  ⚠ 当前网络仍能直连 Google。请切换到【境内网络】，然后按 Enter 复检：",
                ) {
                    return Err("用户中止：未能切换到境内网络".into());
                }
            }
        }
    }
}

/// 执行一键三阶段流程。
pub async fn run(opts: RunOptions) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut state = State::load(&opts.state_file)?;

    println!(
        "=== sni_probe run：一键三阶段（状态文件 {}）===",
        opts.state_file.display()
    );
    if state.completed != 0 {
        println!(
            "  检测到已完成阶段位掩码 {:#b}，将跳过对应阶段续跑",
            state.completed
        );
    }

    // ---------- 阶段一：resolve-map（需海外网络） ----------
    if state.is_done(PHASE_RESOLVE) {
        println!("\n[阶段一] 已完成，跳过（{} 条记录）", state.phase1.len());
    } else {
        println!("\n[阶段一] 海外预解析 —— 需要【海外网络】");
        gate_expect_reachable(opts.timeout).await?;

        let lines = std::fs::read_to_string(&opts.input)?;
        let mut seeds: Vec<Entry> = lines
            .lines()
            .filter_map(target::parse_line)
            .map(|(domain, ip)| Entry {
                domain,
                ip: ip.and_then(|s| s.parse().ok()),
            })
            .collect();
        if opts.max_lines > 0 {
            seeds.truncate(opts.max_lines);
        }

        let resolver = Arc::new(dns::build_resolver(Duration::from_millis(2500))?);
        let (resolved, _stats) =
            pipeline::resolve_map_entries(seeds, resolver, opts.workers).await?;

        state.phase1 = resolved;
        state.mark_done(PHASE_RESOLVE);
        state.save(&opts.state_file)?;
    }

    // ---------- 阶段二：probe-survival（需境内网络、零 DNS） ----------
    if state.is_done(PHASE_SURVIVAL) {
        println!("\n[阶段二] 已完成，跳过（{} 条记录）", state.phase2.len());
    } else {
        println!("\n[阶段二] 境内存活探测 —— 需要【境内网络】，本阶段不发任何 DNS 查询");
        gate_expect_blocked(opts.timeout).await?;

        let (survived, _stats) =
            pipeline::probe_survival_entries(state.phase1.clone(), opts.workers, opts.timeout)
                .await?;

        state.phase2 = survived;
        state.mark_done(PHASE_SURVIVAL);
        state.save(&opts.state_file)?;
    }

    // ---------- 阶段三：build-pools（需海外网络） ----------
    if state.is_done(PHASE_BUILD) {
        println!("\n[阶段三] 已完成，跳过");
    } else {
        println!("\n[阶段三] 海外深度质检 —— 需要【海外网络】");
        gate_expect_reachable(opts.timeout).await?;

        let geodb = geo::GeoDb::open(&opts.geo).ok().map(Arc::new);
        if geodb.is_none() {
            eprintln!(
                "  warning: 未找到 {}，管辖国过滤将失效（国家码全部记作 UNKNOWN）",
                opts.geo.display()
            );
        }

        let accepted = pipeline::build_pools_entries(
            state.phase2.clone(),
            &opts.out_dir,
            geodb,
            opts.workers,
            opts.timeout,
        )
        .await?;

        state.phase3 = accepted;
        state.mark_done(PHASE_BUILD);
        state.save(&opts.state_file)?;
    }

    println!(
        "\n=== 全部完成 ===\n  阶段一预解析 {} 条\n  阶段二存活 {} 条\n  阶段三入库 {} 条\n  .pb 输出目录：{}\n  状态文件：{}",
        state.phase1.len(),
        state.phase2.len(),
        state.phase3.len(),
        opts.out_dir.display(),
        opts.state_file.display()
    );
    Ok(())
}

/// 供 `--help` 与文档引用的一句话流程说明。
pub const FLOW_HINT: &str = "阶段一(海外) → 切境内 → 阶段二(境内) → 切海外 → 阶段三(海外)";

/// 判定状态文件路径是否落在输出目录外（避免误把状态写进资源目录）。
pub fn state_outside_output(state: &Path, out_dir: &Path) -> bool {
    !state.starts_with(out_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_path_guard_rejects_inside_output_dir() {
        assert!(!state_outside_output(
            Path::new("rust/aegis/src/resources/sni/state.bin"),
            Path::new("rust/aegis/src/resources/sni")
        ));
        assert!(state_outside_output(
            Path::new("state.bin"),
            Path::new("rust/aegis/src/resources/sni")
        ));
    }

    #[test]
    fn flow_hint_mentions_all_three_phases() {
        assert!(FLOW_HINT.contains("阶段一"));
        assert!(FLOW_HINT.contains("阶段二"));
        assert!(FLOW_HINT.contains("阶段三"));
    }
}
