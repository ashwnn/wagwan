//! Embedding, story assignment, copy detection, story text, and ranking pipeline.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::{
    cluster::{self, StoryCandidate},
    config::Config,
    db::{self, Db, MemberArticle, PendingArticle, StoredText, StoryState},
    dedup::{self, EarlierCopyCandidate},
    embed::{self, Embedder},
    facts,
    llm::{LlmClient, SourceSentence},
    score::{self, ScoreInput},
    storytext::{self, CandidateSentence},
    tuning,
};

#[derive(Clone)]
pub struct Pipeline {
    db: Db,
    config: Arc<Config>,
    model: Arc<Embedder>,
}

#[derive(Clone)]
struct ScoredStory {
    story_id: i64,
    for_you: u8,
    discover: u8,
    for_you_eligible: bool,
    discover_eligible: bool,
    muted: bool,
    topic_match: bool,
    discover_reason: Option<score::DiscoverReason>,
    input: ScoreInput,
    updated_at: i64,
    text_hash: String,
    centroid: Vec<f32>,
}

impl Pipeline {
    pub fn new(db: Db, config: Arc<Config>, model: Arc<Embedder>) -> Result<Self> {
        Ok(Self { db, config, model })
    }

    /// Embed and assign all currently pending articles, then rebuild text and ranking.
    /// Call this from `spawn_blocking` after each fetch cycle and hourly.
    pub fn process(&self) -> Result<()> {
        self.embed_uninitialized_topics()?;
        let now = unix_now();
        let mut after_seen_at = None;
        let mut after_id = None;
        let mut pending = self.db.pending_articles_after(
            tuning::PIPELINE_ARTICLE_BATCH_SIZE,
            after_seen_at,
            after_id,
        )?;
        if pending.is_empty() {
            self.rebuild_invalidated_texts()?;
            self.recompute_scores(now)?;
            return Ok(());
        }
        let mut stories = self
            .db
            .story_states(now.saturating_sub(tuning::STORY_CANDIDATE_WINDOW_SECS))?;
        let mut candidates = story_candidates(&stories);
        let mut changed_text = HashSet::new();
        loop {
            if let Some(last) = pending.last() {
                after_seen_at = Some(last.seen_at);
                after_id = Some(last.id);
            }
            for article in pending {
                match self.process_article(&article, &mut stories, &mut candidates) {
                    Ok(Some(story_id)) => {
                        changed_text.insert(story_id);
                    }
                    Ok(None) => (),
                    Err(error) => {
                        tracing::warn!(article_id = article.id, %error, "Could not process article; it will be retried")
                    }
                }
            }
            pending = self.db.pending_articles_after(
                tuning::PIPELINE_ARTICLE_BATCH_SIZE,
                after_seen_at,
                after_id,
            )?;
            if pending.is_empty() {
                break;
            }
        }

        let now = unix_now();
        self.close_expired_stories(&mut stories, now);
        self.rebuild_extractive_text(&mut stories, &changed_text)?;
        self.rebuild_invalidated_texts()?;
        self.recompute_scores(now)?;
        Ok(())
    }

