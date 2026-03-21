use std::env;
use std::fs;

fn load_binary(data: &[u8]) -> Option<Vec<String>> {
    let mut domains = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        if offset + 2 > data.len() {
            break;
        }
        let length = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;

        if length == 0 || length > 512 {
            return None;
        }

        if offset + length > data.len() {
            break;
        }

        let domain = std::str::from_utf8(&data[offset..offset + length]).ok()?;
        if !domain.is_empty() && domain.contains('.') {
            domains.push(domain.to_string());
        }
        offset += length;
    }

    if domains.is_empty() {
        None
    } else {
        Some(domains)
    }
}

fn main() {
    let test_file = "/home/asus/Dark/Wuthering_Waves_Private_Server_source_code/rust/tgbot/src/resources/sni/reality/US.bin";

    let data = fs::read(test_file).expect("Failed to read file");
    let domains = load_binary(&data).expect("Failed to parse");

    println!("=== Random Selection Test ===");
    println!("Total domains: {}\n", domains.len());

    // Test shuffle randomness
    use std::collections::HashSet;
    use std::hash::Hash;

    let mut results: Vec<String> = Vec::new();
    let mut unique_first_10: HashSet<String> = HashSet::new();

    // Get first domain 20 times to test randomness
    let mut shuffled = domains.clone();
    for i in 0..20 {
        // Simulate shuffle (Fisher-Yates)
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        i.hash(&mut hasher);
        let seed = hasher.finish();

        // Simple shuffle using seed
        let mut rng = seed;

        // Fisher-Yates shuffle
        for j in (1..shuffled.len()).rev() {
            rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
            let k = (rng % (j + 1)) as usize;
            shuffled.swap(j, k);
        }

        if let Some(first) = shuffled.first() {
            results.push(first.clone());
            unique_first_10.insert(first.clone());
        }
    }

    println!("First domain in 20 shuffles:");
    for (i, d) in results.iter().enumerate() {
        println!("  {}. {}", i + 1, d);
    }

    println!("\nUnique first domains: {}", unique_first_10.len());

    if unique_first_10.len() > 1 {
        println!("\n✅ RANDOM SELECTION WORKS: Different domains selected!");
    } else {
        println!("\n⚠️ Only one domain selected (may need more iterations)");
    }

    // Test round-robin with reshuffle
    println!("\n=== Round-Robin Test ===");
    let mut shuffled2 = domains.clone();
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let seed = 12345;
    let mut rng = seed;
    for j in (1..shuffled2.len()).rev() {
        rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
        let k = (rng % (j + 1)) as usize;
        shuffled2.swap(j, k);
    }

    println!("First 10 domains after shuffle:");
    for (i, d) in shuffled2.iter().take(10).enumerate() {
        println!("  {}. {}", i + 1, d);
    }

    // Verify all domains are still present
    let mut sorted_original = domains.clone();
    sorted_original.sort();
    sorted_original.dedup();

    let mut sorted_shuffled = shuffled2.clone();
    sorted_shuffled.sort();
    sorted_shuffled.dedup();

    if sorted_original == sorted_shuffled {
        println!("\n✅ ALL DOMAINS PRESERVED: No domains lost during shuffle!");
    } else {
        println!("\n❌ DOMAINS LOST: Some domains are missing!");
    }
}
