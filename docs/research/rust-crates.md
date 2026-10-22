# Rust crate research for an RSS reader

Research date: 2026-10-22. Scope: web research only, no code written.

**Method and limits.** Versions, release dates, licenses, MSRV, and release counts come from the crates.io API (https://crates.io/api/v1/crates/NAME and /versions). Version numbers for HTMX come from the npm registry (https://registry.npmjs.org/htmx.org). Feature and API details come from docs.rs and project READMEs. GitHub was not reachable from this session (api.github.com and github.com both returned 403 for every repo tested), so commit activity, open issue counts, and star counts are **UNVERIFIED** for every project. Maintenance is judged here from crates.io release history and download counts only. "Releases last 12 mo" counts non-yanked versions published since 2025-10-22.

---

## 1. feed-rs (RSS, Atom, JSON Feed parsing)

| Field | Value |
|---|---|
| Latest | 3.0.0 (released 2026-09-27) |
| Previous | 2.4.0 (2026-07-07), 2.3.1 (2024-12-25) |
| License | MIT |
| MSRV | UNVERIFIED (not stated on docs.rs) |
| Releases last 12 mo | 2 |
| Downloads | about 2.5M total |
| Maintenance | Active. Recent 3.0.0 major release, so check the changelog before pinning. |

**Notable limitations**
- Documented formats: Atom, RSS 2.0, RSS 1.0, RSS 0.x, JSON Feed. Parser auto-detects XML vs JSON and Atom vs RSS.
- `<ttl>` and `sy:updatePeriod` are not mentioned in the docs. Support is **UNVERIFIED**. Plan to read these from the raw XML if needed.
- Raw extension elements are not mentioned in the docs. Access is **UNVERIFIED**.
- RSS `description` and Atom `subtitle` are merged into one field, so the model does not separate summary from content.
- Parsing is lenient: many fields are `Option<T>` even where the spec requires them.
- Dates use lenient parsing by default.
- Sanitisation is opt-in (`parser::Builder::sanitize_content(true)`).
- Only about 51.5% of the crate is documented on docs.rs.
- 3.0.0 breaking changes are **UNVERIFIED** (changelog not read).

**Alternative if needed:** Parse `<ttl>`, `sy:*`, and any other extensions with a small quick-xml pass over the raw bytes. Not evaluated further.

**Sources:** https://docs.rs/feed-rs/latest/feed_rs/ ; https://crates.io/crates/feed-rs

---

## 2. Readability-style full-text extraction

| Crate | Latest | Released | License | Releases last 12 mo | MSRV | Assessment |
|---|---|---|---|---|---|---|
| **dom_smoothie** | 0.18.2 | 2026-09-21 | MIT | 7 | 1.75 | Active. Port of Mozilla Readability.js. About 740k downloads. Best option. |
| readability (kumabook) | 0.3.0 | 2023-12-20 | MIT | 0 | not stated | Stale (about 2.8 years). Failed two of three pages in the 2025 comparison below. |
| readability-rs (quambene) | 0.5.0 | 2024-12-18 | MIT | 0 | not stated | Stale, about 7k downloads. Not in the 2025 comparison. |
| readable-readability | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | Used `kuchiki`. Non-empty output on HN, but failed on the GitHub page. |
| llm_readability | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED | Empty output on two of three pages in the 2025 comparison. |

**Which is best maintained and most accurate:** dom_smoothie. It is the only actively released readability-style crate. In the 2025 comparison it was the only readability crate judged to find the main text on all three test pages. That evidence is thin: three pages, manual judgement, dated 2025-01-21, and before the 0.17 and 0.18 releases.

**dom_smoothie notes (from its README)**
- API: `Readability::new(html, url, Option<Config>)`, then `parse()` returns an `Article` (title, byline, excerpt, site name, dates, image, content, text_content). Also `is_probably_readable()`, `parse_json_ld()`, and `parse_with_policy(Strict | Moderate | Clean | Raw)`.
- Config includes `text_mode` (Formatted or Markdown), `max_elements_to_parse`, `readable_min_score`, and `classes_to_preserve`.
- Differences from Readability.js: relative `href` values are left as they are; `<font>` becomes `<span>`; empty `<a>` removed; `class="page"` kept only on the article node; `text_content` can run words together; `Formatted` mode does not keep table structure; `Strict` can fail with `GrabFailed` where `parse()` succeeds.
- The README says the alternative candidate-selection mode has "room for improvement."
- Dependencies include dom_query 0.28, gjson 0.8, and unicode-segmentation (crates.io).

**Recommendation to validate:** Build a corpus of 50 to 100 real article URLs from your own feeds, and compare dom_smoothie with its `Markdown` text mode before committing.

**Sources:** https://github.com/niklak/dom_smoothie (README via WebFetch); https://docs.rs/dom_smoothie ; https://emschwartz.me/comparing-13-rust-crates-for-extracting-text-from-html/ (21 Jan 2025 comparison of 13 crates on three pages); https://crates.io/api/v1/crates/readability ; https://crates.io/api/v1/crates/readability-rs

---

## 3. Local embeddings: fastembed and model2vec

### 3a. fastembed (fastembed-rs)

| Field | Value |
|---|---|
| Latest | 7.2.0 (released 2026-10-21) |
| License | Apache-2.0 |
| MSRV | 1.88 |
| Releases last 12 mo | 44 |
| Downloads | about 4.5M total |
| Key dependency | `ort = 2.0.0-rc.15` (a release-candidate pin on the ONNX Runtime wrapper, a stability risk) |
| Other deps | tokenizers ^0.23.2, safetensors ^0.8.0, ndarray ^0.17.2 |

**English models in the `EmbeddingModel` enum** (docs.rs, 7.2.0). The page does not list dimensions except where noted.

| Variant | Model | Dim | Notes |
|---|---|---|---|
| `BGESmallENV15` | BAAI/bge-small-en-v1.5 | 384 (confirmed by docs example) | Marked as the default. Small. |
| `BGESmallENV15Q` | quantized bge-small-en-v1.5 | 384 (from model card, not re-checked) | Quantized, smaller and faster. |
| `AllMiniLML6V2` / `Q` | sentence-transformers/all-MiniLM-L6-v2 | 384 (model card, not re-checked) | Small, classic. |
| `AllMiniLML12V2` / `Q` | all-MiniLM-L12-v2 | 384 (model card, not re-checked) | Slightly larger. |
| `BGEBaseENV15` / `Q` | bge-base-en-v1.5 | 768 (model card, not re-checked) | Mid size. |
| `NomicEmbedTextV15` / `Q` | nomic-ai/nomic-embed-text-v1.5 | 768 (model card, not re-checked) | Long context. |
| `GTEBaseENV15` / `Q`, `GTELargeENV15` / `Q` | Alibaba-NLP gte-*-en-v1.5 | UNVERIFIED | Larger. |
| `MxbaiEmbedLargeV1` / `Q` | mixedbread-ai/mxbai-embed-large-v1 | UNVERIFIED | Large. |
| `JinaEmbeddingsV2BaseEN` | jinaai/jina-embeddings-v2-base-en | UNVERIFIED | Base. |
| `SnowflakeArcticEmbedXS` / `S` (and `Q`) | Snowflake arctic-embed | UNVERIFIED | Language coverage not stated on the page. |
| `EmbeddingGemma300M` (and `Q`, `Q4`) | Google EmbeddingGemma | UNVERIFIED | Listed by docs as multilingual or unspecified, not English-only. |

**Feature flags (7.2.0)**
- `default` = `ort-download-binaries-native-tls` + `hf-hub-native-tls` + `image-models`.
- `ort-download-binaries` (and `-rustls-tls`): downloads prebuilt ONNX Runtime binaries.
- `ort-load-dynamic`: load a shared library at runtime instead.
- `online` = `hf-hub-native-tls`.
- Others: `directml`, `cuda`, `cudnn`, `metal`, `accelerate`, `mkl`, `webgpu`, `qwen3`, `nomic-v2-moe`.

**First-run model download**
- Models download on first use from Hugging Face and load from cache afterwards. No network is needed at runtime once cached.
- Cache directory: `FASTEMBED_CACHE_DIR`, default `.fastembed_cache`. `HF_HOME`, if set, takes precedence.
- `HF_ENDPOINT` sets a Hugging Face mirror.
- Offline use: `try_new_from_user_defined(...)` loads model files from disk.

**Binary size and ONNX Runtime linkage:** The docs do not say whether the runtime is bundled into the binary or needs a shared library. Binary size is **UNVERIFIED**. The ort documentation page was not retrievable (client error), so the exact download-binaries behavior is **UNVERIFIED**.

**CPU throughput:** No benchmark numbers in the fastembed docs. **UNVERIFIED**. Measure on target hardware.

**Sources:** https://docs.rs/fastembed/latest/fastembed/enum.EmbeddingModel.html ; https://github.com/Anush008/fastembed-rs (README via WebFetch) ; https://crates.io/api/v1/crates/fastembed/7.2.0 (features and deps)

### 3b. model2vec-rs (static embeddings, lighter alternative)

| Field | Value |
|---|---|
| Latest | 0.3.0 (released 2026-09-19) |
| License | MIT per README. The crates.io `license` field says "non-standard", so confirm against the LICENSE file. |
| MSRV | 1.88 |
| Releases last 12 mo | 3 |
| Downloads | about 287k total |

**Pretrained models (English unless noted)**

| Model | Params | Notes |
|---|---|---|
| potion-base-8M | 7.5M | Distilled from bge-base-en-v1.5. General. |
| potion-base-4M | 3.7M | General. |
| potion-base-2M | 1.8M | General. |
| potion-base-32M | 32.3M | Distilled from bge-base-en-v1.5. General. |
| potion-retrieval-32M | 32.3M | Retrieval-tuned. |
| potion-multilingual-128M | 128M | Multilingual, distilled from bge-m3. |
| potion-code-16M | 16M | Code. |

Embedding dimensions are not on the docs page: **UNVERIFIED**.

**Speed claim (README, not verified here):** about 8,000 samples per second, single-threaded CPU, Rust, versus 4,650 in Python (about 1.7x).

**API:** `StaticModel::from_pretrained(repo_or_path, hf_token, normalize, subfolder)`, `from_bytes(...)` for in-memory use, `encode(&sentences)`, and `encode_with_args(&sentences, max_len, batch_size)` (defaults: max length 512, batch size 1024). Feature flags: `default`, `fancy-regex`, `local-only`, `wasm`. Weights in f32, f16, or i8 safetensors.

**Sources:** https://github.com/MinishLab/model2vec-rs (README via WebFetch); https://crates.io/api/v1/crates/model2vec-rs

---

## 4. Near-duplicate detection

| Crate | Latest | Released | License | Releases last 12 mo | Downloads | Relevant API | Assessment |
|---|---|---|---|---|---|---|---|
| **gaoya** | 0.2.2 | 2026-06-15 | MIT | 1 | about 191k | `MinHasher32`, `MinHashIndex` (num_bands, band_width, threshold), `query_owned`. SimHash exists but its index API is undocumented. | Only LSH-index option found. Only about 14% of the crate is documented. Threshold semantics are unclear from the docs. |
| probminhash | 0.1.12 | 2025-06-11 | MIT OR Apache-2.0 | 0 | about 2M | ProbMinHash, SuperMinHash, SetSketch, DenseMinHash (weighted Jaccard sketching) | Sketching only, no LSH index. No release in over a year. |
| simhash | 0.3.0 | 2026-03-09 | MIT | 1 | about 1.7M | `simhash`, `hamming_distance`, `hash_similarity` on `u64` | Pairwise only. No feature weighting or index documented. |

**Recommendation:** Use gaoya MinHash with banding, and verify its threshold behavior with unit tests on known duplicate pairs. Writing your own MinHash plus banding is a reasonable fallback, around 100 lines. Brute-force pairwise comparison of 20k articles is about 2 x 10^8 pairs, so banding (LSH) is needed rather than an all-pairs scan.

**Sources:** https://docs.rs/gaoya/latest/gaoya/ ; https://docs.rs/probminhash/latest/probminhash/ ; https://docs.rs/simhash/latest/simhash/

---

## 5. SQLite: storage and vector search

| Crate | Latest | Released | License | Releases last 12 mo | Notes |
|---|---|---|---|---|---|
| sqlx | 0.9.0 | 2026-05-21 | MIT OR Apache-2.0 | 1 | Async. Compile-time checked queries need `DATABASE_URL` or offline data (`sqlx prepare`). SQLite via `libsqlite3-sys`. `sqlite` feature bundles SQLite. |
| rusqlite | 0.40.2 | 2026-08-08 | MIT | 5 | Synchronous. `bundled` feature statically compiles SQLite. About 132M downloads. |
| sqlite-vec (Rust bindings) | 0.1.9 (crate); 0.1.10-alpha.4 latest pre-release (2026-05-18) | 0.1.9 stable crate | MIT/Apache-2.0 | 20 pre-release versions in 12 mo | Docs.rs is 0% documented. Project docs say the extension is alpha and "work-in-progress." Registration via `sqlite3_vec_init` only documented in the crate listing. |

**sqlx details:** The SQLite driver calls the C library, so it is not pure Rust. The crate forbids unsafe code unless the `sqlite` feature is enabled. Compile-time query macros add build-time database or offline-file requirements.

**Do you need sqlite-vec at about 20k vectors?** Probably not. At 384 dimensions with f32, 20,000 vectors is about 30.7 MB. A brute-force cosine scan in Rust is a few milliseconds per query on modern hardware. That estimate is **UNVERIFIED** (no benchmark run). Store vectors as BLOBs and compute similarity in Rust. Add sqlite-vec only if the corpus grows by an order of magnitude or you need ANN indexes.

**Recommendation:** rusqlite with the `bundled` feature. Run queries inside `tokio::task::spawn_blocking` or behind a pool crate. Pool crates were not evaluated (UNVERIFIED).

**Sources:** https://github.com/launchbadge/sqlx (README via WebFetch); https://docs.rs/sqlite-vec/latest/sqlite_vec/ ; https://alexgarcia.xyz/sqlite-vec/ ; https://crates.io/api/v1/crates/rusqlite

---

## 6. Web layer: axum, templates, HTMX

| Item | Latest | Released | License | MSRV | Releases last 12 mo | Notes |
|---|---|---|---|---|---|---|
| axum | 0.8.9 | 2026-04-14 | MIT | 1.80 | 3 | Stable 0.8 line. No 0.9 release seen in crates.io history. About 536M downloads. |
| askama | 0.16.1 | 2026-09-04 | MIT OR Apache-2.0 | 1.88 | 9 | Compile-time, Jinja-like. Type-checked templates. Active. |
| minijinja | 3.0.2 | 2026-10-21 | Apache-2.0 | 1.70 | 21 | Runtime templates (can reload without recompiling). 3.0.0 released 2026-10-08. Newer major, more churn. |
| maud | 0.27.0 | 2025-02-02 | MIT OR Apache-2.0 | not stated | 0 | Compile-time HTML macros. No release in about 20 months. Stale. |
| HTMX | 2.0.11 (npm `latest`, 2026-09-22) | 2026-09-22 | BSD-2-Clause (UNVERIFIED for the 2.x line) | n/a | n/a | npm `next` tag is 4.0.0 (2026-08-28). Pin 2.0.11 until 4.x is stable. |

**Recommendation:** axum 0.8.9 with askama 0.16.1 and HTMX 2.0.11. Integration glue between askama and axum (`IntoResponse`) was not verified against 0.16 docs (**UNVERIFIED**). If you want to edit templates without recompiling, pick minijinja instead.

**Sources:** https://crates.io/api/v1/crates/axum ; https://crates.io/api/v1/crates/askama ; https://crates.io/api/v1/crates/minijinja ; https://crates.io/api/v1/crates/maud ; https://registry.npmjs.org/htmx.org

---

## 7. LLM client (OpenAI-compatible, custom base URL)

| Crate | Latest | Released | License | Releases last 12 mo | Custom base URL | Assessment |
|---|---|---|---|---|---|---|
| async-openai | 0.42.2 | 2026-10-10 | MIT | 44 | Yes. `OpenAIConfig::new().with_api_base(url).with_api_key(key)` (docs.rs verified). Also reads `OPENAI_BASE_URL`. | Typed, complete OpenAI surface. Uses reqwest 0.13 and tokio. High release churn. Heavier API surface than needed. |
| genai | 0.7.0 | 2026-10-20 | MIT OR Apache-2.0 | 76 (incl. pre-releases) | Not shown on docs.rs. Adapters list includes Ollama, OpenAI, OpenRouter-style providers (UNVERIFIED for base-URL override method). | Multi-provider. Very high release churn. |
| rig-core | 0.45.0 | 2026-10-19 | MIT | 25 | Not shown on docs.rs. Providers listed include OpenAI, Anthropic, Cohere, Gemini, Perplexity. | Full LLM application framework (agents, vector stores). Heavier than needed for a single call type. Rust 1.97.1 MSRV. |
| reqwest + serde (plain) | 0.13.5 | 2026-09-08 | MIT OR Apache-2.0 | n/a | Trivially yes. You own the base URL. | Lightest and most stable. Roughly 150 lines for chat and embeddings. |

**Recommendation:** Plain reqwest 0.13 with serde, calling `POST {base}/chat/completions` and `POST {base}/embeddings`. This works with any OpenAI-compatible server (Ollama, OpenRouter, llama.cpp server) by changing only the base URL. These endpoint paths are the common OpenAI-compatible convention (**UNVERIFIED** against each server's current docs). Choose async-openai instead if you want typed request structs and accept its churn.

**Sources:** https://docs.rs/async-openai/latest/async_openai/config/struct.OpenAIConfig.html ; https://docs.rs/genai/latest/genai/ ; https://docs.rs/rig-core/latest/rig/ ; https://crates.io/api/v1/crates/reqwest

---

## 8. OPML parsing

| Crate | Latest | Released | License | Releases last 12 mo | Notes |
|---|---|---|---|---|---|
| opml | 1.1.6 | 2024-01-21 | MIT OR Apache-2.0 | 0 | About 1,270 downloads per month (lib.rs). Supports OPML 1.0, 1.1, 2.0. `OPML::from_str` and `from_reader`. Size about 0.7 to 1.4 MB, about 31k SLoC on lib.rs. |

**Limitations:** The docs do not describe how nested outlines or `xmlUrl` are exposed, or whether serialization is provided. These are **UNVERIFIED**. Test with a real export from Miniflux or FreshRSS before relying on it.

**Alternative:** Parse OPML directly with quick-xml (about 50 lines). Not evaluated further. OPML is a small, stable format, so a hand-written parser is a reasonable choice.

**Sources:** https://docs.rs/opml/latest/opml/ ; https://lib.rs/crates/opml

---

## 9. Feed scheduling and polling

### 9a. Crates for adaptive polling
No dedicated crate for per-feed adaptive polling was verified. `adaptive_backoff` appears on docs.rs but was not evaluated (**UNVERIFIED**). Plan to implement the scheduler in-house.

### 9b. Miniflux (Go)

- **Global batch:** `POLLING_FREQUENCY` default 60 minutes. `BATCH_SIZE` default 100 feeds per interval. `WORKER_POOL_SIZE` default 16.
- **Scheduler modes** (`POLLING_SCHEDULER`, default `round_robin`):
  - `round_robin`: fixed rotating order. Interval bounds: min 60 minutes, max 1440 minutes. The max applies when `Cache-Control` max-age and `Expires` are very high.
  - `entry_frequency`: each feed's interval is based on its average update rate over the past week. Bounds: min 5 minutes, max 1440 minutes. `SCHEDULER_ENTRY_FREQUENCY_FACTOR` default 1.
- **Errors:** `POLLING_PARSING_ERROR_LIMIT` default 3. After that, polling stops and "the user must refresh the feed manually." Set 0 for unlimited.
- **Per-host cap:** `POLLING_LIMIT_PER_HOST` default 0 (disabled).
- **Conditional GET:** A commit titled "fetcher: use ETag as a stronger validator than Last-Modified" changed the check so a changed ETag counts as modified even when Last-Modified is unchanged. Last-Modified is checked only when no ETag is present. The commit cites MDN's rule that `If-None-Match` takes precedence over `If-Modified-Since`.
- **ttl and sy:updatePeriod:** Not mentioned in the config docs. **UNVERIFIED**.

**Sources:** https://miniflux.app/docs/configuration.html ; https://git.causa-arcana.com/forks/miniflux/commit/bf1c8510930155a6f0e07de35149b360e61af3a6 (a Miniflux mirror; the commit is part of the upstream history)

### 9c. FreshRSS (PHP)

- **Minimum refresh:** Feeds are not refreshed more often than every 15 minutes. Global default is set under Configuration, Archiving. Fastest global setting is 20 minutes. Per-feed minimum can be set down to 15 minutes.
- **Trigger model:** Cron (recommended, for example `0 * * * *` running `actualize_script.php`), or an online cron URL (`c=feed&a=actualize`). `maxFeeds` default 10 feeds per run.
- **Push:** WebSub is suggested for near-instant updates.
- **ETag, Last-Modified, ttl, and error backoff:** The docs page fetched did not cover these. **UNVERIFIED.** A third-party "FreshRSS-AutoTTL" extension exists, which suggests core FreshRSS does not auto-honour `<ttl>` (inference, **UNVERIFIED**).

**Sources:** https://freshrss.github.io/FreshRSS/en/users/09_refreshing_feeds.html

### 9d. Specs for `<ttl>` and `sy:updatePeriod`

- Syndication module (`sy:`): `sy:updatePeriod` is hourly, daily, weekly, monthly, or yearly; default daily. `sy:updateFrequency` is a positive integer, default 1, meaning how many times per period. `sy:updateBase` is a W3CDTF date anchor. Example: hourly with frequency 2 means twice per hour.
- RSS 2.0 `<ttl>` (minutes until the cache should be refreshed) is from the RSS 2.0 spec and was **not** fetched in this session. Treat the meaning as known background, not verified here.

**Source:** https://web.resource.org/rss/1.0/modules/syndication/

### 9e. Pattern to implement (summary)

- Send `If-None-Match` (stored ETag) and `If-Modified-Since` on every poll. Treat 304 as unchanged. If the ETag changed, treat the feed as modified even if Last-Modified did not change (Miniflux approach).
- Base the next interval on the largest hint available: `<ttl>` in minutes, the `sy:` period converted to minutes, `Cache-Control: max-age`, or the observed item rate over the last week (Miniflux `entry_frequency`). Clamp to a floor of 15 minutes (FreshRSS floor) and a ceiling of 24 hours (both Miniflux and FreshRSS are compatible).
- On errors: exponential backoff with jitter, capped (for example at 24 hours). Avoid Miniflux's permanent stop after a few errors; it forces manual recovery.
- Cap concurrent requests per host (Miniflux `POLLING_LIMIT_PER_HOST`).

---

## Recommended stack

| Concern | Pick | Version | One-line justification |
|---|---|---|---|
| Feed parsing | feed-rs | 3.0.0 | MIT, active, covers all target formats; read `ttl` and `sy:` from raw XML if its model lacks them. |
| Full-text extraction | dom_smoothie | 0.18.2 | Only actively released readability port, and the only one judged correct in the 2025 comparison. |
| Embeddings | fastembed with BGE-small-en-v1.5 | 7.2.0 | Apache-2.0, active, 384-dimensional English model, downloads on first run and caches locally. |
| Lighter embedding fallback | model2vec-rs with potion-base-8M | 0.3.0 | Static embeddings with no transformer inference; confirm license and quality on your corpus. |
| Near-duplicate detection | gaoya (MinHash LSH) | 0.2.2 | Only LSH index crate found; verify thresholds, or write MinHash plus banding yourself. |
| Storage | rusqlite with `bundled` | 0.40.2 | Simple, synchronous, single-binary friendly; no compile-time database dependency. |
| Vector search | None (BLOB plus in-Rust cosine) | n/a | 20k x 384 f32 is about 31 MB; brute force is fast enough (estimate, UNVERIFIED). |
| Web framework | axum | 0.8.9 | Stable, widely used, MIT, MSRV 1.80. |
| Templates | askama | 0.16.1 | Compile-time type checking, active; maud is stale and minijinja has more churn. |
| Frontend | HTMX | 2.0.11 | Current npm `latest`; avoid 4.0.0 until it is stable. |
| LLM client | reqwest + serde | 0.13.5 | Lightest, no churn; one base URL works for Ollama, OpenRouter, and llama.cpp. |
| OPML | opml | 1.1.6 | Small, MIT/Apache-2.0; stale since 2024 but the format is stable. Test with real exports. |
| Scheduling | In-house, Miniflux-style | n/a | No suitable crate verified; use ETag, ttl or sy hints, entry-frequency adaptation, and jittered backoff. |

---

## UNVERIFIED items (summary)

- GitHub activity, open issues, and stars for every project (GitHub returned 403).
- feed-rs: MSRV, `<ttl>` and `sy:` support, raw extension access, 3.0.0 breaking changes.
- readable-readability and llm_readability release data.
- fastembed: embedding dimensions for non-default models, ONNX Runtime bundling and binary size, CPU throughput.
- model2vec-rs: embedding dimensions, license file contents (crates.io says "non-standard"), throughput on your hardware.
- gaoya SimHash index API and threshold semantics.
- sqlite-vec: stability of the 0.1.x line and Rust registration steps.
- askama and axum integration details for 0.16.
- HTMX 2.x license (BSD-2-Clause believed; not checked).
- genai and rig base-URL configuration methods.
- Endpoint paths for Ollama, OpenRouter, and llama.cpp.
- opml: serialization and `xmlUrl` handling.
- FreshRSS ETag, Last-Modified, ttl, and error handling (docs fetch returned an access-denied page on one URL and no detail on the other).
- ort download-binaries behavior and binary size.
- Adaptive scheduling crate `adaptive_backoff`.
- RSS 2.0 `<ttl>` semantics (not fetched).