    /// Generate and cache LLM stories for eligible For you and Discover stories.
    pub async fn generate_ai(&self) -> Result<()> {
        let (Some(base_url), Some(model_name)) =
            (&self.config.llm_base_url, &self.config.llm_model)
        else {
            return Ok(());
        };
        let client = Arc::new(LlmClient::new(
            base_url,
            model_name,
            self.config.llm_api_key.as_deref(),
        )?);
        let now = unix_now();
        let topics = topic_tuples(&self.db.embedded_topics()?);
        let recent = self
            .db
            .recent_centroids(now.saturating_sub(tuning::SCORE_NOVELTY_WINDOW_SECS))?;
        let voted = self
            .db
            .voted_centroids(now.saturating_sub(tuning::SCORE_VOTE_WINDOW_SECS))?;
        let mut eligible = Vec::new();
        let mut discover = Vec::new();
        let mut after_id = 0;
        loop {
            let page = self
                .db
                .score_states_page(after_id, tuning::SCORE_PAGE_SIZE)?;
            if page.is_empty() {
                break;
            }
            after_id = page.last().map_or(after_id, |story| story.id);
            for state in &page {
                let scored = score_story(state, &topics, &recent, &voted, now);
                if !scored.muted
                    && scored.for_you_eligible
                    && scored.for_you >= self.config.min_score
                {
                    eligible.push(scored.clone());
                }
                if !scored.muted
                    && scored.discover_eligible
                    && now.saturating_sub(scored.updated_at) <= tuning::DISCOVER_WINDOW_SECS
                {
                    discover.push(scored);
                }
            }
        }
        discover.sort_by(|a, b| {
            b.discover
                .cmp(&a.discover)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        eligible.extend(discover.into_iter().take(30));
        eligible.sort_by_key(|story| story.story_id);
        eligible.dedup_by_key(|story| story.story_id);

        let semaphore = Arc::new(Semaphore::new(self.config.llm_concurrency));
        let mut tasks = tokio::task::JoinSet::new();
        for candidate in eligible {
            if self
                .db
                .has_ai_for(candidate.story_id, &candidate.text_hash)?
            {
                continue;
            }
            let source_candidates = self
                .db
                .text_candidates_limited(candidate.story_id, tuning::LLM_MAX_SOURCE_SENTENCES)?;
            let selectable: Vec<CandidateSentence> = source_candidates
                .iter()
                .map(|sentence| CandidateSentence {
                    id: sentence.id,
                    article_id: sentence.article_id,
                    position: sentence.position,
                    text: sentence.text.clone(),
                    embedding: sentence.embedding.clone(),
                    article_seen_at: sentence.article_seen_at,
                    uniqueness: sentence.uniqueness,
                })
                .collect();
            let selected_ids = storytext::select_llm_candidates(&selectable, &candidate.centroid);
            let sources: Vec<SourceSentence> = selected_ids
                .into_iter()
                .filter_map(|id| {
                    source_candidates
                        .iter()
                        .find(|sentence| sentence.id == id)
                        .map(|sentence| SourceSentence {
                            id: sentence.id,
                            outlet: sentence.outlet.clone(),
                            text: sentence.text.clone(),
                            embedding: sentence.embedding.clone(),
                        })
                })
                .collect();
            if sources.is_empty() {
                continue;
            }
            let client = client.clone();
            let model = self.model.clone();
            let db = self.db.clone();
            let expected_hash = candidate.text_hash.clone();
            let permit = semaphore.clone().acquire_owned().await?;
            let story_id = candidate.story_id;
            tasks.spawn(async move {
                let _permit = permit;
                let generated = client
                    .generate(&sources, model.clone())
                    .await
                    .with_context(|| format!("generating AI story {story_id}"))?;
                if generated.sentences.is_empty() {
                    return Ok(());
                }
                let text: Vec<StoredText> = generated
                    .sentences
                    .iter()
                    .map(|sentence| StoredText {
                        text: sentence.text.clone(),
                        sentence_id: sentence.primary_source_id,
                        extra_sources: sentence.extra_source_ids.clone(),
                        verified: sentence.verified,
                    })
                    .collect();
                let disagreements: Vec<db::Disagreement> = generated
                    .disagreements
                    .iter()
                    .map(|item| db::Disagreement {
                        subject: item.subject.clone(),
                        detail: item.detail.clone(),
                    })
                    .collect();
                db.store_ai_text_if_current(
                    story_id,
                    &expected_hash,
                    &text,
                    generated.title.as_deref(),
                    &disagreements,
                )?;
                Ok::<(), anyhow::Error>(())
            });
        }
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok(())) => (),
                Ok(Err(error)) => tracing::warn!(%error, "Could not generate AI story"),
                Err(error) => tracing::warn!(%error, "AI story worker failed"),
            }
        }
        Ok(())
    }

    fn embed_uninitialized_topics(&self) -> Result<()> {
        for topic in self.db.embedded_topics()? {
            if topic.embedding.is_empty() {
                let embedding = self
                    .model
                    .embed(&topic.name)
                    .with_context(|| format!("embedding topic {}", topic.name))?;
                self.db.update_topic_embedding(topic.id, &embedding)?;
            }
        }
        Ok(())
    }

    fn process_article(
        &self,
        article: &PendingArticle,
        stories: &mut Vec<StoryState>,
        candidates: &mut Vec<StoryCandidate>,
    ) -> Result<Option<i64>> {
        let input = embed::article_embedding_input(&article.title, &article.text);
        let embedding = self
            .model
            .embed(&input)
            .with_context(|| format!("embedding article {}", article.id))?;
        let sentence_texts: Vec<String> = article
            .sentences
            .iter()
            .map(|(_, text)| text.clone())
            .collect();
        let sentence_vectors = self
            .model
            .embed_batch(&sentence_texts)
            .with_context(|| format!("embedding sentences for article {}", article.id))?;
        let sentence_embeddings: Vec<(i64, Vec<f32>)> = article
            .sentences
            .iter()
            .zip(sentence_vectors)
            .map(|((id, _), vector)| (*id, vector))
            .collect();
        let signature = dedup::minhash(&article.text);
        let minhash_bytes: Vec<u8> = signature
            .0
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let current_facts = facts::fact_tokens(&article.text);
        let target = cluster::choose_story(&embedding, &current_facts, candidates, article.seen_at);
        let story_id = target.map(|(id, _)| id);
        let state_index = story_id.and_then(|id| stories.iter().position(|story| story.id == id));
        let copy_match = state_index.and_then(|index| {
            let mut earlier: Vec<&MemberArticle> = stories[index].members.iter().collect();
            earlier.sort_by_key(|member| (member.seen_at, member.id));
            // Existing signatures are not included in StoryState, so calculate them from source text.
            let owned: Vec<(i64, dedup::MinHash128, HashSet<String>)> = earlier
                .iter()
                .map(|member| {
                    (
                        member.id,
                        dedup::minhash(&member.text),
                        dedup::shingles(&member.text),
                    )
                })
                .collect();
            let refs: Vec<EarlierCopyCandidate<'_>> = owned
                .iter()
                .map(|(id, signature, shingles)| EarlierCopyCandidate {
                    article_id: *id,
                    signature,
                    shingles,
                })
                .collect();
            dedup::earliest_copy(&article.text, &refs)
        });
        if let Some(matched) = &copy_match {
            tracing::debug!(
                article_id = article.id,
                copy_of = matched.article_id,
                jaccard = matched.jaccard,
                containment = matched.containment,
                "Detected syndicated or duplicate article"
            );
        }
        let copy_of = copy_match.map(|matched| matched.article_id);
        let uniqueness = if copy_of.is_some() {
            0.0
        } else {
            let earlier_facts: Vec<HashSet<String>> = state_index
                .map(|index| {
                    stories[index]
                        .members
                        .iter()
                        .filter(|member| {
                            member.copy_of.is_none()
                                && (member.seen_at, member.id) < (article.seen_at, article.id)
                        })
                        .map(|member| facts::fact_tokens(&member.text))
                        .collect()
                })
                .unwrap_or_default();
            facts::uniqueness(&current_facts, &earlier_facts)
        };
        let mut member_embeddings: Vec<Vec<f32>> = state_index
            .map(|index| {
                stories[index]
                    .members
                    .iter()
                    .filter(|member| !member.embedding.is_empty())
                    .map(|member| member.embedding.clone())
                    .collect()
            })
            .unwrap_or_default();
        member_embeddings.push(embedding.clone());
        let centroid =
            cluster::updated_centroid(&member_embeddings).unwrap_or_else(|| embedding.clone());
        let assigned = self.db.assign_article(
            article,
            &embedding,
            &minhash_bytes,
            &sentence_embeddings,
            story_id,
            copy_of,
            uniqueness,
            &centroid,
        )?;
        if assigned == 0 {
            return Ok(None);
        }

        let index = if let Some(index) = stories.iter().position(|story| story.id == assigned) {
            index
        } else {
            stories.push(StoryState {
                id: assigned,
                centroid: centroid.clone(),
                first_seen_at: article.seen_at,
                updated_at: article.seen_at,
                closed: false,
                vote: 0,
                text_built_for: None,
                members: Vec::new(),
            });
            stories.len() - 1
        };
        let state = &mut stories[index];
        state.centroid = centroid;
        state.first_seen_at = state.first_seen_at.min(article.seen_at);
        if copy_of.is_none() {
            state.updated_at = state.updated_at.max(article.seen_at);
        }
        state.members.push(MemberArticle {
            id: article.id,
            feed_id: article.feed_id,
            title: article.title.clone(),
            text: article.text.clone(),
            seen_at: article.seen_at,
            embedding,
            copy_of,
            uniqueness,
            folder: article.folder.clone(),
            outlet: article.outlet.clone(),
            feed_muted: article.feed_muted,
        });
        if let Some(candidate) = candidates
            .iter_mut()
            .find(|candidate| candidate.id == assigned)
        {
            candidate.centroid = state.centroid.clone();
            candidate.first_seen_at = state.first_seen_at;
            if copy_of.is_none() {
                candidate.updated_at = candidate.updated_at.max(article.seen_at);
                candidate.facts.extend(current_facts);
            }
        } else {
            candidates.push(StoryCandidate {
                id: assigned,
                centroid: state.centroid.clone(),
                first_seen_at: state.first_seen_at,
                updated_at: state.updated_at,
                closed: state.closed,
                facts: current_facts,
            });
        }
        Ok(copy_of.is_none().then_some(assigned))
    }

    fn close_expired_stories(&self, stories: &mut [StoryState], now: i64) {
        for story in stories {
            if cluster::should_close_story(story.first_seen_at, story.updated_at, now) {
                story.closed = true;
            }
        }
    }

    fn rebuild_extractive_text(
        &self,
        active: &mut [StoryState],
        changed: &HashSet<i64>,
    ) -> Result<()> {
        for story in active
            .iter_mut()
            .filter(|story| changed.contains(&story.id))
        {
            self.rebuild_one_story_text(story)?;
        }
        Ok(())
    }

    fn rebuild_invalidated_texts(&self) -> Result<()> {
        let mut after_id = 0;
        loop {
            let mut page = self
                .db
                .stories_needing_text_page(after_id, tuning::SCORE_PAGE_SIZE)?;
            if page.is_empty() {
                break;
            }
            after_id = page.last().map_or(after_id, |story| story.id);
            for story in &mut page {
                self.rebuild_one_story_text(story)?;
            }
        }
        Ok(())
    }

    fn rebuild_one_story_text(&self, story: &mut StoryState) -> Result<()> {
        let mut original_ids: Vec<i64> = story
            .members
            .iter()
            .filter(|member| member.copy_of.is_none())
            .map(|member| member.id)
            .collect();
        original_ids.sort_unstable();
        let hash = ids_hash(&original_ids);
        if original_ids.is_empty() {
            self.db
                .store_text(story.id, "extractive", &[], Some(&hash), None, &[])?;
            story.text_built_for = Some(hash);
            return Ok(());
        }
        if story.text_built_for.as_deref() == Some(hash.as_str()) {
            return Ok(());
        }
        let db_candidates = self.db.text_candidates(story.id)?;
        let candidates: Vec<CandidateSentence> = db_candidates
            .iter()
            .map(|sentence| CandidateSentence {
                id: sentence.id,
                article_id: sentence.article_id,
                position: sentence.position,
                text: sentence.text.clone(),
                embedding: sentence.embedding.clone(),
                article_seen_at: sentence.article_seen_at,
                uniqueness: sentence.uniqueness,
            })
            .collect();
        let selected = storytext::build_extractive(&candidates, &story.centroid);
        let by_id: HashMap<i64, &db::TextSentence> = db_candidates
            .iter()
            .map(|sentence| (sentence.id, sentence))
            .collect();
        let stored: Vec<StoredText> = selected
            .iter()
            .filter_map(|id| {
                by_id.get(id).map(|sentence| StoredText {
                    text: sentence.text.clone(),
                    sentence_id: sentence.id,
                    extra_sources: Vec::new(),
                    verified: true,
                })
            })
            .collect();
        let best_title = story
            .members
            .iter()
            .filter(|member| member.copy_of.is_none())
            .max_by(|a, b| {
                a.uniqueness
                    .total_cmp(&b.uniqueness)
                    .then_with(|| b.seen_at.cmp(&a.seen_at))
            })
            .map(|member| member.title.as_str());
        self.db.store_text(
            story.id,
            "extractive",
            &stored,
            Some(&hash),
            best_title,
            &[],
        )?;
        story.text_built_for = Some(hash);
        Ok(())
    }

    fn recompute_scores(&self, now: i64) -> Result<()> {
        let topics = topic_tuples(&self.db.embedded_topics()?);
        let recent = self
            .db
            .recent_centroids(now.saturating_sub(tuning::SCORE_NOVELTY_WINDOW_SECS))?;
        let voted = self
            .db
            .voted_centroids(now.saturating_sub(tuning::SCORE_VOTE_WINDOW_SECS))?;
        let topics_unmuted: Vec<(String, Vec<f32>, bool)> = topics
            .iter()
            .filter(|(_, _, muted)| !*muted)
            .cloned()
            .collect();
        let mut after_id = 0;
        loop {
            let page = self
                .db
                .score_states_page(after_id, tuning::SCORE_PAGE_SIZE)?;
            if page.is_empty() {
                break;
            }
            after_id = page.last().map_or(after_id, |story| story.id);
            for state in &page {
                let scored = score_story(state, &topics, &recent, &voted, now);
                let (_, topic_name) = score::best_topic(&state.centroid, &topics_unmuted);
                let label = if scored.topic_match {
                    topic_name.map(str::to_owned)
                } else {
                    state
                        .members
                        .iter()
                        .filter(|member| member.copy_of.is_none())
                        .max_by(|a, b| {
                            a.uniqueness
                                .total_cmp(&b.uniqueness)
                                .then_with(|| b.seen_at.cmp(&a.seen_at))
                        })
                        .map(|member| {
                            member
                                .folder
                                .as_deref()
                                .unwrap_or(&member.outlet)
                                .to_owned()
                        })
                };
                let reason = scored
                    .discover_reason
                    .map(score::DiscoverReason::as_str)
                    .unwrap_or("");
                self.db.apply_score(
                    state.id,
                    scored.for_you,
                    scored.discover,
                    reason,
                    label.as_deref(),
                    scored.topic_match,
                    scored.muted,
                    scored.input.coverage,
                )?;
            }
        }
        Ok(())
    }
}

