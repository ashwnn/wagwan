# Wagwan: Implementation Plan

Wagwan is a self-hosted, single-user RSS reader written in Rust. It merges articles from many feeds into **stories**, drops copycat articles, and ranks what is left by what you care about. Each story reads as one text in which every sentence links back to the article it came from. AI is optional: without it the story text is stitched from real sentences, and with it an LLM rewrites the text while keeping per-sentence sources.

This document is the single source of truth for building v1. It is written so an AI coding agent can pick up any milestone and implement it end to end. When something here conflicts with your instincts, follow this document; when something is missing, choose the simplest option and add it to section 15.

- Clickable UI prototype: [`docs/mockup.html`](docs/mockup.html). Open it at phone width.
- Background research, with sources: [`docs/research/`](docs/research/)

---

## Contents

1. Goals and non-goals
2. Decisions already made
3. Glossary
4. Architecture
5. Data model
6. Pipeline
7. Scoring
8. Story text (reader)
9. UI
10. Configuration (environment)
11. Deployment (Docker Compose and Cloudflare Access)
12. Repository layout and dependencies
13. Milestones
14. Testing and evaluation
15. Open questions and risks
16. Rules for agents working on this repo

---

## 1. Goals and non-goals

### Problems this solves

| User frustration | What Wagwan does |
|---|---|
| Some articles have more information than others | Stories combine sentences from all original sources, so nothing is missed and each detail links to who reported it |
| Too much copycat content | Near-duplicate articles (syndicated wire copy, rewrites) are detected and never shown |
| Too much that I do not care about | Stories get a "For you" score from written topics and thumbs; low scores are folded away. A separate "Discover" tab surfaces things that are new or unusual instead |

### Goals for v1

- Handle 10 to 400 feeds on a small VPS or home server.
- Small footprint: about 30 MB Docker image, under 200 MB RAM at 400 feeds, near-zero idle CPU.
- One container, deployed with `docker compose up -d`, configured only with environment variables.
- Phone-first web UI that works without JavaScript for reading and uses a small amount of vanilla JavaScript for interactions.
- AI is fully optional and off by default. Everything works without it.

### Non-goals for v1

Do not build these, and do not add hooks for them:

- User accounts, login, or any built-in auth. Cloudflare Access sits in front (section 11).
- Third-party client sync APIs (Google Reader, Fever).
- A SPA frontend, a JS build step, CSS frameworks, or icon fonts.
- A vector database. Vectors live in SQLite BLOBs and are compared in memory.
- Multi-language support. English feeds only; other languages may cluster poorly.
- Push notifications, email digests, or mobile apps.
- Republishing article text. Extracted text is stored locally for processing and is never served as a full article.

---

## 2. Decisions already made

These are settled. Do not reopen them without the owner's approval.

| Topic | Decision |
|---|---|
| Name | Wagwan. Binary `wagwan`, data directory `/data` in the container |
| Users | Single user, self-hosted |
| Auth | None in the app. Users put Cloudflare Access (via Cloudflare Tunnel) in front |
| Language | Rust (stable), single crate, single binary |
| Deployment | Docker Compose, one service plus an optional `cloudflared` service |
| Config | Environment variables only. Feeds, topics and mutes are managed in the UI and stored in SQLite |
| License | `MIT OR Apache-2.0` |
| UI style | Minimal "Web 2.0": system font, blue headlines, thin gray rules, native controls, phone first. See `docs/mockup.html` |
| Tabs | For you, Discover, Saved, Settings |
| Reading | In-app story page. The whole sentence is the link; tapping it opens a bottom sheet with the outlet name and an "Open in <outlet>" button |
| AI off | Story text is extractive: real sentences from the original articles |
| AI on | LLM rewrites with a source list per sentence; sentences that fail verification are shown in gray |
| Interests | Written topics plus thumbs up and down |
| Scores | Points from 0 to 99, shown on every story |
| Retention | 90 days for articles and stories; saved stories are kept forever |
| Text shown on screen | No "sources, original, copies" counts and no "written by AI" labels. The story page shows "First seen · Updated" and the score line only |

---

## 3. Glossary

| Term | Meaning |
|---|---|
| Feed | One RSS, Atom or JSON Feed URL |
| Article | One item from a feed, with its extracted text |
| Story | A cluster of articles about the same real-world event |
| Original | An article in a story that is not a near-duplicate of an earlier one |
| Copy | An article whose text is a near-duplicate of an earlier article in the same story. Copies are stored but never shown |
| Topic | A short phrase the user writes, such as "BC housing" or "Rust". Stored with its embedding |
| Mute | A topic or a feed whose stories are hidden |
| Story text | The sentences shown on the story page, each linked to one source article |
| For you score | 0 to 99, how relevant a story is to the user |
| Discover score | 0 to 99, how notable a story is regardless of the user's topics |

---

## 4. Architecture

One process, one SQLite file, one tokio runtime.

```
             +----------------------- wagwan (single binary) ------------------------+
             |                                                                       |
 feeds  ---> |  scheduler -> fetcher -> extractor -> embedder -> clusterer -> scorer  |
 (HTTP)      |                                           |            |              |
             |                                           v            v              |
             |                                    story text builder (+ optional LLM)|
             |                                                  |                    |
             |                      SQLite (WAL) <--------------+                    |
             |                           ^                                           |
             |      axum HTTP server ----+---> askama templates + 1 small JS file    |
             +-----------------------------------------------------------------------+
                         ^
           cloudflared (optional) <--- Cloudflare Access <--- your phone
```

