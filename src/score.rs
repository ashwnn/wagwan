//! Ranking formulas for the For you and Discover lists.

/// Inputs that can be computed from one story and its current neighbors.
#[derive(Clone, Copy, Debug)]
pub struct ScoreInput {
    pub topic_sim: f32,
    pub taste: f32,
    pub coverage: usize,
    pub age_h: f32,
    pub novelty: f32,
    pub detail: f32,
    pub vote: i8,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoverReason {
    WidelyCovered,
    Unusual,
    NewDetail,
}

impl DiscoverReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WidelyCovered => "widely_covered",
            Self::Unusual => "unusual",
            Self::NewDetail => "new_detail",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scores {
    pub for_you: u8,
    pub discover: u8,
    pub for_you_eligible: bool,
    pub discover_eligible: bool,
    pub discover_reason: Option<DiscoverReason>,
}

pub const TOPIC_MATCH: f32 = crate::tuning::SCORE_TOPIC_MATCH;
pub const MUTE_THRESHOLD: f32 = crate::tuning::SCORE_MUTE_THRESHOLD;

/// Compute both 0..99 scores. Muted stories are never eligible for a list.
pub fn score(input: ScoreInput) -> Scores {
    if input.muted {
        return Scores {
            for_you: 0,
            discover: 0,
            for_you_eligible: false,
            discover_eligible: false,
            discover_reason: None,
        };
    }

    let coverage = input.coverage as f32;
    let raw_for_you = crate::tuning::SCORE_FOR_YOU_TOPIC_WEIGHT * input.topic_sim
        + crate::tuning::SCORE_FOR_YOU_TASTE_WEIGHT * input.taste
        + crate::tuning::SCORE_FOR_YOU_COVERAGE_WEIGHT * (1.0 + coverage).ln()
        - crate::tuning::SCORE_FOR_YOU_AGE_PENALTY * input.age_h.max(0.0);
    let for_you = score_curve(
        raw_for_you,
        crate::tuning::SCORE_FOR_YOU_SIGMOID_STEEPNESS,
        crate::tuning::SCORE_FOR_YOU_SIGMOID_MIDPOINT,
    );
    let for_you_eligible = input.topic_sim >= TOPIC_MATCH || input.vote > 0;
    let discover_eligible = !for_you_eligible;
    let raw_discover = crate::tuning::SCORE_DISCOVER_COVERAGE_WEIGHT * (1.0 + coverage).ln()
        + crate::tuning::SCORE_DISCOVER_NOVELTY_WEIGHT * input.novelty
        + input.detail
        + input.taste
        - crate::tuning::SCORE_DISCOVER_AGE_PENALTY * input.age_h.max(0.0);
    let reason = if coverage >= crate::tuning::SCORE_WIDELY_COVERED_FEEDS {
        DiscoverReason::WidelyCovered
    } else if input.novelty >= crate::tuning::SCORE_UNUSUAL_NOVELTY {
        DiscoverReason::Unusual
    } else {
        DiscoverReason::NewDetail
    };

    Scores {
        for_you,
        discover: if discover_eligible {
            score_curve(
                raw_discover,
                crate::tuning::SCORE_DISCOVER_SIGMOID_STEEPNESS,
                crate::tuning::SCORE_DISCOVER_SIGMOID_MIDPOINT,
            )
        } else {
            0
        },
        for_you_eligible,
        discover_eligible,
        discover_reason: discover_eligible.then_some(reason),
    }
}

fn score_curve(raw: f32, steepness: f32, midpoint: f32) -> u8 {
    let exponent = -steepness * (raw - midpoint);
    let sigmoid = if exponent >= 0.0 {
        let e = (-exponent).exp();
        e / (1.0 + e)
    } else {
        1.0 / (1.0 + exponent.exp())
    };
    (crate::tuning::SCORE_MAX * sigmoid)
        .round()
        .clamp(0.0, crate::tuning::SCORE_MAX) as u8
}

/// Highest dot similarity against unmuted topics, paired with the topic label.
/// Embeddings are expected to be normalized by the embedder.
pub fn best_topic<'a>(
    centroid: &[f32],
    topics: &'a [(String, Vec<f32>, bool)],
) -> (f32, Option<&'a str>) {
    topics
        .iter()
        .filter(|(_, _, muted)| !muted)
        .map(|(name, embedding, _)| (dot(centroid, embedding), Some(name.as_str())))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap_or((0.0, None))
}

/// Apply both mute rules from the plan: matching any muted topic hides a story,
/// as does having every original article come from a muted feed.
pub fn is_muted(centroid: &[f32], muted_topics: &[Vec<f32>], original_feed_muted: &[bool]) -> bool {
    muted_topics
        .iter()
        .any(|embedding| dot(centroid, embedding) >= MUTE_THRESHOLD)
        || (!original_feed_muted.is_empty() && original_feed_muted.iter().all(|muted| *muted))
}

/// User taste is mean liked similarity minus mean disliked similarity. Votes
/// should already be limited to the preceding 90 days.
pub fn taste(centroid: &[f32], voted_centroids: &[(&[f32], i8)]) -> f32 {
    let mut liked_sum = 0.0;
    let mut liked_count = 0usize;
    let mut disliked_sum = 0.0;
    let mut disliked_count = 0usize;
    for (voted, vote) in voted_centroids {
        if *vote > 0 {
            liked_sum += dot(centroid, voted);
            liked_count += 1;
        } else if *vote < 0 {
            disliked_sum += dot(centroid, voted);
            disliked_count += 1;
        }
    }
    let liked = if liked_count == 0 {
        0.0
    } else {
        liked_sum / liked_count as f32
    };
    let disliked = if disliked_count == 0 {
        0.0
    } else {
        disliked_sum / disliked_count as f32
    };
    liked - disliked
}