fn story_candidates(stories: &[StoryState]) -> Vec<StoryCandidate> {
    stories
        .iter()
        .map(|story| StoryCandidate {
            id: story.id,
            centroid: story.centroid.clone(),
            first_seen_at: story.first_seen_at,
            updated_at: story.updated_at,
            closed: story.closed,
            facts: story
                .members
                .iter()
                .filter(|member| member.copy_of.is_none())
                .flat_map(|member| facts::fact_tokens(&member.text))
                .collect(),
        })
        .collect()
}

fn score_story(
    story: &StoryState,
    topics: &[(String, Vec<f32>, bool)],
    recent: &[(i64, Vec<f32>)],
    voted: &[(i64, Vec<f32>, i8)],
    now: i64,
) -> ScoredStory {
    let input = score_input(story, topics, recent, voted, now);
    let result = score::score(input);
    let topic_match = input.topic_sim >= score::TOPIC_MATCH;
    let ids: Vec<i64> = story
        .members
        .iter()
        .filter(|member| member.copy_of.is_none())
        .map(|member| member.id)
        .collect();
    ScoredStory {
        story_id: story.id,
        for_you: result.for_you,
        discover: result.discover,
        for_you_eligible: result.for_you_eligible,
        discover_eligible: result.discover_eligible,
        muted: input.muted,
        topic_match,
        discover_reason: result.discover_reason,
        input,
        updated_at: story.updated_at,
        text_hash: ids_hash(&sorted_ids(ids)),
        centroid: story.centroid.clone(),
    }
}

