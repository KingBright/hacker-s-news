use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Normalize text for stricter matching (remove whitespace, special chars, lowercase)
pub fn normalize_text(text: &str) -> String {
    // 1. Convert to lowercase
    let lower = text.to_lowercase();

    // 2. Remove all whitespace
    let re_space = Regex::new(r"\s+").unwrap();
    let no_space = re_space.replace_all(&lower, "");

    // 3. Remove punctuation/symbols (simplified regex)
    // Keep only alphanumeric and Chinese characters for comparison
    // This regex matches anything that is NOT a word char or Chinese char
    // (Adjust per requirement, this is a basic version)
    let re_punct = Regex::new(r"[^\w\u4e00-\u9fa5]+").unwrap();
    re_punct.replace_all(&no_space, "").to_string()
}

/// Calculate 64-bit SimHash for text similarity
pub fn calculate_simhash(text: &str) -> u64 {
    let mut counts = [0i32; 64];
    // Use chars including spaces for bigrams to capture word boundaries
    let chars: Vec<char> = text.chars().collect();

    if chars.is_empty() {
        return 0;
    }

    // Use bigrams for better context sensitivity
    // If text is "ABCD", bigrams are "AB", "BC", "CD"
    let mut tokens = Vec::new();
    for window in chars.windows(2) {
        tokens.push(window.iter().collect::<String>());
    }

    // Fallback for very short string
    if tokens.is_empty() {
        tokens.push(text.to_string());
    }

    for token in tokens {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        let hash = hasher.finish();

        for i in 0..64 {
            let bit = (hash >> i) & 1;
            if bit == 1 {
                counts[i] += 1;
            } else {
                counts[i] -= 1;
            }
        }
    }

    let mut fingerprint: u64 = 0;
    for i in 0..64 {
        if counts[i] > 0 {
            fingerprint |= 1 << i;
        }
    }

    fingerprint
}

/// Calculate Hamming Distance between two hashes
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize() {
        let raw = "Hello, World!  你好，世界。";
        let norm = normalize_text(raw);
        // expected: "helloworld你好世界" (assuming punctuation removed)
        assert_eq!(norm, "helloworld你好世界");
    }

    #[test]
    fn test_simhash_similar() {
        let t1 = "Apple发布了新的iPhone 15 Pro Max，搭载A17芯片";
        let t2 = "Apple刚发布了iPhone 15 Pro Max，采用A17处理芯片"; // slight var

        let h1 = calculate_simhash(t1);
        let h2 = calculate_simhash(t2);
        let dist = hamming_distance(h1, h2);

        println!("Distance: {}", dist);
        assert!(dist < 15); // Expect reasonably close
    }

    #[test]
    fn test_simhash_different() {
        let t1 = "今天天气真不错，适合出去玩";
        let t2 = "股市大跌，投资者心情沉重";

        let h1 = calculate_simhash(t1);
        let h2 = calculate_simhash(t2);
        let dist = hamming_distance(h1, h2);

        println!("Distance: {}", dist);
        assert!(dist > 20); // Expect different
    }
}
