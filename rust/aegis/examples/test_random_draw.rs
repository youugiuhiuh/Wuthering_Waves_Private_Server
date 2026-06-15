use std::collections::HashSet;

fn main() {
    println!("=== 完整随机抽取流程测试 ===\n");

    // 模拟场景：用户选择生成 20 个 Reality 节点
    println!("用户请求: Reality US, 数量=20");

    // 模拟 SNISelector 行为
    let domains = vec![
        "example1.com",
        "example2.com",
        "example3.com",
        "example4.com",
        "example5.com",
        "example6.com",
        "example7.com",
        "example8.com",
        "example9.com",
        "example10.com",
    ];

    println!("\n1. 读取 US.bin → {} 个域名", domains.len());

    // 模拟随机索引池
    let mut indices: Vec<usize> = (0..domains.len()).collect();
    use rand::seq::SliceRandom;
    let mut rng = rand::thread_rng();
    indices.shuffle(&mut rng);

    println!("2. Fisher-Yates 洗牌生成索引池");

    // 抽取 20 个（用完重新洗牌）
    let mut results = Vec::new();
    let mut count = 0;
    while count < 20 {
        if indices.is_empty() {
            indices = (0..domains.len()).collect();
            indices.shuffle(&mut rng);
            println!("   → 索引池用完，重新洗牌");
        }

        let idx = indices.pop().unwrap();
        results.push(domains[idx].to_string());
        count += 1;

        if count % 5 == 0 {
            println!("   已抽取 {} 个, 剩余索引: {}", count, indices.len());
        }
    }

    println!("\n3. 抽取结果:");
    for (i, d) in results.iter().enumerate() {
        println!("   {}. {}", i + 1, d);
    }

    // 验证无重复
    let unique: HashSet<_> = results.iter().collect();
    println!(
        "\n4. 去重验证: {} 个抽取中 {} 个唯一域名",
        results.len(),
        unique.len()
    );

    if unique.len() == 20 {
        println!("✅ 无重复！");
    } else {
        println!("⚠️ 有重复（因为用完了一轮重新洗牌）");
    }

    println!("\n=== 持久化状态 ===");
    println!("保存状态到 /etc/wwps/tgbot/sni_state/reality_US.enc");
    println!("  - domains: 10 个");
    println!("  - shuffled_indices: [剩余索引]");
    println!("  - used_count: 20");
}
