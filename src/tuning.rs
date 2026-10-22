//! Fixed timing and text limits used by the ingestion pipeline.

/// Maximum wall clock time for a single feed or article request.
pub const FETCH_TIMEOUT_SECS: u64 = 20;
/// Maximum number of redirects followed for an HTTP request.
pub const FETCH_MAX_REDIRECTS: usize = 5;
/// Maximum decompressed body size accepted from a feed or article page.
pub const FETCH_MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
/// Per-host request limit shared by feed and article downloads.
pub const FETCH_PER_HOST_CONCURRENCY: usize = 2;
/// Maximum retry delay after repeated fetch failures.
pub const FETCH_MAX_BACKOFF_SECS: u64 = 24 * 60 * 60;
/// Fractional jitter bound applied to fetch retry delays.
pub const FETCH_BACKOFF_JITTER: f64 = 0.10;
/// Default lower bound for adaptive feed polling; the environment may override it.
pub const FETCH_MIN_INTERVAL_SECS: u64 = 15 * 60;
/// Default upper bound for adaptive feed polling; the environment may override it.
pub const FETCH_MAX_INTERVAL_SECS: u64 = 6 * 60 * 60;
/// Number of days used to estimate a feed's publishing rate.
pub const FETCH_RATE_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;
/// Number of days of feed history ignored when initially importing a feed.
pub const FEED_INITIAL_IMPORT_AGE_SECS: u64 = 7 * 24 * 60 * 60;
/// Minimum extracted page length before falling back to feed-provided content.
pub const ARTICLE_FALLBACK_MIN_CHARS: usize = 300;
/// Maximum line length still considered for short-line boilerplate removal.
pub const BOILERPLATE_MAX_CHARS: usize = 25;
/// Minimum sentence length retained for downstream processing.
pub const SENTENCE_MIN_CHARS: usize = 25;
/// Maximum sentence length retained for downstream processing.
pub const SENTENCE_MAX_CHARS: usize = 400;
/// Interval between background checks for due feeds.
pub const SCHEDULER_TICK_SECS: u64 = 60;

/// Initial story-joining threshold; calibrated evaluation can update this value.
pub const JOIN_THRESHOLD: f32 = 0.80;
/// Maximum age of a story that can accept new articles.
pub const STORY_MAX_AGE_SECS: i64 = 10 * 24 * 60 * 60;
/// Inactivity after which a story stops accepting new articles.
pub const STORY_IDLE_SECS: i64 = 72 * 60 * 60;
/// Candidate stories must have an original update within this window.
pub const STORY_CANDIDATE_WINDOW_SECS: i64 = 72 * 60 * 60;
/// Time decay sigma, in hours, for the online story assignment score.
pub const STORY_TIME_SIGMA_HOURS: f64 = 72.0;
/// Number of word tokens included in article embeddings.
pub const ARTICLE_EMBED_WORDS: usize = 120;
/// Maximum model download request duration.
pub const MODEL_DOWNLOAD_TIMEOUT_SECS: u64 = 300;
/// Number of shingle hashes stored in MinHash signatures.
pub const MINHASH_SIZE: usize = 128;
/// Minimum estimated Jaccard before exact copy containment is calculated.
pub const COPY_CANDIDATE_JACCARD: f32 = 0.50;
/// Jaccard threshold for declaring a near-duplicate copy.
pub const COPY_JACCARD_THRESHOLD: f32 = 0.80;
/// Containment threshold for recognizing syndicated text with added material.
pub const COPY_CONTAINMENT_THRESHOLD: f32 = 0.80;
/// Calibration status; replace after the SemEval evaluation is run.
pub const JOIN_THRESHOLD_CALIBRATION: &str = "uncalibrated: potion-base-8M; eval pending";
/// Maximum size of one model file downloaded during startup.
pub const MODEL_MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;

/// Maximum sentence candidates sent to the optional LLM.
pub const LLM_MAX_SOURCE_SENTENCES: usize = 40;
/// Maximum sentence candidates selected from any one source article.
pub const LLM_MAX_SENTENCES_PER_SOURCE: usize = 5;
/// Maximum output sentences accepted from an LLM response.
pub const LLM_MAX_OUTPUT_SENTENCES: usize = 10;
/// Maximum decoded LLM response size, including its JSON envelope.
pub const LLM_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// LLM verification embedding similarity floor.
pub const LLM_MIN_VERIFICATION_SIMILARITY: f32 = 0.75;
/// OpenAI-compatible request timeout.
pub const LLM_REQUEST_TIMEOUT_SECS: u64 = 60;
/// Story rewrite sampling temperature.
pub const LLM_TEMPERATURE: f64 = 0.2;
/// Number of retries after invalid model story JSON.
pub const LLM_INVALID_JSON_RETRIES: usize = 1;