- **Background loop.** A single tokio task wakes every 60 seconds and processes the feeds that are due (section 6.1). Heavy CPU work, such as embedding and clustering, runs in `spawn_blocking`.
- **Web server.** axum serves HTML pages rendered with askama, one static CSS file and one static JS file, all embedded in the binary.
- **Storage.** SQLite in WAL mode via `rusqlite` with the `bundled` feature. One write connection is shared through a `Mutex`, and reads use a small pool of read-only connections.
- **In memory.** Embeddings of stories active in the last 72 hours and topic embeddings are cached for fast similarity checks and are reloaded from SQLite at startup.

---

## 5. Data model

Use plain SQL migrations in `migrations/NNNN_name.sql`, applied in order at startup and tracked in a `schema_version` table. All timestamps are Unix seconds (UTC) as `INTEGER`. Embeddings are little-endian `f32` arrays stored as `BLOB`.

```sql
CREATE TABLE feeds (
  id              INTEGER PRIMARY KEY,
  url             TEXT NOT NULL UNIQUE,
  title           TEXT,
  site_url        TEXT,
  folder          TEXT,              -- from OPML outline, used as a fallback label
  muted           INTEGER NOT NULL DEFAULT 0,
  etag            TEXT,
  last_modified   TEXT,
  interval_secs   INTEGER NOT NULL DEFAULT 1800,
  next_fetch_at   INTEGER NOT NULL DEFAULT 0,
  last_ok_at      INTEGER,
  error_count     INTEGER NOT NULL DEFAULT 0,
  last_error      TEXT,
  disabled        INTEGER NOT NULL DEFAULT 0,
  created_at      INTEGER NOT NULL
);

CREATE TABLE articles (
  id              INTEGER PRIMARY KEY,
  feed_id         INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
  guid            TEXT NOT NULL,
  url             TEXT NOT NULL,
  title           TEXT NOT NULL,
  published_at    INTEGER,           -- from the feed, may be missing or wrong
  seen_at         INTEGER NOT NULL,  -- when Wagwan first saw it; used for ordering
  text            TEXT,              -- extracted plain text, NULL if extraction failed
  text_source     TEXT NOT NULL,     -- 'page' | 'feed' (fell back to feed content)
  embedding       BLOB,              -- title + lede
  minhash         BLOB,              -- 128 x u32
  story_id        INTEGER REFERENCES stories(id),
  copy_of         INTEGER REFERENCES articles(id),  -- set if near-duplicate
  uniqueness      REAL,              -- 0..1, share of this article's facts nobody else had
  UNIQUE (feed_id, guid)
);
CREATE INDEX articles_story ON articles(story_id);
CREATE INDEX articles_seen ON articles(seen_at);

CREATE TABLE sentences (
  id              INTEGER PRIMARY KEY,
  article_id      INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  position        INTEGER NOT NULL,  -- order within the article
  text            TEXT NOT NULL,
  embedding       BLOB NOT NULL
);
CREATE INDEX sentences_article ON sentences(article_id);

CREATE TABLE stories (
  id              INTEGER PRIMARY KEY,
  title           TEXT NOT NULL,      -- title of the best original article, or LLM title
  label           TEXT,               -- matched topic name, or feed folder, shown in the list
  centroid        BLOB NOT NULL,
  first_seen_at   INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,   -- last time an original article joined
  closed          INTEGER NOT NULL DEFAULT 0,  -- no new articles may join
  for_you         INTEGER NOT NULL DEFAULT 0,  -- 0..99
  discover        INTEGER NOT NULL DEFAULT 0,  -- 0..99
  discover_reason TEXT,               -- 'widely_covered' | 'unusual' | 'new_detail'
  text_version    INTEGER NOT NULL DEFAULT 0,
  text_built_for  TEXT,               -- hash of original article ids used
  read_at         INTEGER,            -- last time the user opened it
  saved_at        INTEGER,
  vote            INTEGER NOT NULL DEFAULT 0  -- -1, 0, 1
);
CREATE INDEX stories_updated ON stories(updated_at);

CREATE TABLE story_text (
  story_id        INTEGER NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
  mode            TEXT NOT NULL,      -- 'extractive' | 'ai'
  position        INTEGER NOT NULL,
  text            TEXT NOT NULL,      -- sentence shown to the user
  sentence_id     INTEGER NOT NULL REFERENCES sentences(id),  -- primary source sentence
  extra_sources   TEXT,               -- JSON array of other sentence ids, AI mode only
  verified        INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY (story_id, mode, position)
);

CREATE TABLE disagreements (          -- AI mode only
  story_id        INTEGER NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
  position        INTEGER NOT NULL,
  subject         TEXT NOT NULL,      -- e.g. "Homes added"
  detail          TEXT NOT NULL,      -- e.g. "Harbour Ledger 4,000, Coastline Daily 4,200"
  PRIMARY KEY (story_id, position)
);

CREATE TABLE topics (
  id              INTEGER PRIMARY KEY,
  name            TEXT NOT NULL UNIQUE,
  embedding       BLOB NOT NULL,
  muted           INTEGER NOT NULL DEFAULT 0,   -- 1 = hide stories that match
  created_at      INTEGER NOT NULL
);

CREATE VIRTUAL TABLE stories_fts USING fts5(title, body, content='');
```