fn score_input(
    story: &StoryState,
    topics: &[(String, Vec<f32>, bool)],
    recent: &[(i64, Vec<f32>)],
    voted: &[(i64, Vec<f32>, i8)],
    now: i64,
) -> ScoreInput {
    let active_topics: Vec<(String, Vec<f32>, bool)> = topics
        .iter()
        .filter(|(_, _, muted)| !*muted)
        .cloned()
        .collect();
    let (topic_sim, _) = score::best_topic(&story.centroid, &active_topics);
    let coverage = story
        .members
        .iter()
        .filter(|member| member.copy_of.is_none())
        .map(|member| member.feed_id)
        .collect::<HashSet<_>>()
        .len();
    let originals: Vec<&MemberArticle> = story
        .members
        .iter()
        .filter(|member| member.copy_of.is_none())
        .collect();
    let muted_topic_embeddings: Vec<Vec<f32>> = topics
        .iter()
        .filter(|(_, _, muted)| *muted)
        .map(|(_, embedding, _)| embedding.clone())
        .collect();
    let feed_muted: Vec<bool> = originals.iter().map(|member| member.feed_muted).collect();
    let muted = score::is_muted(&story.centroid, &muted_topic_embeddings, &feed_muted);
    let voted_vectors: Vec<(&[f32], i8)> = voted
        .iter()
        .filter(|(id, _, _)| *id != story.id)
        .map(|(_, embedding, vote)| (embedding.as_slice(), *vote))
        .collect();
    let taste = score::taste(&story.centroid, &voted_vectors);
    let recent_vectors: Vec<Vec<f32>> = recent
        .iter()
        .filter(|(id, _)| *id != story.id)
        .map(|(_, centroid)| centroid.clone())
        .collect();
    let novelty = score::novelty(&story.centroid, &recent_vectors);
    let detail = originals
        .iter()
        .map(|member| member.uniqueness)
        .reduce(f32::max)
        .unwrap_or(0.0);
    let age_h = now.saturating_sub(story.updated_at).max(0) as f32 / 3600.0;
    ScoreInput {
        topic_sim,
        taste,
        coverage,
        age_h,
        novelty,
        detail,
        vote: story.vote as i8,
        muted,
    }
}