pub fn novelty(centroid: &[f32], recent_centroids: &[Vec<f32>]) -> f32 {
    let max_similarity = recent_centroids
        .iter()
        .map(|recent| dot(centroid, recent))
        .reduce(f32::max)
        .unwrap_or(0.0);
    1.0 - max_similarity
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b).map(|(left, right)| left * right).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_curve(raw: f32, steepness: f32, midpoint: f32) -> u8 {
        (crate::tuning::SCORE_MAX / (1.0 + (-steepness * (raw - midpoint)).exp()))
            .round()
            .clamp(0.0, crate::tuning::SCORE_MAX) as u8
    }

    #[test]
    fn computes_for_you_formula_and_eligibility() {
        let input = ScoreInput {
            topic_sim: 0.8,
            taste: 0.2,
            coverage: 2,
            age_h: 4.0,
            novelty: 0.3,
            detail: 0.5,
            vote: 0,
            muted: false,
        };
        let scores = score(input);
        let raw = crate::tuning::SCORE_FOR_YOU_TOPIC_WEIGHT * input.topic_sim
            + crate::tuning::SCORE_FOR_YOU_TASTE_WEIGHT * input.taste
            + crate::tuning::SCORE_FOR_YOU_COVERAGE_WEIGHT * (1.0 + input.coverage as f32).ln()
            - crate::tuning::SCORE_FOR_YOU_AGE_PENALTY * input.age_h;
        assert_eq!(
            scores.for_you,
            expected_curve(
                raw,
                crate::tuning::SCORE_FOR_YOU_SIGMOID_STEEPNESS,
                crate::tuning::SCORE_FOR_YOU_SIGMOID_MIDPOINT
            )
        );
        assert!(scores.for_you_eligible);
        assert!(!scores.discover_eligible);
        assert_eq!(scores.discover, 0);
    }

    #[test]
    fn computes_discover_formula_and_reason_priority() {
        let mut input = ScoreInput {
            topic_sim: TOPIC_MATCH - 0.01,
            taste: 0.1,
            coverage: crate::tuning::SCORE_WIDELY_COVERED_FEEDS as usize,
            age_h: 2.0,
            novelty: crate::tuning::SCORE_UNUSUAL_NOVELTY + 0.1,
            detail: 0.4,
            vote: 0,
            muted: false,
        };
        let result = score(input);
        let raw = crate::tuning::SCORE_DISCOVER_COVERAGE_WEIGHT
            * (1.0 + input.coverage as f32).ln()
            + crate::tuning::SCORE_DISCOVER_NOVELTY_WEIGHT * input.novelty
            + input.detail
            + input.taste
            - crate::tuning::SCORE_DISCOVER_AGE_PENALTY * input.age_h;
        assert_eq!(
            result.discover,
            expected_curve(
                raw,
                crate::tuning::SCORE_DISCOVER_SIGMOID_STEEPNESS,
                crate::tuning::SCORE_DISCOVER_SIGMOID_MIDPOINT
            )
        );
        assert_eq!(result.discover_reason, Some(DiscoverReason::WidelyCovered));
        assert!(!result.for_you_eligible);
        assert!(result.discover_eligible);

        input.coverage = 1;
        assert_eq!(score(input).discover_reason, Some(DiscoverReason::Unusual));
        input.novelty = 0.0;
        assert_eq!(
            score(input).discover_reason,
            Some(DiscoverReason::NewDetail)
        );
    }

    #[test]
    fn votes_can_make_for_you_eligible_but_muted_stories_are_hidden() {
        let input = ScoreInput {
            topic_sim: 0.0,
            taste: 0.0,
            coverage: 1,
            age_h: 0.0,
            novelty: 0.0,
            detail: 0.0,
            vote: 1,
            muted: false,
        };
        assert!(score(input).for_you_eligible);
        let muted = score(ScoreInput {
            muted: true,
            ..input
        });
        assert_eq!(muted.for_you, 0);
        assert_eq!(muted.discover, 0);
        assert!(!muted.for_you_eligible && !muted.discover_eligible);
        assert_eq!(muted.discover_reason, None);
    }

    #[test]
    fn applies_topic_and_all_original_feed_mute_rules() {
        assert!(is_muted(&[1.0, 0.0], &[vec![1.0, 0.0]], &[false]));
        assert!(is_muted(&[1.0, 0.0], &[], &[true, true]));
        assert!(!is_muted(&[1.0, 0.0], &[], &[true, false]));
        assert!(!is_muted(&[1.0, 0.0], &[], &[]));
    }

    #[test]
    fn taste_and_novelty_use_mean_votes_and_max_recent_similarity() {
        let centroid = [1.0, 0.0];
        let up = [0.8, 0.6];
        let down = [-1.0, 0.0];
        assert!((taste(&centroid, &[(&up, 1), (&down, -1)]) - 1.8).abs() < 1e-6);
        assert!((novelty(&centroid, &[vec![0.4, 0.0], vec![0.8, 0.0]]) - 0.2).abs() < 1e-6);
        assert_eq!(novelty(&centroid, &[]), 1.0);
    }
}