Notes:

- `stories_fts.body` is the concatenated story text. Rebuild a story's row whenever its text changes.
- Retention job (daily): delete articles with `seen_at` older than `WAGWAN_RETENTION_DAYS`, except those in stories with `saved_at` set. Delete stories left with no articles. Then run `PRAGMA incremental_vacuum`.

---

## 6. Pipeline

Each step below has default constants. Put every constant in `src/tuning.rs` as a `pub const` with a comment explaining it. Do not make them environment variables.

### 6.1 Scheduling and fetching

- Every 60 seconds, select feeds where `next_fetch_at <= now AND disabled = 0`, ordered by `next_fetch_at`.
- Fetch with at most `WAGWAN_FETCH_CONCURRENCY` (default 16) requests in flight overall and at most 2 per host.
- Send `If-None-Match` and `If-Modified-Since` when stored. `304` counts as success with no new items.
- Use the User-Agent `Wagwan/<version> (+https://github.com/<org>/wagwan)`, a 20 second timeout, at most 5 redirects, and a 5 MB body limit.
- Parse with `feed-rs`. Store new items only, keyed by `(feed_id, guid)`. If an item has no guid, use its link as the guid.
- **Adaptive interval** after each successful fetch:
  - Count the items with `seen_at` in the last 7 days.
  - Set `interval = clamp(7 days / max(count, 1), 15 min, 6 h)`.
  - If the feed declares `<ttl>`, use `max(interval, ttl)`.
- **Errors.** On failure, set `next_fetch_at = now + min(interval * 2^error_count, 24 h)` with ±10% jitter. After 10 consecutive failures, set `disabled = 1` and show the feed as failing in Settings, with a Retry button.
- Ignore items whose `published_at` is more than 7 days old when the feed is first added, to avoid importing a backlog.

### 6.2 Extraction

- For each new article, fetch `url` (same limits as above) and run `dom_smoothie` to get the main text.
- If extraction fails or yields fewer than 300 characters, fall back to the feed's own `content` or `summary` with HTML stripped, and set `text_source = 'feed'`.
- Normalize the text: collapse whitespace and drop lines shorter than 25 characters that look like boilerplate ("Advertisement", "Share this", "Sign up").
- Split the text into sentences with a simple rule-based splitter (`.`, `!`, `?` followed by whitespace and a capital letter or a quote). Handle common abbreviations (Mr., Dr., St., U.S., e.g., i.e., a.m., p.m.) and decimals. Keep sentences of 25 to 400 characters.
- Store the sentences with their positions. Embed each sentence (section 6.3).

### 6.3 Embeddings

- Use `model2vec-rs` with the `minishlab/potion-base-8M` static model. It is about 30 MB, has 256 dimensions, and needs no ONNX runtime.
- Download the model on first start into `/data/models/` from `WAGWAN_MODEL_URL` (default: the Hugging Face repo) and verify a SHA-256 checksum pinned in code. If `/data/models/` already holds the files, skip the download, so offline installs can mount them.
- **Article embedding:** the title plus the first 120 words of text.
- **Topic embedding:** the topic phrase itself.
- Normalize all vectors to unit length, so cosine similarity is a dot product.
- **Quality gate:** milestone M2 includes an evaluation (section 14.2). If potion-base-8M fails it, switch to `fastembed` with `bge-small-en-v1.5` and update this section. Do not ship both.

### 6.4 Clustering into stories

This is online, single-pass clustering against story centroids, following Miranda et al. 2018 (see `docs/research/clustering.md`).

For each new article, in `seen_at` order:

1. **Candidates:** stories that are not closed and whose `updated_at` falls within the last 72 hours.
2. **Similarity:** `sim = dot(article.embedding, story.centroid)`.
3. **Time weight:** `w = exp(-(hours since story.updated_at)^2 / (2 * 72^2))`, so `score = sim * w`.
4. **Join** the best story if `score >= JOIN_THRESHOLD` (start at 0.80 and calibrate in M2) **and** the article shares at least one "fact token" (section 6.6) with the story's originals. The fact-token gate prevents unrelated stories from chaining together.
5. **Otherwise** create a new story with this article as its first member.
6. **When an article joins**, update `centroid = normalize(mean of member embeddings)`. Set `updated_at = now` only if the article is an original (section 6.5).
7. **Close** a story when it has had no new originals for 72 hours or is more than 10 days old. Closed stories accept no new articles.

Assignments are permanent: no re-clustering and no merging of existing stories.

### 6.5 Copies (near-duplicates)

Compare only within a story. Stories rarely have more than 30 articles, so no LSH index is needed.

- Compute a MinHash signature (128 hashes) over word 5-gram shingles of the normalized, lowercased text. This is about 80 lines of code; do not add a crate.
- A new article is a **copy** of an earlier member if either:
  - their estimated Jaccard similarity is at least 0.8, or
  - their **containment** `|A ∩ B| / min(|A|, |B|)` is at least 0.8. This catches syndicated copy with an added local paragraph.
  
  Compute containment exactly on shingle sets for the pairs whose MinHash Jaccard is at least 0.5.
- Set `copy_of` to the earliest matching article. Copies never contribute sentences, never count toward coverage, and are never shown in the UI.

### 6.6 Uniqueness (facts per article)

This is a cheap heuristic with no NLP models.

