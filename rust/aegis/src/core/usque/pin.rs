//! usque 二进制的版本 pin 与校验纯函数。
//!
//! # 为什么是「硬编码 pin」而不是运行时查最新版
//!
//! usque 的 Release **没有 GPG / minisign 签名**，只提供同仓库的
//! `checksums.txt`。这意味着：
//!
//! - 没有独立信任根可以验证二进制来源；
//! - `checksums.txt` 与二进制在**同一个 Release** 里，属于「同域自证」——
//!   它能防传输损坏，**防不了仓库被投毒或上游误发版**。
//!
//! 因此版本号与 sha256 都硬编码在代码里（见 [`USQUE_VERSION`] 与
//! [`USQUE_SHA256_LINUX_AMD64`]），升级必须改代码并发版。
//! 这样上游的任何变化都无法悄悄到达生产。
//!
//! # 本文件的函数都是纯函数
//!
//! 无 I/O、无网络、无环境依赖，因此可以直接单测。
//! 真正的下载与执行在 [`super::installer`]。

use crate::core::paths::usque as usque_paths;

/// 已 pin 的 usque 版本。
///
/// 升级流程：改这个常量 + 改对应架构的 sha256 常量 + 跑真机验证。
pub const USQUE_VERSION: &str = "4.2.1";

/// Linux amd64 的官方 zip 包 sha256。
///
/// 来源：`https://github.com/Diniboy1123/usque/releases/download/v4.2.1/checksums.txt`
/// 真机实测校验一致（见 `tasks/usque-tcp/FINDINGS.md`）。
pub const USQUE_SHA256_LINUX_AMD64: &str =
    "4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313";

/// Linux arm64 的官方 zip 包 sha256。
///
/// 同样取自 v4.2.1 的 `checksums.txt`。
pub const USQUE_SHA256_LINUX_ARM64: &str =
    "c88b061c2a567f30813d7505637d1fb6fe7ec5898b5b61fd05122409fa5ad925";

pub const GITHUB_REPO: &str = "Diniboy1123/usque";

/// 由架构得到官方资产名。
///
/// **只支持已 pin 的两种架构。** 未知架构返回 `None` —— 调用方必须
/// 据此**明确失败**，而不是退化成「不校验就装」。
///
/// 这是刻意的安全取舍：本模块没有其它信任根，跳过校验等于允许任意二进制入库。
pub fn asset_name(arch: &str) -> Option<String> {
    let suffix = match arch {
        "x86_64" | "amd64" => "linux_amd64",
        "aarch64" | "arm64" => "linux_arm64",
        _ => return None,
    };
    Some(format!("usque_{USQUE_VERSION}_{suffix}.zip"))
}

/// 已 pin 架构对应的期望 sha256；未知架构返回 `None`。
pub fn expected_sha256(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" | "amd64" => Some(USQUE_SHA256_LINUX_AMD64),
        "aarch64" | "arm64" => Some(USQUE_SHA256_LINUX_ARM64),
        _ => None,
    }
}

/// 由资产名构造下载 URL。
pub fn download_url(asset: &str) -> String {
    format!("https://github.com/{GITHUB_REPO}/releases/download/v{USQUE_VERSION}/{asset}")
}

/// `checksums.txt` 的 URL。
pub fn checksums_url() -> String {
    format!("https://github.com/{GITHUB_REPO}/releases/download/v{USQUE_VERSION}/checksums.txt")
}

/// 比较两个 sha256 是否相等（大小写不敏感，容忍 `sha256:` 前缀与空白）。
///
/// **只接受 64 位十六进制串。** 长度或字符集不符一律 `false`。
///
/// 这是刻意的严格：这个值来自网络（不可信输入）。若允许任意短串比较，
/// 一个被构造成空串或截断的「哈希」会在某些下游实现里被当成校验通过；
/// 而本模块没有任何其它信任根，放过一次就等于允许任意二进制入库。
pub fn verify_sha256(expected: &str, actual: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .trim_start_matches("sha256:")
            .trim()
            .to_ascii_lowercase()
    };
    let (e, a) = (norm(expected), norm(actual));
    is_hex64(&e) && is_hex64(&a) && e == a
}

