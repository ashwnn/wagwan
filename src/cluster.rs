//! Pure online article-to-story assignment and centroid helpers.

use std::collections::HashSet;

use crate::{embed::similarity, tuning};

#[derive(Debug, Clone)]
pub struct StoryCandidate {
    pub id: i64,
    pub centroid: Vec<f32>,
    pub first_seen_at: i64,
    pub updated_at: i64,
    pub closed: bool,
    /// Union of facts across earlier originals in this candidate story.
    pub facts: HashSet<String>,
}

/// Pick the highest time-weighted semantic match that shares a concrete fact.
pub fn choose_story(
    article_embedding: &[f32],
    article_facts: &HashSet<String>,
    stories: &[StoryCandidate],
    now: i64,
) -> Option<(i64, f32)> {
    let mut best: Option<(i64, f32)> = None;
    for story in stories {
        if story.closed
            || now.saturating_sub(story.updated_at) > tuning::STORY_CANDIDATE_WINDOW_SECS
            || now.saturating_sub(story.first_seen_at) > tuning::STORY_MAX_AGE_SECS
            || !shares_fact(article_facts, &story.facts)
        {
            continue;
        }
        let Some(sim) = similarity(article_embedding, &story.centroid) else {
            continue;
        };
        let hours = now.saturating_sub(story.updated_at).max(0) as f64 / 3600.0;
        let sigma = tuning::STORY_TIME_SIGMA_HOURS;
        let weight = (-hours * hours / (2.0 * sigma * sigma)).exp() as f32;
        let score = sim * weight;
        if score >= tuning::JOIN_THRESHOLD && best.is_none_or(|(_, best_score)| score > best_score)
        {
            best = Some((story.id, score));
        }
    }
    best
}

pub fn should_close_story(first_seen_at: i64, updated_at: i64, now: i64) -> bool {
    now.saturating_sub(updated_at) >= tuning::STORY_IDLE_SECS
        || now.saturating_sub(first_seen_at) > tuning::STORY_MAX_AGE_SECS
}

/// Recompute a centroid from original member vectors and normalize its mean.
pub fn updated_centroid(member_embeddings: &[Vec<f32>]) -> Option<Vec<f32>> {
    let first = member_embeddings.first()?;
    if first.is_empty() || member_embeddings.iter().any(|v| v.len() != first.len()) {
        return None;
    }
    let mut mean = vec![0.0f32; first.len()];
    for vector in member_embeddings {
        if vector.iter().any(|value| !value.is_finite()) {
            return None;
        }
        for (total, value) in mean.iter_mut().zip(vector) {
            *total += *value;
        }
    }
    let norm = mean.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return None;
    }
    for value in &mut mean {
        *value /= norm;
    }
    Some(mean)
}

fn shares_fact(a: &HashSet<String>, b: &HashSet<String>) -> bool {
    !a.is_empty() && a.iter().any(|fact| b.contains(fact))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64, centroid: Vec<f32>, facts: &[&str]) -> StoryCandidate {
        StoryCandidate {
            id,
            centroid,
            first_seen_at: 1_000,
            updated_at: 1_000,
            closed: false,
            facts: facts.iter().map(|fact| (*fact).to_owned()).collect(),
        }
    }

    #[test]
    fn joins_best_semantic_candidate_only_when_fact_gate_matches() {
        let stories = vec![
            candidate(1, vec![0.9, 0.435], &["person:alex"]),
            candidate(2, vec![1.0, 0.0], &["person:alex"]),
        ];
        let selected = choose_story(
            &[1.0, 0.0],
            &HashSet::from(["person:alex".into()]),
            &stories,
            1_000,
        );
        assert_eq!(selected.map(|(id, _)| id), Some(2));

        let unrelated = choose_story(
            &[1.0, 0.0],
            &HashSet::from(["date:2026-01-01".into()]),
            &stories,
            1_000,
        );
        assert_eq!(unrelated, None);
    }

    #[test]
    fn rejects_closed_expired_and_too_old_stories() {
        let mut closed = candidate(1, vec![1.0, 0.0], &["x"]);
        closed.closed = true;
        let mut idle = candidate(2, vec![1.0, 0.0], &["x"]);
        idle.updated_at = 1_000 - tuning::STORY_CANDIDATE_WINDOW_SECS - 1;
        let mut ancient = candidate(3, vec![1.0, 0.0], &["x"]);
        ancient.first_seen_at = 1_000 - tuning::STORY_MAX_AGE_SECS - 1;
        let stories = [closed, idle, ancient];
        assert_eq!(
            choose_story(&[1.0, 0.0], &HashSet::from(["x".into()]), &stories, 1_000),
            None
        );
    }

    #[test]
    fn close_boundary_uses_idle_and_max_age_limits() {
        assert!(!should_close_story(
            100,
            100,
            100 + tuning::STORY_IDLE_SECS - 1
        ));
        assert!(should_close_story(100, 100, 100 + tuning::STORY_IDLE_SECS));
        assert!(!should_close_story(
            100,
            100 + tuning::STORY_MAX_AGE_SECS,
            100 + tuning::STORY_MAX_AGE_SECS
        ));
        assert!(should_close_story(
            100,
            101 + tuning::STORY_MAX_AGE_SECS,
            101 + tuning::STORY_MAX_AGE_SECS
        ));
    }

    #[test]
    fn centroid_is_normalized_mean_and_rejects_invalid_dimensions() {
        let centroid = updated_centroid(&[vec![1.0, 0.0], vec![0.0, 1.0]]).unwrap();
        assert!((centroid[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((centroid[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(updated_centroid(&[vec![1.0], vec![1.0, 0.0]]), None);
        assert_eq!(updated_centroid(&[vec![f32::NAN]]), None);
    }
}