- **Fact tokens** extracted from an article's text:
  - Numbers, normalized: strip commas; handle "3.2 million" and "3,200,000"; percentages; currency.
  - Dates and times.
  - Capitalized multi-word spans that are not at the start of a sentence.
  - Quoted strings of 3 or more words.
- **Uniqueness** of an original article: the share of its fact tokens that do not appear in any other original in the story that was seen earlier.
- It is used to order sentences in the extractive story text and to compute the Discover "new_detail" reason.

### 6.7 Retention

A daily job covers retention, as described in section 5.

---

## 7. Scoring

Both scores are recomputed for a story whenever:

- it gains an original,
- the user votes on any story,
- topics or mutes change, or
- once an hour, for recency.

Keep the formulas in `src/score.rs` with unit tests.

### 7.1 Shared inputs

| Input | Definition |
|---|---|
| `topic_sim` | Highest `dot(story.centroid, topic.embedding)` over unmuted topics |
| `coverage` | Number of distinct feeds with an original article in the story |
| `age_h` | Hours since `story.updated_at` |
| `taste` | `mean(dot(centroid, liked)) - mean(dot(centroid, disliked))` over stories voted on in the last 90 days, using each story's centroid. Zero when there are no votes |
| `muted` | True if `dot(centroid, muted_topic) >= MUTE_THRESHOLD` (0.75) for any muted topic, or if every original comes from muted feeds |

Muted stories are never shown in any tab and are not scored.

### 7.2 For you

```
raw = 3.0 * topic_sim
    + 2.0 * taste
    + 0.4 * ln(1 + coverage)
    - 0.03 * age_h
for_you = round(99 * sigmoid(4 * (raw - 2.2)))     // clamp to 0..99
```

- The For you tab lists stories whose `topic_sim >= TOPIC_MATCH` (0.70) or that the user voted up, sorted by `for_you`, newest first on ties.
- Stories below `WAGWAN_MIN_SCORE` (default 40) go into the folded "N more, lower priority" section at the bottom.
- `story.label` is the name of the best-matching topic.
- The constants `2.2` and `4` set where the score curve sits. Tune them in M4 so a typical strong match lands between 80 and 95.

### 7.3 Discover

Only stories that are not in For you are eligible, meaning `topic_sim < TOPIC_MATCH` and no up vote.

```
novelty   = 1 - max(dot(centroid, c)) over centroids of stories from the previous 7 days (excluding itself)
detail    = max uniqueness among its originals
raw = 1.5 * ln(1 + coverage)
    + 2.0 * novelty
    + 1.0 * detail
    + 1.0 * taste
    - 0.02 * age_h
discover = round(99 * sigmoid(3 * (raw - 2.0)))
```

- `discover_reason`, shown in the list as the first part of the meta line:
  - `widely_covered` ("Widely covered, N feeds") when `coverage >= 6`;
  - otherwise `unusual` ("Unusual") when `novelty >= 0.35`;
  - otherwise `new_detail` ("Lots of new detail").
- `story.label` for Discover stories is the `folder` of the feed that contributed the best original, or the feed title if there is no folder.
- The Discover tab shows the top 30 by `discover` from the last 48 hours.

### 7.4 Votes

- Thumbs up sets `vote = 1`; thumbs down sets `vote = -1`; tapping the active thumb again sets `vote = 0`.
- After a vote, recompute scores for stories updated in the last 72 hours, in the background. The UI updates optimistically.

---

## 8. Story text (reader)

The story page shows a single text in which every sentence comes from, and links to, one source article.

### 8.1 Extractive mode (always available, default)

Build or rebuild the text when a story gains an original and the user's next request for the story arrives. Rebuild lazily on request, then cache: `text_built_for` holds a hash of the original article ids.

1. **Pool:** all sentences from original articles in the story.
2. **Start** with the first two sentences of the original with the highest uniqueness (its lede).
3. **Fill:** repeatedly add the sentence with the highest `0.6 * relevance - 0.4 * redundancy` (Maximal Marginal Relevance), where:
   - `relevance` is `dot(sentence, centroid)`;
   - `redundancy` is the highest `dot(sentence, s)` over sentences already chosen.

   Skip any sentence with redundancy at least 0.85. Stop at 12 sentences or when no candidate has `relevance >= 0.5`.
4. **Order:** the lede first, then the rest by their source article's `seen_at`, then by their position within that article.
5. **Store** the result in `story_text` with `mode = 'extractive'`.

### 8.2 AI mode (when `WAGWAN_LLM_BASE_URL` is set)

Generate the AI text after the extractive text, for stories with `for_you >= WAGWAN_MIN_SCORE` or in the Discover top 30. Run at most `WAGWAN_LLM_CONCURRENCY` (default 2) calls at a time. Cache the result by `text_built_for`.

**Prompt input:** the story's candidate sentences from 8.1 (up to 40), numbered `[S1]..[Sn]` with their outlet name.

**Prompt instructions (system):**

- Write a neutral news story of at most 10 sentences using only facts from the numbered sentences.
- Return JSON matching `{"title": str, "sentences": [{"text": str, "sources": [int]}], "disagreements": [{"subject": str, "detail": str}]}`.
- Every sentence must list the numbers of the source sentences that support it. Never add facts that are not in them.
- List disagreements only when sources give different numbers, dates or claims about the same thing, naming the outlets.

**Request:**

