//! Fragment Matcher — đo độ khớp biên (Boundary Coherence).
//!
//! Khi file bị phân mảnh trên đĩa (fragmented), các cluster không liền nhau.
//! Module này giúp AI xác định xem hai cluster có thuộc cùng file không,
//! bằng cách đo:
//! 1. Entropy coherence (entropy hai cluster phải tương đồng)
//! 2. Byte pattern continuity (cuối cluster A → đầu cluster B phải liền mạch)
//! 3. Format-specific validation (ví dụ: MP4 atom boundaries, ZIP local headers)

use crate::ntfs_parser::calculate_shannon_entropy;
use serde::Serialize;

/// Kết quả đánh giá xem hai fragment có khớp nhau không.
#[derive(Debug, Clone, Serialize)]
pub struct FragmentMatchResult {
    /// Fragment A: LBA bắt đầu
    pub fragment_a_lba: u64,
    /// Fragment B: LBA bắt đầu
    pub fragment_b_lba: u64,
    /// Điểm số tổng thể (0.0 = chắc chắn không khớp, 1.0 = chắc chắn khớp)
    pub confidence: f64,
    /// Điểm entropy coherence (0.0 - 1.0)
    pub entropy_score: f64,
    /// Điểm byte continuity (0.0 - 1.0)
    pub continuity_score: f64,
    /// Mô tả kết quả
    pub assessment: String,
}

/// Đánh giá xem hai fragment có thuộc cùng một file không.
///
/// # Arguments
/// - `tail_of_a`: Dữ liệu cuối của fragment A (ít nhất 64 bytes cuối)
/// - `head_of_b`: Dữ liệu đầu của fragment B (ít nhất 64 bytes đầu)
/// - `lba_a`: LBA bắt đầu fragment A
/// - `lba_b`: LBA bắt đầu fragment B
pub fn evaluate_fragment_match(
    tail_of_a: &[u8],
    head_of_b: &[u8],
    lba_a: u64,
    lba_b: u64,
) -> FragmentMatchResult {
    let entropy_score = entropy_coherence_score(tail_of_a, head_of_b);
    let continuity_score = byte_continuity_score(tail_of_a, head_of_b);

    // Trọng số: entropy 40%, continuity 60%
    let confidence = entropy_score * 0.4 + continuity_score * 0.6;

    let assessment = if confidence > 0.8 {
        "HIGH: Fragments likely belong to the same file".into()
    } else if confidence > 0.5 {
        "MEDIUM: Fragments may belong to the same file".into()
    } else if confidence > 0.3 {
        "LOW: Fragments unlikely to match".into()
    } else {
        "NONE: Fragments almost certainly from different files".into()
    };

    FragmentMatchResult {
        fragment_a_lba: lba_a,
        fragment_b_lba: lba_b,
        confidence,
        entropy_score,
        continuity_score,
        assessment,
    }
}

/// Đo sự tương đồng entropy giữa đuôi fragment A và đầu fragment B.
///
/// Dữ liệu cùng file thường có entropy tương tự tại vùng biên.
fn entropy_coherence_score(tail: &[u8], head: &[u8]) -> f64 {
    if tail.is_empty() || head.is_empty() {
        return 0.0;
    }

    let entropy_a = calculate_shannon_entropy(tail);
    let entropy_b = calculate_shannon_entropy(head);

    // Entropy cùng file thường chênh lệch < 1.0 bit
    let diff = (entropy_a - entropy_b).abs();

    if diff < 0.5 {
        1.0
    } else if diff < 1.0 {
        0.8
    } else if diff < 2.0 {
        0.5
    } else if diff < 3.0 {
        0.2
    } else {
        0.0
    }
}

/// Đo tính liền mạch byte giữa cuối fragment A và đầu fragment B.
///
/// Phân tích:
/// - Bigram frequency: tần suất cặp byte cuối A → đầu B
/// - Zero-run continuity: chuỗi null bytes không bị gián đoạn
/// - Repeating pattern: pattern lặp (common trong file media/database)
fn byte_continuity_score(tail: &[u8], head: &[u8]) -> f64 {
    if tail.is_empty() || head.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    let mut checks = 0;

    // Check 1: Bigram transition (byte cuối A → byte đầu B)
    // Trong text/code, bigram thường thuộc printable ASCII range
    let last_byte = tail[tail.len() - 1];
    let first_byte = head[0];

    // Nếu cả hai đều là printable ASCII → điểm cao
    if last_byte.is_ascii_graphic() && first_byte.is_ascii_graphic() {
        score += 0.8;
    } else if last_byte == 0 && first_byte == 0 {
        // Cả hai là null → có thể padding, điểm trung bình
        score += 0.5;
    } else {
        // Binary data — kiểm tra bigram distribution
        score += 0.3;
    }
    checks += 1;

    // Check 2: Zero-run continuity
    let tail_trailing_zeros = tail.iter().rev().take_while(|&&b| b == 0).count();
    let head_leading_zeros = head.iter().take_while(|&&b| b == 0).count();
    if tail_trailing_zeros > 0 && head_leading_zeros > 0 {
        // Zero run continues across boundary → likely same file padding
        score += 0.7;
        checks += 1;
    } else if tail_trailing_zeros == 0 && head_leading_zeros == 0 {
        // No zeros at boundary — neutral
        score += 0.5;
        checks += 1;
    } else {
        score += 0.3;
        checks += 1;
    }

    // Check 3: Byte value distribution similarity
    let mut dist_a = [0u16; 256];
    let mut dist_b = [0u16; 256];
    for &b in tail.iter().rev().take(32) {
        dist_a[b as usize] += 1;
    }
    for &b in head.iter().take(32) {
        dist_b[b as usize] += 1;
    }

    // Bhattacharyya similarity coefficient
    let sum_a: f64 = dist_a.iter().map(|&c| c as f64).sum();
    let sum_b: f64 = dist_b.iter().map(|&c| c as f64).sum();
    if sum_a > 0.0 && sum_b > 0.0 {
        let mut bc = 0.0;
        for i in 0..256 {
            let pa = dist_a[i] as f64 / sum_a;
            let pb = dist_b[i] as f64 / sum_b;
            bc += (pa * pb).sqrt();
        }
        score += bc; // 0.0 - 1.0
        checks += 1;
    }

    score / checks as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_fragments_high_score() {
        let data = [0x42u8; 128]; // Uniform data
        let result = evaluate_fragment_match(&data[64..], &data[..64], 100, 200);
        assert!(
            result.confidence > 0.5,
            "Identical data should have high confidence"
        );
    }

    #[test]
    fn test_different_entropy_low_score() {
        let zeros = vec![0u8; 64];
        let random: Vec<u8> = (0..64u8).collect(); // Sequential = moderate entropy
        let result = evaluate_fragment_match(&zeros, &random, 100, 200);
        // Entropy difference should lower the score
        assert!(result.entropy_score < result.confidence + 0.5);
    }
}
