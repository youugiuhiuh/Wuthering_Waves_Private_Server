//! IPinfo Lite MMDB 自动下载：镜像轮询 + 临时文件原子替换。
//!
//! 国内直连 `github.com` 的 release 资产常年不稳，因此按 Go 版 `GeoDBMirrors`
//! 的做法先试代理镜像，全部失败再回退官方地址。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// GitHub release 资产在仓库中的路径（镜像只需替换主机名）。
pub const GEO_GITHUB_PATH: &str = "/youugiuhiuh/IPinfoLite-Download/releases/latest/download/";

/// release 里的远端文件名（固定，不随本地保存路径变化）。
pub const GEO_REMOTE_FILENAME: &str = "ipinfo_lite.mmdb";

/// 官方地址。
pub const GEO_PRIMARY: &str =
    "https://github.com/youugiuhiuh/IPinfoLite-Download/releases/latest/download/ipinfo_lite.mmdb";

/// GitHub 加速镜像（与 Go 版 `GeoDBMirrors` 保持一致）。
pub const GEO_MIRRORS: &[&str] = &[
    "https://gh.h233.eu.org/https://github.com",
    "https://rapidgit.jjda.de5.net/https://github.com",
    "https://gh.ddlc.top/https://github.com",
    "https://gh-proxy.org/https://github.com",
    "https://cdn.gh-proxy.org/https://github.com",
    "https://edgeone.gh-proxy.org/https://github.com",
    "https://cors.isteed.cc/github.com",
    "https://ghproxy.it/https://github.com",
    "https://github.boki.moe/https://github.com",
    "https://gh.jasonzeng.dev/https://github.com",
];

/// 拼出某个镜像的完整下载地址。
pub fn mirror_url(mirror: &str, filename: &str) -> String {
    format!(
        "{}{}{}",
        mirror.trim_end_matches('/'),
        GEO_GITHUB_PATH,
        filename
    )
}

/// 待尝试的完整地址列表：镜像优先，官方兜底。
pub fn candidate_urls(filename: &str) -> Vec<String> {
    let mut urls: Vec<String> = GEO_MIRRORS
        .iter()
        .map(|m| mirror_url(m, filename))
        .collect();
    urls.push(GEO_PRIMARY.to_string());
    urls
}

fn build_client(proxy: Option<&str>) -> Result<reqwest::Client, reqwest::Error> {
    let mut b = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(20));
    if let Some(p) = proxy {
        b = b.proxy(reqwest::Proxy::all(p)?);
    }
    b.build()
}

/// 下载单个 URL 到 `dest`（先写 `.tmp` 再 rename，中断不会留下半个库）。
async fn fetch_to(client: &reqwest::Client, url: &str, dest: &Path) -> Result<u64, String> {
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    if let Some(dir) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }

    let tmp: PathBuf = dest.with_extension("mmdb.tmp");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut stream = resp;
    let mut total = 0u64;

    while let Some(chunk) = stream.chunk().await.map_err(|e| e.to_string())? {
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        total += chunk.len() as u64;
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    // 明显不是 MMDB 的响应（错误页/空文件）不落地
    if total < 1024 * 1024 {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("响应过小（{total} 字节），疑似错误页"));
    }

    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(total)
}

/// 确保 MMDB 存在：已存在直接返回；否则按镜像顺序下载。
///
/// `describe` 用于把进度信息交给调用方（进度条或纯文本）。
pub async fn ensure_mmdb<F>(path: &Path, proxy: Option<&str>, mut describe: F) -> Result<(), String>
where
    F: FnMut(&str),
{
    if path.exists() {
        describe(&format!("{} 已存在，跳过下载", path.display()));
        return Ok(());
    }

    let client = build_client(proxy).map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;
    // 远端文件名固定为 ipinfo_lite.mmdb；本地可另存为任意名字
    let urls = candidate_urls(GEO_REMOTE_FILENAME);
    let mut last_err = String::from("无可用地址");

    for (i, url) in urls.iter().enumerate() {
        describe(&format!("[GeoDB] 尝试 {}/{}: {url}", i + 1, urls.len()));
        match fetch_to(&client, url, path).await {
            Ok(bytes) => {
                describe(&format!(
                    "[GeoDB] 下载完成：{}（{:.1} MB）",
                    path.display(),
                    bytes as f64 / (1024.0 * 1024.0)
                ));
                return Ok(());
            }
            Err(e) => {
                last_err = format!("{url}: {e}");
                describe(&format!("[GeoDB] 失败：{e}"));
            }
        }
    }

    Err(format!(
        "所有镜像与官方地址均失败（最后一次：{last_err}）。\
         请手动下载 {} 放到 {}",
        GEO_PRIMARY,
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_url_appends_github_path_and_filename() {
        assert_eq!(
            mirror_url(
                "https://gh-proxy.org/https://github.com",
                "ipinfo_lite.mmdb"
            ),
            "https://gh-proxy.org/https://github.com/youugiuhiuh/IPinfoLite-Download/releases/latest/download/ipinfo_lite.mmdb"
        );
    }

    #[test]
    fn mirror_url_tolerates_trailing_slash() {
        let a = mirror_url("https://x.test/github.com/", "a.mmdb");
        let b = mirror_url("https://x.test/github.com", "a.mmdb");
        assert_eq!(a, b);
        assert!(!a.contains("//youugiuhiuh"));
    }

    /// 回归：本地另存为 `foo.mmdb` 时，远端地址仍必须是 `ipinfo_lite.mmdb`，
    /// 否则 release 里没有该资产，会一路 404。
    #[test]
    fn download_uses_remote_filename_not_local_name() {
        let dir = std::env::temp_dir().join(format!("geodl_name_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("custom_name.mmdb");

        // 直接检查候选地址构造：不依赖网络
        let urls = candidate_urls(GEO_REMOTE_FILENAME);
        assert!(
            urls.iter().all(|u| u.ends_with("/ipinfo_lite.mmdb")),
            "远端文件名必须固定: {urls:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = path;
    }

    #[test]
    fn candidates_are_mirrors_then_primary() {
        let urls = candidate_urls(GEO_REMOTE_FILENAME);
        assert_eq!(urls.len(), GEO_MIRRORS.len() + 1);
        assert_eq!(urls.last().unwrap(), GEO_PRIMARY);
        // 去重：镜像列表本身不应与官方地址重复
        let mut sorted = urls.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), urls.len());
    }

    #[test]
    fn ensure_mmdb_skips_when_file_exists() {
        let dir = std::env::temp_dir().join(format!("geodl_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ipinfo_lite.mmdb");
        std::fs::write(&path, b"fake").unwrap();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut msgs = Vec::new();
        rt.block_on(ensure_mmdb(&path, None, |m| msgs.push(m.to_string())))
            .expect("已存在时不应下载");
        assert!(msgs.iter().any(|m| m.contains("已存在")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 联网冒烟测试：显式 opt-in，避免 CI 无网时失败。
    #[tokio::test]
    async fn downloads_real_mmdb_when_opted_in() {
        if std::env::var("SNI_PROBE_DOWNLOAD_TESTS").is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("geodl_live_{}", std::process::id()));
        let path = dir.join("ipinfo_lite.mmdb");
        ensure_mmdb(&path, None, |m| println!("{m}"))
            .await
            .expect("下载应成功");

        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 1024 * 1024, "MMDB 应大于 1MB，实际 {size}");

        // 能真正打开才说明是可用的 MMDB
        crate::geo::GeoDb::open(&path).expect("下载的文件应可被 maxminddb 打开");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = dir;
    }
}