- Use the OpenAI-compatible `POST {base}/chat/completions` endpoint, with `response_format: {"type":"json_object"}` when the provider supports it.
- Set the temperature to 0.2.
- Time out after 60 seconds.
- On invalid JSON, retry once. If it fails again, keep the extractive text and log the failure.

**Verification:** for each output sentence, all of these must hold or it is stored with `verified = 0`:

- every number and capitalized multi-word span in it appears in at least one cited source sentence;
- the highest `dot(embed(output), embed(cited))` across the cited sentences is at least 0.75;
- every cited id exists.

**Storage:** `mode = 'ai'`. Set `sentence_id` to the cited sentence with the highest similarity and put the rest in `extra_sources`. Store disagreements, and use the LLM title as the story title only if it is non-empty.

The UI shows AI text by default when it exists. "Show original sentences" in the More sheet switches to the extractive text.

### 8.3 Links to sources

- Each sentence links to its article URL plus a text fragment that jumps to and highlights the sentence in supporting browsers: `#:~:text=<first 6 words>,<last 4 words>`, percent-encoded.
- In AI mode, the link and the quoted "original wording" in the bottom sheet use the primary source sentence.

### 8.4 New since last read

When the story page is opened, sentences whose source article has `seen_at > story.read_at` get the `new` class (yellow highlight), and the "Highlighted is new since you last read this" legend is shown. Then set `read_at = now`.

---

## 9. UI

The reference is `docs/mockup.html`. Match its layout, spacing, colors and behavior. Server-render every page with askama, and use the one JS file only for progressive enhancement.

### 9.1 Routes

| Method and path | Purpose |
|---|---|
| `GET /` | Redirect to `/for-you` |
| `GET /for-you` | For you list |
| `GET /discover` | Discover list |
| `GET /saved` | Saved stories, newest saved first |
| `GET /search?q=` | FTS5 search over story titles and text, results in list style |
| `GET /s/{id}` | Story page. `?mode=extractive` forces original sentences |
| `POST /s/{id}/vote` | Form field `v` = `1`, `-1` or `0`. Returns 204 for fetch requests, or redirects back for form posts |
| `POST /s/{id}/save` | Toggles saved |
| `POST /mute/topic` | Field `name`. Adds a muted topic |
| `POST /mute/feed/{id}` | Mutes a feed |
| `GET /settings` | Topics, feeds, mutes, AI status, version |
| `POST /settings/topics` | Add (`name`) or delete (`delete_id`) a topic |
| `POST /settings/feeds` | Add a feed URL, delete, retry, or unmute |
| `POST /settings/opml` | Upload an OPML file to import |
| `GET /settings/opml` | Download all feeds as OPML |
| `GET /healthz` | `200 ok` when the DB is reachable. Used by the Docker healthcheck |
| `GET /static/*` | `app.css`, `app.js` and the favicon, embedded with `include_bytes!` and served with long cache headers |

### 9.2 Screens

**List (For you and Discover)**

- **Top bar:** "Wagwan" and a Refresh link that triggers a fetch of all due feeds and reloads the page.
- **Search field:** a native `<input type=search>` at 16px, so iOS does not zoom.
- **Status line:** "Updated 7:42 am" (the last completed fetch cycle) and a one-line explanation of the tab.
- **Rows:**
  - the headline as a link;
  - a meta line with an "Updated" marker (For you, if `updated_at > read_at`) or the Discover reason, then the label and the age;
  - the points shown on the right.

  Read stories have gray headlines. The Discover reason wording comes from section 7.3; the mockup's "New angle" and "New to you" are placeholders.
- **Lower priority:** For you ends with a native `<details>` holding "N more, lower priority".
- **Bottom tab bar:** For you, Discover, Saved, Settings, with 44px or larger tap targets and safe-area padding.

**Story page**

- **Top bar:** "‹ Back" and "More".
- **Header:** the headline, then "First seen 6h ago · Updated 40m ago", then the score line "94 pts · Matches BC housing" (For you) or "88 pts · Unusual" (Discover).
- **Body:** each sentence is a focusable element with `role=link`.
  - Tapping a sentence opens the bottom sheet: the outlet name, the original sentence (AI mode), a warning for unverified sentences, and an "Open in <outlet>" button.
  - Without JavaScript, each sentence is a plain `<a href>` to the source.
- **Sources differ:** shown only in AI mode, and only when there are disagreements.
- **Read in full:** links to each original article, one row per outlet.
- **Bottom action bar:** Relevant (thumbs up), Not for me (thumbs down), Save. Each action is optimistic, with a toast that offers Undo.
- **More sheet:**
  - Show original sentences (or Show summary);
  - Share (`navigator.share`, falling back to copying the link);
  - Mute topic: <label>;
  - Mute a source… (a list of outlets).

**Settings**

A plain page with native forms, in this order:

- Topics (add, delete);
- Feeds (add URL, list with status, Retry for failing feeds, delete, OPML import and export);
- Muted (topics and feeds, with unmute);
- AI (shows "Off" or the model name, read-only);
- About (version, data size, last fetch).

### 9.3 Front-end rules