/// 从 Release 的 `checksums.txt` 中提取指定资产的 sha256。
///
/// 兼容的格式（真实文件用两空格，但容忍单空格与 tab）：
///
/// ```text
/// 4117e206…  usque_4.2.1_linux_amd64.zip
/// ```
///
/// 也容忍文件名带 `./` 或路径前缀（`*` 是 GNU coreutils 的二进制模式标记）。
/// 找不到该资产时返回 `None`。
pub fn parse_checksums(text: &str, asset: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // 以第一个空白分隔：左边哈希，右边文件名。
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        // 归一化文件名：先去掉 GNU coreutils 的二进制模式标记 `*`，
        // 再去掉目录前缀。顺序不能反——`./dist/*name.zip` 里 `*` 在
        // 斜杠**之后**，先按 `/` 切分会把 `*` 留在名字开头。
        let name = name.trim_start_matches('*');
        let name = name.rsplit('/').next().unwrap_or(name);
        let name = name.trim_start_matches('*');
        if name == asset && is_hex64(hash) {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

/// 是否为 64 位小写/大写十六进制串。
fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 由架构得到「期望的资产名 + 期望的 sha256」。
///
/// 两者必须同时存在；任一缺失即返回 `None`（未知架构），
/// 调用方据此拒绝安装而不是跳过校验。
pub fn pinned_target(arch: &str) -> Option<(String, &'static str)> {
    Some((asset_name(arch)?, expected_sha256(arch)?))
}

/// 已 pin 的二进制在磁盘上的路径（便于调用方少写一次 import）。
pub fn binary_path() -> &'static str {
    usque_paths::BIN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_name_known_arches() {
        assert_eq!(
            asset_name("x86_64").as_deref(),
            Some("usque_4.2.1_linux_amd64.zip")
        );
        assert_eq!(
            asset_name("amd64").as_deref(),
            Some("usque_4.2.1_linux_amd64.zip")
        );
        assert_eq!(
            asset_name("aarch64").as_deref(),
            Some("usque_4.2.1_linux_arm64.zip")
        );
        assert_eq!(
            asset_name("arm64").as_deref(),
            Some("usque_4.2.1_linux_arm64.zip")
        );
    }

    /// 未知架构必须返回 None，调用方据此**拒绝安装**。
    ///
    /// 不能退化成「不校验就装」——本模块没有其它信任根。
    #[test]
    fn test_unknown_arch_is_none_not_fallback() {
        assert_eq!(asset_name("riscv64"), None);
        assert_eq!(asset_name("mips"), None);
        assert_eq!(asset_name(""), None);
        assert_eq!(expected_sha256("riscv64"), None);
        assert_eq!(pinned_target("riscv64"), None);
    }

    /// sha256 必须是 64 位十六进制。
    ///
    /// 防的是粘贴时被截断/多带字符——那种错误会让校验**永远失败**，
    /// 或者在实现不严谨时被静默跳过。
    #[test]
    fn test_pinned_hashes_are_well_formed() {
        for h in [USQUE_SHA256_LINUX_AMD64, USQUE_SHA256_LINUX_ARM64] {
            assert_eq!(h.len(), 64, "sha256 必须是 64 位: {h}");
            assert!(
                h.bytes().all(|b| b.is_ascii_hexdigit()),
                "sha256 必须全是十六进制: {h}"
            );
            assert_eq!(h, h.to_ascii_lowercase(), "sha256 应为小写: {h}");
        }
    }

    /// pin 的 amd64 哈希必须与真机实测值一致（防手误改坏）。
    #[test]
    fn test_amd64_hash_matches_field_measurement() {
        assert_eq!(
            USQUE_SHA256_LINUX_AMD64,
            "4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313"
        );
    }

    #[test]
    fn test_download_url_shape() {
        let a = asset_name("x86_64").unwrap();
        assert_eq!(
            download_url(&a),
            "https://github.com/Diniboy1123/usque/releases/download/v4.2.1/usque_4.2.1_linux_amd64.zip"
        );
        assert_eq!(
            checksums_url(),
            "https://github.com/Diniboy1123/usque/releases/download/v4.2.1/checksums.txt"
        );
    }

    #[test]
    fn test_verify_sha256_equal_and_case_insensitive() {
        let h = USQUE_SHA256_LINUX_AMD64;
        assert!(verify_sha256(h, h));
        assert!(verify_sha256(h, &h.to_ascii_uppercase()));
        assert!(verify_sha256(&h.to_ascii_uppercase(), h));
        assert!(verify_sha256(&format!("sha256:{h}"), h));
        assert!(verify_sha256(h, &format!("  {h}  ")));
    }

    #[test]
    fn test_verify_sha256_rejects_mismatch_and_garbage() {
        let h = USQUE_SHA256_LINUX_AMD64;
        assert!(!verify_sha256(h, ""));
        assert!(!verify_sha256("", h));
        assert!(!verify_sha256(h, "deadbeef"));
        // 只差最后一个字符也必须失败
        let mut altered = h.to_string();
        altered.pop();
        assert!(!verify_sha256(h, &altered));
        // 大小写归一后长度仍不符
        assert!(!verify_sha256("ABC", "abc"));
    }

    /// 真实 `checksums.txt` 的两种架构都要能解析出来。
    #[test]
    fn test_parse_checksums_real_file() {
        let real = "\
4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313  usque_4.2.1_linux_amd64.zip
c88b061c2a567f30813d7505637d1fb6fe7ec5898b5b61fd05122409fa5ad925  usque_4.2.1_linux_arm64.zip
c9675395d6f253b14037570e02dc0d40595dbbb5861d00503f52c4778c9109f3  checksums.txt
";
        assert_eq!(
            parse_checksums(real, "usque_4.2.1_linux_amd64.zip").as_deref(),
            Some(USQUE_SHA256_LINUX_AMD64)
        );
        assert_eq!(
            parse_checksums(real, "usque_4.2.1_linux_arm64.zip").as_deref(),
            Some(USQUE_SHA256_LINUX_ARM64)
        );
    }

    #[test]
    fn test_parse_checksums_tolerates_format_variants() {
        // 单空格
        assert_eq!(
            parse_checksums(
                "4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313 usque_4.2.1_linux_amd64.zip",
                "usque_4.2.1_linux_amd64.zip"
            )
            .as_deref(),
            Some(USQUE_SHA256_LINUX_AMD64)
        );
        // tab
        assert_eq!(
            parse_checksums(
                "4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313\tusque_4.2.1_linux_amd64.zip",
                "usque_4.2.1_linux_amd64.zip"
            )
            .as_deref(),
            Some(USQUE_SHA256_LINUX_AMD64)
        );
        // GNU 二进制模式标记 + 目录前缀
        assert_eq!(
            parse_checksums(
                "4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313  ./dist/*usque_4.2.1_linux_amd64.zip",
                "usque_4.2.1_linux_amd64.zip"
            )
            .as_deref(),
            Some(USQUE_SHA256_LINUX_AMD64)
        );
        // 大写哈希
        assert_eq!(
            parse_checksums(
                "4117E20695078AF9C11EDECD1A826C009BBC7EA0B7F64458612B4198910BC313  usque_4.2.1_linux_amd64.zip",
                "usque_4.2.1_linux_amd64.zip"
            )
            .as_deref(),
            Some(USQUE_SHA256_LINUX_AMD64)
        );
        // 注释与空行
        assert_eq!(
            parse_checksums(
                "# comment\n\n4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313  usque_4.2.1_linux_amd64.zip\n",
                "usque_4.2.1_linux_amd64.zip"
            )
            .as_deref(),
            Some(USQUE_SHA256_LINUX_AMD64)
        );
    }

    #[test]
    fn test_parse_checksums_missing_asset_is_none() {
        let real = "4117e20695078af9c11edecd1a826c009bbc7ea0b7f64458612b4198910bc313  usque_4.2.1_linux_amd64.zip\n";
        assert_eq!(parse_checksums(real, "usque_9.9.9_linux_riscv.zip"), None);
        assert_eq!(parse_checksums("", "anything"), None);
    }

    /// 非 64 位 hex 的「哈希」不得被当成有效值返回。
    /// 这防的是 checksums.txt 被构造成含短串时静默通过。
    #[test]
    fn test_parse_checksums_rejects_non_hex_hash() {
        assert_eq!(
            parse_checksums(
                "notahash  usque_4.2.1_linux_amd64.zip",
                "usque_4.2.1_linux_amd64.zip"
            ),
            None
        );
        assert_eq!(
            parse_checksums(
                "deadbeef  usque_4.2.1_linux_amd64.zip",
                "usque_4.2.1_linux_amd64.zip"
            ),
            None
        );
    }

    #[test]
    fn test_pinned_target_pairs_name_and_hash() {
        let (name, hash) = pinned_target("x86_64").expect("amd64 应已 pin");
        assert_eq!(name, "usque_4.2.1_linux_amd64.zip");
        assert_eq!(hash, USQUE_SHA256_LINUX_AMD64);
    }
}
