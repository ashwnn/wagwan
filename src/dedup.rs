//! Deterministic MinHash and exact containment for within-story copy detection.

use std::collections::HashSet;

use crate::tuning;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinHash128(pub [u32; 128]);

#[derive(Debug, Clone)]
pub struct CopyMatch {
    pub article_id: i64,
    pub jaccard: f32,
    pub containment: f32,
}

/// Lowercase and tokenize text into word 5-gram shingles.
pub fn shingles(text: &str) -> HashSet<String> {
    let words: Vec<String> = text
        .split(|ch: char| !ch.is_alphanumeric() && ch != '\'')
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect();
    if words.len() < 5 {
        return HashSet::new();
    }
    words.windows(5).map(|window| window.join(" ")).collect()
}

/// Compute a stable 128-permutation MinHash signature without a dependency.
pub fn minhash(text: &str) -> MinHash128 {
    let set = shingles(text);
    let mut signature = [u32::MAX; 128];
    for shingle in set {
        let base = fnv1a(shingle.as_bytes());
        for (index, value) in signature.iter_mut().enumerate() {
            let (a, b) = permutation(index as u32);
            let hash = a.wrapping_mul(base).wrapping_add(b);
            *value = (*value).min(hash);
        }
    }
    MinHash128(signature)
}

pub fn estimated_jaccard(a: &MinHash128, b: &MinHash128) -> f32 {
    a.0.iter().zip(b.0.iter()).filter(|(x, y)| x == y).count() as f32 / tuning::MINHASH_SIZE as f32
}

pub fn exact_jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    intersection as f32 / (a.len() + b.len() - intersection) as f32
}

/// |A ∩ B| / min(|A|, |B|), the syndicated-copy test for an added local section.
pub fn containment(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    let denominator = a.len().min(b.len());
    if denominator == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f32 / denominator as f32
}

#[derive(Debug, Clone)]
pub struct EarlierCopyCandidate<'a> {
    pub article_id: i64,
    pub signature: &'a MinHash128,
    pub shingles: &'a HashSet<String>,
}

/// Return the earliest copy match from candidates supplied in seen order.
pub fn earliest_copy(text: &str, candidates: &[EarlierCopyCandidate<'_>]) -> Option<CopyMatch> {
    let current_shingles = shingles(text);
    if current_shingles.is_empty() {
        return None;
    }
    let current_signature = minhash(text);
    let mut match_result: Option<CopyMatch> = None;
    for candidate in candidates {
        let estimated = estimated_jaccard(&current_signature, candidate.signature);
        if estimated < tuning::COPY_CANDIDATE_JACCARD {
            continue;
        }
        let jaccard = exact_jaccard(&current_shingles, candidate.shingles);
        let contained = containment(&current_shingles, candidate.shingles);
        let near_duplicate = estimated >= tuning::COPY_JACCARD_THRESHOLD;
        let syndicated_copy = estimated >= tuning::COPY_CANDIDATE_JACCARD
            && contained >= tuning::COPY_CONTAINMENT_THRESHOLD;
        if near_duplicate || syndicated_copy {
            match_result = Some(CopyMatch {
                article_id: candidate.article_id,
                jaccard,
                containment: contained,
            });
            break;
        }
    }
    match_result
}

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c9dc5u32;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

fn permutation(index: u32) -> (u32, u32) {
    let mut x = index.wrapping_add(0x9e3779b9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85ebca6b);
    x ^= x >> 13;
    x = x.wrapping_mul(0xc2b2ae35);
    x ^= x >> 16;
    // Odd multipliers make each affine map invertible modulo 2^32.
    (x | 1, x.rotate_left(13))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shingles_are_case_insensitive_word_five_grams() {
        assert_eq!(
            shingles("Alpha beta gamma delta epsilon"),
            shingles("ALPHA BETA GAMMA DELTA EPSILON")
        );
        assert_eq!(shingles("one two three four"), HashSet::new());
        assert_eq!(
            shingles("one, two three four five"),
            HashSet::from(["one two three four five".to_owned()])
        );
    }

    #[test]
    fn minhash_is_deterministic_and_identical_text_has_full_jaccard() {
        let text =
            "A stable synthetic report containing enough distinct words for useful shingles.";
        let first = minhash(text);
        assert_eq!(first, minhash(text));
        assert_eq!(estimated_jaccard(&first, &minhash(text)), 1.0);
    }

    #[test]
    fn exact_jaccard_and_containment_handle_added_syndicated_material() {
        let source = shingles("alpha beta gamma delta epsilon zeta eta theta iota kappa");
        let extended = shingles(
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron",
        );
        assert_eq!(exact_jaccard(&source, &source), 1.0);
        assert!(containment(&source, &extended) >= 0.8);
        assert!(exact_jaccard(&source, &extended) < 1.0);
        assert_eq!(containment(&HashSet::new(), &source), 0.0);
    }

    #[test]
    fn labeled_synthetic_syndication_pairs_meet_precision_and_recall_gate() {
        // These are deliberately synthetic known pairs, not downloaded publisher data.
        let base = (0..100)
            .map(|index| format!("sourceword{index} contextword{index} detailword{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let local = (0..16)
            .map(|index| format!("localword{index} neighborhoodword{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let positives = [
            (base.clone(), format!("{base} {local}")),
            (
                format!("{base} closing section"),
                format!("{base} closing section {local}"),
            ),
            (base.to_ascii_uppercase(), format!("{base} {local}")),
        ];
        let negatives = [
            (
                "Synthetic report says coastal roads closed after heavy rain and flooding."
                    .to_owned(),
                "Synthetic report says coastal schools opened after heavy snow and power outages."
                    .to_owned(),
            ),
            (
                "Synthetic report: council approves new housing plan near the river.".to_owned(),
                "Synthetic report: council rejects transit plan near the airport.".to_owned(),
            ),
            (
                "Synthetic report says the team won the final after a late goal.".to_owned(),
                "Synthetic report says the team lost the opener after an early penalty.".to_owned(),
            ),
        ];
        let detected = |first: &str, second: &str| {
            let signature = minhash(first);
            let shingles = shingles(first);
            let candidates = [EarlierCopyCandidate {
                article_id: 1,
                signature: &signature,
                shingles: &shingles,
            }];
            earliest_copy(second, &candidates).is_some()
        };

        let true_positive = positives.iter().filter(|(a, b)| detected(a, b)).count();
        let false_negative = positives.len() - true_positive;
        let false_positive = negatives.iter().filter(|(a, b)| detected(a, b)).count();
        let true_negative = negatives.len() - false_positive;
        let accuracy =
            (true_positive + true_negative) as f32 / (positives.len() + negatives.len()) as f32;
        assert_eq!(
            false_negative, 0,
            "synthetic syndicated pairs should be detected"
        );
        assert_eq!(
            false_positive, 0,
            "distinct synthetic reports should not be marked copies"
        );
        assert!(accuracy >= 0.95);
    }
}