- **Pages:** keep `app.css` under about 250 lines and `app.js` under about 250 lines, written as plain ES2020 with no dependencies and no build step.
- **No JavaScript:** every action must work as a form POST. JS only adds the bottom sheet, toasts, optimistic voting and Undo.
- **Themes:** support light and dark with `prefers-color-scheme`, using the mockup's CSS variables.
- **Accessibility:** use 44px minimum tap targets, visible focus, `aria-pressed` on the thumbs, and support `prefers-reduced-motion`.
- **Text:** never use em or en dashes in UI copy.

---

## 10. Configuration (environment)

All configuration comes from environment variables, read once at startup into a `Config` struct. Unknown `WAGWAN_*` variables log a warning. Each variable may also be given as `<NAME>_FILE` pointing to a file (for Docker secrets), which matters for the API key.

| Variable | Default | Meaning |
|---|---|---|
| `WAGWAN_BIND` | `0.0.0.0:8080` | Listen address inside the container |
| `WAGWAN_DATA_DIR` | `/data` | SQLite database and models |
| `WAGWAN_TZ` | `UTC` | IANA time zone for displayed times, e.g. `America/Vancouver` |
| `WAGWAN_RETENTION_DAYS` | `90` | Age at which articles and stories are deleted. Saved stories are kept |
| `WAGWAN_MIN_SCORE` | `40` | For you stories below this go into "lower priority" |
| `WAGWAN_FETCH_CONCURRENCY` | `16` | Maximum fetches in flight |
| `WAGWAN_FETCH_MIN_INTERVAL` | `15m` | Lower bound for adaptive polling |
| `WAGWAN_FETCH_MAX_INTERVAL` | `6h` | Upper bound for adaptive polling |
| `WAGWAN_OPML_SEED` | unset | Path to an OPML file imported once, on first start with an empty DB |
| `WAGWAN_TOPICS_SEED` | unset | Comma-separated topics added on first start, e.g. `BC housing,Rust,Open-source AI` |
| `WAGWAN_MODEL_URL` | Hugging Face URL for potion-base-8M | Where to download the embedding model |
| `WAGWAN_LLM_BASE_URL` | unset | OpenAI-compatible base URL, e.g. `http://ollama:11434/v1`. Unset means AI is off |
| `WAGWAN_LLM_API_KEY` | unset | Bearer token, if the provider needs one |
| `WAGWAN_LLM_MODEL` | unset | Model name; required when the base URL is set |
| `WAGWAN_LLM_CONCURRENCY` | `2` | Maximum LLM calls in flight |
| `WAGWAN_LOG` | `info` | `tracing` filter, e.g. `wagwan=debug` |

Durations accept `30s`, `15m`, `6h` and `1d`.

---

## 11. Deployment (Docker Compose and Cloudflare Access)

### 11.1 Image

- Multi-stage `Dockerfile`:
  - **Build:** `rust:1-alpine` building for `x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl`, with `--release`, `lto = "fat"`, `codegen-units = 1`, `strip = true` and `panic = "abort"`.
  - **Runtime:** `FROM scratch` with only the binary. TLS uses `rustls` with `webpki-roots`, so no CA bundle is needed.
- Run as UID 10001. `/data` is the only writable path.
- The binary has a `wagwan healthcheck` subcommand that calls `GET /healthz` on localhost, because `scratch` has no curl.
- Publish multi-arch images (amd64, arm64) to GHCR from CI on tags.
- **Footprint targets** (check in M6): image under 30 MB, RSS memory under 200 MB with 400 feeds, idle CPU under 1%.

### 11.2 `docker-compose.yml` (ship this file in the repo root)

```yaml
services:
  wagwan:
    image: ghcr.io/<org>/wagwan:latest
    restart: unless-stopped
    environment:
      WAGWAN_TZ: America/Vancouver
      # WAGWAN_TOPICS_SEED: "BC housing,Rust,Open-source AI"
      # WAGWAN_LLM_BASE_URL: http://ollama:11434/v1
      # WAGWAN_LLM_MODEL: llama3.2:3b
    volumes:
      - wagwan-data:/data
    ports:
      - "127.0.0.1:8080:8080"   # local only; remote access goes through Cloudflare
    healthcheck:
      test: ["CMD", "/wagwan", "healthcheck"]
      interval: 30s
      timeout: 5s
      retries: 3

  cloudflared:
    image: cloudflare/cloudflared:latest
    profiles: ["tunnel"]
    restart: unless-stopped
    command: tunnel --no-autoupdate run
    environment:
      TUNNEL_TOKEN: ${CLOUDFLARE_TUNNEL_TOKEN}
    depends_on:
      wagwan:
        condition: service_healthy

volumes:
  wagwan-data:
```

- **Local only:** `docker compose up -d` serves Wagwan at `http://localhost:8080`.
- **Remote access:** `CLOUDFLARE_TUNNEL_TOKEN=... docker compose --profile tunnel up -d`.

### 11.3 Cloudflare Access setup (document in `docs/cloudflare.md`)

Wagwan has no login. The README must say this in its first screen of text, and must tell users never to publish port 8080 to the internet.

The steps for `docs/cloudflare.md`:

1. In Cloudflare Zero Trust, create a **Tunnel**, choose Docker, and copy the token into `CLOUDFLARE_TUNNEL_TOKEN`.
2. Add a **Public hostname**, such as `news.example.com`, with service `http://wagwan:8080`.
3. Create an **Access application** (self-hosted) for that hostname, with a policy that allows only your email address (one-time PIN or your identity provider).
4. Start with `docker compose --profile tunnel up -d` and open the hostname on your phone. Cloudflare asks you to log in, then shows Wagwan.
5. Optional: add the site to your phone's home screen. Ship a minimal `manifest.webmanifest` with name, icons and `display: standalone`. No service worker in v1.