fn topic_tuples(topics: &[db::EmbeddedTopic]) -> Vec<(String, Vec<f32>, bool)> {
    topics
        .iter()
        .filter(|topic| !topic.embedding.is_empty())
        .map(|topic| (topic.name.clone(), topic.embedding.clone(), topic.muted))
        .collect()
}

fn sorted_ids(mut ids: Vec<i64>) -> Vec<i64> {
    ids.sort_unstable();
    ids
}

fn ids_hash(ids: &[i64]) -> String {
    let mut digest = Sha256::new();
    for id in ids {
        digest.update(id.to_le_bytes());
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ArticleInput, SentenceInput};
    use std::time::Duration;

    #[test]
    fn fixture_feeds_flow_through_real_static_embeddings_into_stories_and_copies() {
        let db = Db::memory().unwrap();
        let now = unix_now();
        let config = Arc::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            data_dir: std::env::temp_dir(),
            tz: "UTC".into(),
            retention_days: 90,
            min_score: 40,
            fetch_concurrency: 16,
            fetch_min_interval: Duration::from_secs(900),
            fetch_max_interval: Duration::from_secs(21600),
            opml_seed: None,
            topics_seed: Vec::new(),
            model_url: "https://example.test/model".into(),
            llm_base_url: None,
            llm_api_key: None,
            llm_model: None,
            llm_concurrency: 2,
            log: "info".into(),
        });
        db.add_topic("housing", &[], false, now).unwrap();
        let original = "Housing planners approved 3,200 homes in River District today. Housing residents will receive the new homes in the first phase of construction.";
        let rewrite = "Housing councillors confirmed 3,200 homes after reviewing the River District proposal. Housing garden projects include 180 new garden plots for local residents.";
        let harbour = "Harbour crews will replace the northern seawall after the storm. Harbour repairs will cost 45 million dollars and begin next month.";
        for (index, (title, text)) in [
            ("Housing development approved", original),
            ("Housing development approved", original),
            ("Housing garden proposal adds local detail", rewrite),
            ("Harbour seawall repairs", harbour),
        ]
        .iter()
        .enumerate()
        {
            let feed = db
                .add_feed(&format!("https://fixture-{index}.example.test/feed"))
                .unwrap();
            let xml = format!(
                "<rss version=\"2.0\"><channel><title>Fixture {index}</title><link>https://fixture.example.test/</link><description>Offline pipeline fixture</description><item><guid>article-{index}</guid><title>{title}</title><link>https://fixture.example.test/{index}</link><description><![CDATA[<p>{text}</p>]]></description></item></channel></rss>"
            );
            let parsed = crate::fetch::parse_feed_body(xml.as_bytes(), None, None).unwrap();
            let item = parsed.items.into_iter().next().unwrap();
            let clean = crate::extract::normalize_text(&crate::extract::strip_html(
                item.content.as_deref().unwrap(),
            ));
            let sentences = crate::extract::split_sentences(&clean)
                .into_iter()
                .enumerate()
                .map(|(position, text)| SentenceInput {
                    position: position as i64,
                    text,
                    embedding: Vec::new(),
                })
                .collect();
            db.insert_article(&ArticleInput {
                feed_id: feed,
                guid: item.guid,
                url: item.url.unwrap(),
                title: item.title,
                published_at: None,
                seen_at: now + index as i64,
                text: Some(clean),
                text_source: "feed".into(),
                embedding: None,
                minhash: None,
                sentences,
            })
            .unwrap();
        }
        // An unembeddable article must not prevent processing later feed items.
        let feed = db.add_feed("https://empty.example.test/feed").unwrap();
        db.insert_article(&ArticleInput {
            feed_id: feed,
            guid: "empty".into(),
            url: "https://empty.example.test/item".into(),
            title: String::new(),
            published_at: None,
            seen_at: now - 1,
            text: Some(String::new()),
            text_source: "feed".into(),
            embedding: None,
            minhash: None,
            sentences: Vec::new(),
        })
        .unwrap();
        let pipeline = Pipeline::new(
            db.clone(),
            config,
            Arc::new(Embedder::fixture_model().unwrap()),
        )
        .unwrap();
        pipeline.process().unwrap();
        let stories = db.story_states(now - 3600).unwrap();
        assert_eq!(stories.len(), 2);
        let housing = stories
            .iter()
            .find(|story| story.members.len() == 3)
            .unwrap();
        assert_eq!(
            housing
                .members
                .iter()
                .filter(|member| member.copy_of.is_some())
                .count(),
            1
        );
        let originals: HashSet<i64> = housing
            .members
            .iter()
            .filter(|m| m.copy_of.is_none())
            .map(|m| m.id)
            .collect();
        assert_eq!(originals.len(), 2);
        let page = db.story_page(housing.id, None, now + 10).unwrap().unwrap();
        assert_eq!(page.story.coverage, 2);
        assert!(!page.sentences.is_empty());
        assert_eq!(page.sources.len(), 2);
        assert_eq!(page.text_mode, "extractive");
        let stored = db.text_candidates(housing.id).unwrap();
        assert!(
            stored
                .iter()
                .all(|sentence| originals.contains(&sentence.article_id))
        );
        assert!(
            page.sentences
                .iter()
                .all(|sentence| !sentence.text.is_empty())
        );
        assert!(
            page.sentences
                .iter()
                .any(|sentence| sentence.text.contains("garden"))
        );
        assert_eq!(db.story_list("for-you", None, 40).unwrap().len(), 1);
        assert_eq!(db.story_list("discover", None, 40).unwrap().len(), 1);
        assert_eq!(
            db.story_list("search", Some("Housing"), 40).unwrap().len(),
            1
        );
        let hash = housing.text_built_for.clone();
        pipeline.process().unwrap();
        let after = db.story_states(now - 3600).unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(
            after
                .iter()
                .find(|s| s.id == housing.id)
                .unwrap()
                .text_built_for,
            hash
        );
        assert_eq!(db.pending_articles_after(10, None, None).unwrap().len(), 1);
    }
}