/// Extractive text starts with this many sentences from the highest-uniqueness source.
pub const STORYTEXT_LEDE_SENTENCES: usize = 2;
/// Maximum sentence count in an extractive story.
pub const STORYTEXT_MAX_SENTENCES: usize = 12;
/// Maximum relevance for a sentence to enter the MMR selection.
pub const STORYTEXT_MIN_RELEVANCE: f32 = 0.5;
/// Similarity at which a candidate is considered redundant.
pub const STORYTEXT_MAX_REDUNDANCY: f32 = 0.85;
/// MMR relevance multiplier.
pub const STORYTEXT_RELEVANCE_WEIGHT: f32 = 0.6;
/// MMR redundancy multiplier.
pub const STORYTEXT_REDUNDANCY_WEIGHT: f32 = 0.4;
/// Number of words included in the opening side of a source fragment.
pub const STORYTEXT_FRAGMENT_FIRST_WORDS: usize = 6;
/// Number of words included in the closing side of a source fragment.
pub const STORYTEXT_FRAGMENT_LAST_WORDS: usize = 4;

/// Topic similarity required for For you eligibility.
pub const SCORE_TOPIC_MATCH: f32 = 0.70;
/// Muted topic similarity required to hide a story.
pub const SCORE_MUTE_THRESHOLD: f32 = 0.75;
/// For you topic similarity coefficient.
pub const SCORE_FOR_YOU_TOPIC_WEIGHT: f32 = 3.0;
/// For you taste coefficient.
pub const SCORE_FOR_YOU_TASTE_WEIGHT: f32 = 2.0;
/// For you coverage coefficient.
pub const SCORE_FOR_YOU_COVERAGE_WEIGHT: f32 = 0.4;
/// For you age penalty per hour.
pub const SCORE_FOR_YOU_AGE_PENALTY: f32 = 0.03;
/// For you score sigmoid steepness.
pub const SCORE_FOR_YOU_SIGMOID_STEEPNESS: f32 = 4.0;
/// For you score sigmoid midpoint.
pub const SCORE_FOR_YOU_SIGMOID_MIDPOINT: f32 = 2.2;
/// Discover coverage coefficient.
pub const SCORE_DISCOVER_COVERAGE_WEIGHT: f32 = 1.5;
/// Discover novelty coefficient.
pub const SCORE_DISCOVER_NOVELTY_WEIGHT: f32 = 2.0;
/// Discover age penalty per hour.
pub const SCORE_DISCOVER_AGE_PENALTY: f32 = 0.02;
/// Discover score sigmoid steepness.
pub const SCORE_DISCOVER_SIGMOID_STEEPNESS: f32 = 3.0;
/// Discover score sigmoid midpoint.
pub const SCORE_DISCOVER_SIGMOID_MIDPOINT: f32 = 2.0;
/// Highest source coverage considered widely covered.
pub const SCORE_WIDELY_COVERED_FEEDS: f32 = 6.0;
/// Novelty needed to label Discover reason as unusual.
pub const SCORE_UNUSUAL_NOVELTY: f32 = 0.35;
/// Maximum displayed ranking score.
pub const SCORE_MAX: f32 = 99.0;

/// Pending articles loaded per keyset page by the pipeline.
pub const PIPELINE_ARTICLE_BATCH_SIZE: i64 = 50;
/// Story metadata loaded per score or invalidation page.
pub const SCORE_PAGE_SIZE: i64 = 250;
/// Recent story window used to estimate novelty.
pub const SCORE_NOVELTY_WINDOW_SECS: i64 = 7 * 24 * 60 * 60;
/// Recent vote window used to estimate taste.
pub const SCORE_VOTE_WINDOW_SECS: i64 = 90 * 24 * 60 * 60;
/// Discover candidates are eligible for generated text for this duration.
pub const DISCOVER_WINDOW_SECS: i64 = 48 * 60 * 60;

/// Delay before retrying a failed local embedding-model initialization.
pub const MODEL_RETRY_INTERVAL_SECS: u64 = 300;
/// Background score decay and story-closure refresh cadence.
pub const SCORE_REFRESH_INTERVAL_SECS: u64 = 3600;
/// Retention and incremental-vacuum cadence, independent of model availability.
pub const RETENTION_INTERVAL_SECS: i64 = 24 * 3600;