Checking the `Cf-Access-Jwt-Assertion` header inside Wagwan is out of scope for v1 (section 15). Because the port is bound to localhost, the tunnel is the only way in.

---

## 12. Repository layout and dependencies

### 12.1 Layout

Single crate. No workspace.

```
wagwan/
  PLAN.md
  README.md               what it is, screenshot, "no built-in auth" warning, quick start
  LICENSE-MIT
  LICENSE-APACHE
  Cargo.toml
  Dockerfile
  docker-compose.yml
  .github/workflows/ci.yml        fmt, clippy -D warnings, test, docker build
  .github/workflows/release.yml   multi-arch image to GHCR on tag
  migrations/0001_init.sql
  src/
    main.rs        CLI (serve | healthcheck | import-opml | eval), startup, shutdown
    config.rs      env parsing
    db.rs          connections, migrations, queries
    tuning.rs      all algorithm constants
    fetch.rs       scheduler and HTTP fetching
    extract.rs     dom_smoothie, cleanup, sentence split
    embed.rs       model download and checksum, model2vec wrapper
    cluster.rs     story assignment, centroids, closing
    dedup.rs       MinHash, containment
    facts.rs       fact tokens and uniqueness
    score.rs       For you and Discover formulas
    storytext.rs   extractive builder, text fragment links
    llm.rs         OpenAI-compatible client, prompt, verification
    opml.rs
    web/
      mod.rs       router
      pages.rs     handlers
      templates/   askama .html files
      static/      app.css, app.js, icons, manifest
  tests/
    fixtures/      saved feeds and pages for offline tests
  docs/
    mockup.html
    cloudflare.md
    research/
```

### 12.2 Dependencies

Adding a crate not on this list needs a one-line justification in the PR description.

| Crate | Use |
|---|---|
| `tokio` (rt-multi-thread, macros, time, sync) | Runtime |
| `axum` 0.8 | HTTP server |
| `askama` | Templates |
| `reqwest` (rustls-tls, gzip, brotli; no default features) | HTTP client |
| `feed-rs` | Feed parsing |
| `dom_smoothie` | Article extraction |
| `model2vec-rs` | Embeddings |
| `rusqlite` (bundled) | SQLite with FTS5 |
| `serde`, `serde_json` | JSON for the LLM and settings |
| `quick-xml` or `opml` | OPML import and export |
| `tracing`, `tracing-subscriber` | Logging |
| `anyhow`, `thiserror` | Errors |
| `sha2` | Model checksum |
| `time` | Time zones and formatting |

Not used:

- HTMX (the JS file covers it);
- `sqlx` (no compile-time DB);
- `gaoya` (MinHash is written in-house);
- an LLM SDK (`reqwest` is enough);
- `fastembed` (unless the M2 gate fails).

---

## 13. Milestones

Each milestone ends in an implemented state. Owner override (2026-10-22): build all milestones without adding or running tests; implementation may proceed while runtime and evaluation gates remain pending. The original done lists remain the completion criteria. Subsequent owner instruction on 2026-10-22 authorized testing, runtime verification and screenshot capture; validation is now in progress.

### M1. Ingest

- Set up the repo skeleton, config, migrations, CLI, and `GET /healthz`.
- Build the scheduler, fetcher (conditional GET, backoff, adaptive interval), extraction with feed fallback, and the sentence splitter.
- Implement OPML import (CLI and `WAGWAN_OPML_SEED`) and the Settings feed list with add, delete and retry.
- **Done when:**
  - importing a 50-feed OPML and running for an hour stores articles with text;
  - a `304` response is logged as not modified;
  - a broken feed URL shows as failing in Settings;
  - `cargo test` covers the sentence splitter (20 or more cases, including abbreviations and decimals) and the interval and backoff math.

### M2. Stories and copies

- Model download with checksum, embeddings, clustering, MinHash copies, fact tokens and uniqueness, closing, and retention.
- A temporary debug page at `/debug/stories` listing stories with their member articles, copy flags and similarity values. Remove it in M6.
- `wagwan eval` subcommand (section 14.2).
- **Done when:**
  - the eval reports pairwise F1 of at least 0.85 for same-story detection, or the embedding switch in 6.3 has been made;
  - copy detection catches 95% or more of the syndicated pairs in the fixtures with no false positives;
  - one day of real data shows no story with more than 1 obviously unrelated article (manual check of 30 stories, noted in the PR).

### M3. Lists and story page (no AI)

- Templates, CSS and JS matching `docs/mockup.html`:
  - For you, Discover and Saved lists, search, and the story page with extractive text;
  - text-fragment links, the bottom sheet, toasts, votes, save, mutes, the More sheet, and the new-since-last-read highlight.
- Topics in Settings, and `WAGWAN_TOPICS_SEED`.
- **Done when:**
  - every action works with JavaScript disabled;
  - the pages match the mockup at 390px wide in light and dark;
  - Lighthouse accessibility is 95 or higher on mobile;
  - no page is wider than the viewport at 360px.

### M4. Scoring

