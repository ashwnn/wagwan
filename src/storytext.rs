//! Extractive sentence selection and source text-fragment links.

#[derive(Clone, Debug)]
pub struct CandidateSentence {
    pub id: i64,
    pub article_id: i64,
    pub position: i64,
    pub text: String,
    pub embedding: Vec<f32>,
    pub article_seen_at: i64,
    pub uniqueness: f32,
}

/// Select sentence IDs for the extractive story text using the plan's lede and
/// maximal marginal relevance rules.
pub fn build_extractive(candidates: &[CandidateSentence], centroid: &[f32]) -> Vec<i64> {
    select_sentences(candidates, centroid, crate::tuning::STORYTEXT_MAX_SENTENCES)
}

/// Select a bounded, relevance-ordered sentence pool for LLM prompting.
pub fn select_llm_candidates(candidates: &[CandidateSentence], centroid: &[f32]) -> Vec<i64> {
    select_sentences(
        candidates,
        centroid,
        crate::tuning::LLM_MAX_SOURCE_SENTENCES,
    )
}

fn select_sentences(candidates: &[CandidateSentence], centroid: &[f32], limit: usize) -> Vec<i64> {
    let candidates: Vec<&CandidateSentence> = candidates
        .iter()
        .filter(|candidate| !candidate.text.trim().is_empty())
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut best_article: Option<(i64, f32)> = None;
    for &candidate in &candidates {
        match best_article {
            Some((_, uniqueness)) if uniqueness >= candidate.uniqueness => (),
            _ => best_article = Some((candidate.article_id, candidate.uniqueness)),
        }
    }
    let lede_article = best_article.map(|(id, _)| id).unwrap_or_default();
    let mut lede: Vec<&CandidateSentence> = candidates
        .iter()
        .filter(|candidate| candidate.article_id == lede_article)
        .copied()
        .collect();
    lede.sort_by_key(|candidate| candidate.position);
    let mut chosen: Vec<&CandidateSentence> = lede
        .into_iter()
        .take(crate::tuning::STORYTEXT_LEDE_SENTENCES.min(limit))
        .collect();
    if chosen.is_empty() {
        return Vec::new();
    }
    let lede_ids: Vec<i64> = chosen.iter().map(|candidate| candidate.id).collect();

    while chosen.len() < limit {
        let mut best: Option<(&CandidateSentence, f32)> = None;
        for &candidate in &candidates {
            if chosen.iter().any(|selected| selected.id == candidate.id) {
                continue;
            }
            let relevance = crate::score::dot(&candidate.embedding, centroid);
            if relevance < crate::tuning::STORYTEXT_MIN_RELEVANCE {
                continue;
            }
            let redundancy = chosen
                .iter()
                .map(|selected| crate::score::dot(&candidate.embedding, &selected.embedding))
                .reduce(f32::max)
                .unwrap_or(0.0);
            if redundancy >= crate::tuning::STORYTEXT_MAX_REDUNDANCY {
                continue;
            }
            let mmr = crate::tuning::STORYTEXT_RELEVANCE_WEIGHT * relevance
                - crate::tuning::STORYTEXT_REDUNDANCY_WEIGHT * redundancy;
            if best.is_none_or(|(_, best_mmr)| mmr > best_mmr) {
                best = Some((candidate, mmr));
            }
        }
        match best {
            Some((candidate, _)) => chosen.push(candidate),
            None => break,
        }
    }

    chosen.sort_by(|a, b| {
        let a_lede = lede_ids.contains(&a.id);
        let b_lede = lede_ids.contains(&b.id);
        b_lede
            .cmp(&a_lede)
            .then_with(|| a.article_seen_at.cmp(&b.article_seen_at))
            .then_with(|| a.position.cmp(&b.position))
            .then_with(|| a.id.cmp(&b.id))
    });
    chosen.into_iter().map(|candidate| candidate.id).collect()
}

/// Return the source URL with a text fragment for the sentence. Any existing
/// fragment is replaced because only one fragment can be active at a time.
pub fn fragment_url(url: &str, sentence: &str) -> String {
    let base = url.split_once('#').map_or(url, |(base, _)| base);
    let words: Vec<&str> = sentence.split_whitespace().collect();
    if words.is_empty() {
        return base.to_owned();
    }
    let first = words
        .iter()
        .take(crate::tuning::STORYTEXT_FRAGMENT_FIRST_WORDS)
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let last = words
        .iter()
        .rev()
        .take(crate::tuning::STORYTEXT_FRAGMENT_LAST_WORDS)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{base}#:~:text={},{}",
        encode_fragment_text(&first),
        encode_fragment_text(&last)
    )
}

fn encode_fragment_text(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sentence(
        id: i64,
        article_id: i64,
        position: i64,
        text: &str,
        embedding: Vec<f32>,
        uniqueness: f32,
        seen: i64,
    ) -> CandidateSentence {
        CandidateSentence {
            id,
            article_id,
            position,
            text: text.to_owned(),
            embedding,
            article_seen_at: seen,
            uniqueness,
        }
    }

    #[test]
    fn selection_keeps_high_uniqueness_lede_then_relevant_nonredundant_sentence() {
        let candidates = vec![
            sentence(2, 1, 1, "second lede sentence", vec![0.9, 0.1], 1.0, 10),
            sentence(1, 1, 0, "first lede sentence", vec![1.0, 0.0], 1.0, 10),
            sentence(4, 2, 0, "redundant sentence", vec![0.99, 0.1], 0.2, 11),
            sentence(5, 3, 0, "new relevant detail", vec![0.8, 0.6], 0.4, 12),
            sentence(6, 4, 0, "irrelevant sentence", vec![0.0, 1.0], 0.5, 13),
        ];
        assert_eq!(build_extractive(&candidates, &[1.0, 0.0]), vec![1, 2, 5]);
    }

    #[test]
    fn ignores_blank_text_and_llm_selection_respects_global_limit() {
        let mut candidates = vec![sentence(1, 1, 0, "  \n ", vec![1.0, 0.0], 1.0, 1)];
        candidates.extend(
            (0..crate::tuning::LLM_MAX_SOURCE_SENTENCES + 5).map(|index| {
                sentence(
                    index as i64 + 2,
                    index as i64 + 2,
                    0,
                    "usable text",
                    vec![1.0, 0.0],
                    1.0,
                    index as i64 + 2,
                )
            }),
        );
        let selected = select_llm_candidates(&candidates, &[1.0, 0.0]);
        assert!(!selected.contains(&1));
        assert!(selected.len() <= crate::tuning::LLM_MAX_SOURCE_SENTENCES);
    }

    #[test]
    fn text_fragment_replaces_existing_fragment_and_percent_encodes_spaces() {
        assert_eq!(
            fragment_url("https://example.test/a#old", "Alpha beta, gamma"),
            "https://example.test/a#:~:text=Alpha%20beta%2C%20gamma,Alpha%20beta%2C%20gamma"
        );
        assert_eq!(
            fragment_url("https://example.test/a", ""),
            "https://example.test/a"
        );
    }
}
