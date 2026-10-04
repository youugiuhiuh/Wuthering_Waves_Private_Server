use prost::Message;

/// `src/resources/sni/` 下的 protobuf 资源目录。
const SNI_DIR: &str = "src/resources/sni";

/// `proto/sni.proto` 的最小镜像，用于在**编译期**统计各文件域数。
///
/// 为什么手写而不是 `include!` 已生成代码：build.rs 先被编译、后才运行，
/// clean build 时 `OUT_DIR/sni.rs` 尚不存在，`include!` 会直接编译失败。
///
/// 漂移防护：运行期用例 `t1_build_time_largest_matches_runtime_decode`
/// 断言此处算出的域数与运行期用**生成的**类型解码出的域数一致。
/// 一旦 `proto/sni.proto` 增删字段导致两侧解析结果不同，该用例会失败。
#[derive(Clone, PartialEq, Message)]
struct DomainList {
    #[prost(string, repeated, tag = "1")]
    domains: Vec<String>,
}

/// 找出域数最多的 .pb，把结果作为编译期常量交给运行期使用。
///
/// 动机：运行期一旦需要 fallback（GeoIP 判定不出国家、或该国 .pb 域数过少），
/// 原实现会把 172 个文件**全部解码**来找最大值 —— 实测峰值 305 MB。
/// 这类与运行期无关的构建期事实，不该由线上进程反复重算。
fn emit_largest_pb_constant() {
    let dir = std::path::Path::new(SNI_DIR);
    let mut best: Option<(String, usize)> = None;

    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("读取 {SNI_DIR} 失败: {e}"));

    let mut count = 0usize;
    for entry in entries {
        let entry = entry.expect("读取目录项失败");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pb") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        count += 1;
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("读取 {} 失败: {e}", path.display()));

        // 解码失败的文件不参与评选：与原实现一致（原实现用 filter_map 跳过）。
        if let Ok(list) = DomainList::decode(bytes.as_slice())
            && best.as_ref().is_none_or(|(_, n)| list.domains.len() > *n)
        {
            best = Some((name.to_string(), list.domains.len()));
        }
    }

    match best {
        Some((name, domains)) => {
            println!("cargo:rustc-env=AEGIS_LARGEST_PB={name}");
            println!("cargo:rustc-env=AEGIS_LARGEST_PB_COUNT={domains}");
            println!("cargo:warning=SNI 最大文件: {name} ({domains} 域名 / 共扫描 {count} 个 .pb)");
        }
        None => panic!("{SNI_DIR} 下没有可解码的 .pb 文件"),
    }
}

fn main() {
    prost_build::compile_protos(&["../../proto/sni.proto"], &["../../proto"])
        .expect("Failed to compile protobuf");

    // 改动任一 .pb 必须重新计算常量，否则运行期会拿着过期的「最大文件」。
    println!("cargo:rerun-if-changed={SNI_DIR}");
    println!("cargo:rerun-if-changed=../../proto/sni.proto");

    emit_largest_pb_constant();
}