- Implement `score.rs` exactly as in section 7, with background recomputation after votes, topic changes and mutes, and hourly recency updates.
- Discover reasons and labels.
- **Done when:**
  - unit tests cover each formula and the mute rules;
  - after voting up three stories on a topic, other stories on that topic rise in For you within 1 minute;
  - no story appears in both For you and Discover.

### M5. Optional AI

- `llm.rs`: client, prompt, JSON parsing with retry, verification, disagreements, and the "Show original sentences" toggle.
- Settings shows the AI status.
- **Done when:**
  - with Ollama in compose, top stories get AI text, and every sentence opens a sheet with its source;
  - a sentence with an invented number is stored with `verified = 0` (unit test with a canned LLM response);
  - with the LLM unreachable, pages still render using the extractive text.

### M6. Ship

- Dockerfile, compose file, `docs/cloudflare.md`, README with screenshots, the web manifest, CI and release workflows. Remove the debug page.
- **Done when:**
  - a fresh `docker compose up -d` on a clean machine works with only `WAGWAN_TZ` set;
  - the image is under 30 MB;
  - memory stays under 200 MB after 24 hours with 400 feeds (log the numbers in the PR);
  - the Cloudflare guide has been followed end to end once.

---

## 14. Testing and evaluation

### 14.1 Tests

- **Unit tests** for:
  - the sentence splitter, fact tokens, MinHash and containment;
  - the interval and backoff math;
  - the scoring formulas;
  - text fragment encoding;
  - LLM JSON parsing and verification.
- **Offline fixtures** in `tests/fixtures/`: a few real feeds and article pages saved to disk, including one syndicated pair and one page where extraction fails. No test may hit the network.
- **Integration test:** feed a fixture set through the whole pipeline with an in-memory DB and assert the story count, the copies and the story text.

### 14.2 Clustering evaluation (`wagwan eval`)

- **Dataset:** SemEval-2022 Task 8 news similarity pairs (CC BY 4.0, https://zenodo.org/records/6507872), English pairs only.
  - Download it manually into `tests/eval/`; it is not committed.
  - Treat pairs with an "overall" score of 1.5 or below as the same story.
- **Output:** the command embeds each pair as in 6.3 and prints precision, recall and F1 for thresholds from 0.70 to 0.95 in steps of 0.01.
- **Calibration:** pick `JOIN_THRESHOLD` from the best F1 and record the result in `tuning.rs` with the date and model name.

---

## 15. Open questions and risks

| Item | Status and default |
|---|---|
| Release verification | CI exercises mobile fixture views and native container startup/persistence before tagged publishing. Full duration, quality and account deployment gates still require measured evidence; see docs/progress.md |
| Implementation and validation order | Owner initially deferred testing, then authorized verification on 2026-10-22. Runtime, quality and footprint gates remain pending until evidence proves them |
| Local Compose startup | Compose builds the local source by default, avoiding a dependency on a release image that has not yet been published; tagged GHCR releases remain configured |
| Time zones | `time-tz` supplies IANA rules inside the scratch container; no host tzdata files are needed |
| Text build timing | Extractive text is rebuilt in the background after original-article changes and cached by their IDs, so request handlers never run embedding or text-selection CPU work |
| Vote timing | Migration 0003 adds `voted_at` so the 90-day taste window follows the vote time even for an older saved story |
| FTS storage | Migration 0002 uses a contentful FTS5 index so story updates and retention can delete individual rows safely. The short story text is duplicated on disk |
| potion-base-8M may be too weak for event-level similarity | M2 gate decides. Fallback is fastembed bge-small (adds ONNX Runtime, image grows to about 80 MB) |
| model2vec-rs license shows as "non-standard" on crates.io | Check the license file before M2. If it is not MIT-compatible, write a small loader for the safetensors model (static embeddings are a lookup plus a mean) |
| Paywalled sites yield only the feed summary | Accepted. Those articles contribute fewer sentences |
| Sites blocking non-browser fetches | Accepted. Fall back to feed content and do not spoof browser user agents |
| Feeds with wrong or missing dates | Use `seen_at` for ordering everywhere |
| Checking Cloudflare Access JWTs in the app | Not in v1. Revisit if users ask for defense in depth (`WAGWAN_CF_ACCESS_TEAM` and `WAGWAN_CF_ACCESS_AUD`) |
| Non-English feeds | Not in v1 |
| Topic labels for Discover from feed folders can be vague | Accepted for v1. AI mode may later produce labels |

---

## 16. Rules for agents working on this repo

1. Read this file and `docs/mockup.html` before writing code. Owner override (2026-10-22): implement all milestones without testing; external verification is deferred.
2. Keep it small. Prefer fewer files, fewer crates, and plain functions over traits and generics. Add no abstraction layer for a second implementation that does not exist.
3. Put constants in `tuning.rs`, configuration in `config.rs`, and SQL in `db.rs`. No SQL in handlers.
4. Testing was initially deferred by the owner, then explicitly authorized after implementation on 2026-10-22. Before every commit, run `cargo fmt`, `cargo clippy -- -D warnings` and `cargo test`.
5. No network access in tests. No `unwrap()` outside tests and startup.
6. Every user-visible string is plain and short. No "AI" or "powered by" wording, and no em or en dashes.
7. Do not add accounts, a JS framework, a CSS framework, a sync API, analytics, or telemetry.
8. When this plan is ambiguous, pick the simplest option that satisfies the milestone, note it in the PR, and add a line to section 15.
9. Update this file in the same PR when an implementation choice changes something it specifies.
