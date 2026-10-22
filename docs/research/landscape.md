# Self-hosted RSS reader landscape (research as of 2026-10-22)

Scope: web research only, no code run. Star counts, licenses and descriptions come from the GitHub repo pages as fetched on 2026-10-22. Where a page did not show a last-commit date, the field says "not shown" rather than guessing. Search coverage is partial; treat "not found" as "not found in this search", not "does not exist".

Headline finding: I found no Rust RSS reader or aggregator from 2025-2026 that does embedding-based story clustering with AI. The closest Rust projects are plain TUI readers with no dedup or AI. The clustering-plus-AI space is occupied almost entirely by Python, TypeScript and Go projects, most with small star counts, plus a few large ones (Horizon, Kagi News) that are not feed readers in the classic sense.

---

## 1. Rust readers and aggregators

| Project | Repo | Stars (approx) | License | Last activity | Type | Reusable or worth learning from |
|---|---|---|---|---|---|---|
| newsboat | https://github.com/newsboat/newsboat | 3.9k | MIT | not shown | Terminal reader. Repo is mostly C++17; README requires Rust 1.94+ and the repo has a `rust/` directory (what Rust code does is UNVERIFIED) | Mature feed-parsing and TUI workflow, and a long-running GReader client. Study its query/filter language and its update/cache model |
| russ | https://github.com/ckampfe/russ | 270 | AGPL-3.0-or-later | not shown | Terminal reader, pure Rust, offline-first | SQLite storage via rusqlite (single `feeds.db`), vim-style keys. Filtering is read-state only; no dedup, no keyword filter, no AI. Good reference for a minimal Rust data layer |
| rss.rs (ntsk) | https://github.com/ntsk/rss.rs | 0 | MIT | not shown | Terminal reader, Rust | RSS 1.0/2.0 and Atom, OPML import/export, reader-mode extraction, auto-refresh. Small, readable reference for OPML and feed handling |
| feedr | https://docs.rs/crate/feedr/0.2.0 (crates.io, 3 releases) | UNVERIFIED | UNVERIFIED | UNVERIFIED | Terminal reader | Found on crates.io only; repo and license not checked |
| tuifeed | https://docs.rs/crate/tuifeed/0.2.0 (9 releases on lib.rs) | UNVERIFIED | UNVERIFIED | UNVERIFIED | Terminal reader | Found on crates.io only |
| reedy | https://lib.rs/crates/reedy | UNVERIFIED | UNVERIFIED | UNVERIFIED | Terminal reader | Found on crates.io only |
| rss-tui | https://docs.rs/crate/rss-tui/0.7.0 | UNVERIFIED | UNVERIFIED | UNVERIFIED | Terminal reader | Found on crates.io only |

Notes:
- No axum-based Rust RSS reader with a web UI surfaced in search. A selfhostyourself.com list of axum projects contained no feed readers (it listed Pods-Blitz, Minne and Stump, none of them readers).
- No Rust project with embeddings or story clustering surfaced. Not exhaustively checked.
- Rust feed-parsing crates (feed-rs, rss, atom_syndication) were not evaluated in this pass. UNVERIFIED as to current maintenance.

---

## 2. Non-Rust projects closest to "cluster into stories + optional AI"

| Project | Repo | Stars | License | Language | What it does | What it does well | What it lacks / caveats |
|---|---|---|---|---|---|---|---|
| Horizon (Thysrael) | https://github.com/Thysrael/Horizon | 9.1k | MIT | Python | Pulls HN, RSS, Reddit, Telegram, X, GitHub; dedups across platforms; LLM scores and filters against user profiles; bilingual daily briefings; MCP server; Docker | Strongest "ranking by interest + AI" implementation in this list, with large adoption | Briefing product, not a reader. A fork exists at https://github.com/Po1nt9/Horizon (0 stars) |
| AIHOT (KKKKhazix) | https://github.com/KKKKhazix/AIHOT | 25 | MIT (code; brand excluded) | TypeScript, Fastify, Postgres, pg-boss, Docker Compose | Collects from RSS, web lists, JSON APIs, X, WeChat; embeddings find clustering candidates over 2 weeks; a model decides same-event vs follow-up; heat is computed per event, with each independent source counted once within 48h | Best written-down design for event-level clustering and source-weighted heat. Prompts live in an `industry/` folder, so the pipeline is swappable | Chinese-first output; 2 commits shown; OpenAI-compatible key required |
| Ember (brandonhon) | https://github.com/brandonhon/ember | 5 | MIT | Go backend, Svelte UI, SQLite | Self-hosted RSS/Atom reader; cross-feed dedup shown as "Also in N feeds"; optional summaries (Ollama default, OpenAI-compatible, or Claude); optional ad-stripping; Fever-compatible API; AI opt-out via env var | Closest to our product shape: reader first, AI optional, dedup across feeds, Fever API | Dedup is "also in N feeds", not story clustering; very new, small |
| NewsBucket (SegoCode) | https://github.com/SegoCode/NewsBucket | 0 | PolyForm Noncommercial + AGPL-3.0 | not stated | Reads feeds, LLM clusters same-story articles, keeps only stories covered by 2+ outlets, outputs JSON on GitHub Actions; Telegram miniapp and GitHub Pages | Clean "story = cluster of outlets" model; multilingual via translation step | Not a reader; PolyForm Noncommercial limits commercial reuse of code; OpenCode/Gemini dependency |
| Meridian (CartesianXR7) | https://github.com/CartesianXR7/Meridian | 4 | MIT | Python | Groups headlines with DBSCAN over SentenceTransformers embeddings (cached); NER and VADER sentiment | Classical embedding-plus-DBSCAN baseline, easy to read | No LLM, no reader UI; tiny |
| rfrenoy/rss-reader | https://github.com/rfrenoy/rss-reader | 2 | not shown | TypeScript (inferred from file names) | Daily markdown digests; dedup by cosine similarity >= 0.85 (configurable) using Ollama `nomic-embed-text`; novelty score; LLM summaries and tags (Ollama `qwen2.5:7b` default or Claude) | Clear local-embedding dedup plus novelty ranking; a useful default-threshold reference | Digest output, not an interactive reader; license missing |
| Pace (av) | https://github.com/av/pace | 8 | MIT | TypeScript on Bun (inferred from tsconfig and bun.lock) | Self-hosted dashboard; transforms for clustering, filtering, dedup, time-decay, keyword scoring; optional LLM transforms; keyboard reader; agent-readable brief over HTTP | Declarative transform pipeline (ranking is plain code, LLM optional). Good model for "ranking by interests" that stays inspectable | Dashboard-first; small |
| RSSbrew (yinan-c) | https://github.com/yinan-c/RSSbrew | 295 | AGPL-3.0 | Python (inferred from file layout) | Merges feeds, regex and boolean filters on link/title/description, daily or weekly AI digests, one-line and longer per-article summaries via any OpenAI-compatible model | Explicit filter grammar; Docker; self-hosted | Self-described as still in development; digest-oriented, no clustering |
| JARR | https://github.com/jaesivsm/JARR | 129 | AGPL-3.0 | Python | Web aggregator and reader; clustering of articles by links or content with TF-IDF; hosted instance at app.jarr.info | Shows a non-AI clustering baseline in a real reader. Last activity not shown | Commit count 2,666 but date not shown; TF-IDF only |
| Stringer | https://github.com/stringer-rss/stringer | UNVERIFIED | MIT (per Railway page) | Ruby on Rails 8, PostgreSQL, Hotwire | Self-hosted single-user reader, "no social feed or algorithm"; OPML; Fever API at `/fever`; SSRF protection on fetches | Clean minimal reader with Fever; a counter-example to the algorithmic approach | No clustering, no AI |
| Kagi News / kite-public | https://github.com/kagisearch/kite-public | 1.1k | Code MIT; application data CC BY-NC 4.0 | JS front end (Vite); curated feed lists in JSON | Hosted, free AI news app; community-curated feed sources in a public repo; AI summaries with citations | Shows the "stories with citations across many sources" UX at scale. Clustering method not described in the article read (UNVERIFIED). The data license blocks commercial reuse of the feed list | Not self-hostable as a reader; the app data is served from kite.kagi.com |
| NewsBlur story clustering | https://forum.newsblur.com/t/story-clustering-automatically-group-duplicate-stories-across-your-feeds/13512 | n/a (hosted) | not verified | Python | Clusters duplicates via title matching plus Elasticsearch more_like_this; shows highest-scoring version with other sources underneath; cached 14 days; on by default (announced March 2026, iOS launch April 3, 2026) | Mainstream reader shipped clustering; user feedback is a free requirements list | False positives (comic strips and academic papers clustered together); read items showing as unread inside clusters |
| Condensit | not found | n/a | n/a | n/a | n/a | Search surfaced only academic work on RSS clustering via fuzzy equivalence relations (https://research.tudelft.nl/en/publications/generating-fuzzy-equivalence-classes-on-rss-news-articles-for-ret) and maximal spanning trees (https://research.tudelft.nl/en/publications/using-maximal-spanning-trees-and-word-similarity-to-generate-hier). Useful as citations, not as a project |
Not found or not checked: Condensit (no project located), Feedbin and Miniflux clustering (neither has it; see section 3).

---

## 3. User complaints about existing self-hosted readers (duplicates, overload, relevance)

1. **Miniflux: no native dedup, and the architecture makes it hard.** Feature request "Deduplicate Feature", open. Users write Python scripts against the Miniflux API (URL match, exact title, then Levenshtein fuzzy match). A maintainer notes entries map one-to-one to feeds, so an entry cannot belong to several feeds without broad changes. Source: https://github.com/miniflux/v2/issues/797
2. **Wire-story duplicates force manual triage.** A Miniflux user marks near-identical AP/wire copies read one by one, considered paid readers (Inoreader, Feedly) for their dedup, but did not want to send self-hosted RSS-Bridge feeds to a third party. Source: https://next.lemm.ee/post/4361321
3. **Duplicates at scale are the top pain.** A commenter with 100+ feeds calls avoiding re-reading the same entry "the single most important feature"; another wants whole duplicate groups dropped at once via topic clustering. Source: https://lemmy.linuxuserspace.show/post/128556
4. **Relevance and junk.** Commenters say most feeds mix wanted and unwanted content, and that "so many RSS feeds are full of junk"; rule-based filtering in Feedbro works but "takes some time to set up and fine-tune". Source: https://lemmy.linuxuserspace.show/post/128556 (thread date not shown; treat as older, UNVERIFIED)
5. **Excerpt-only feeds.** Feeds increasingly carry excerpts, so readers are less useful without full-text fetching. Source: https://lemmy.linuxuserspace.show/post/128556
6. **Tiny Tiny RSS: the opposite request.** A user on TT-RSS 17.12 wants the same article shown under every feed that carried it, not just the first. Shows that dedup policy must be a user choice. Source: https://discourse.tt-rss.org/t/article-duplication-desired/1967
7. **Closed aggregators are too restrictive.** Users of Feedly and Google News say they cannot define their own sources or drop negatively framed sources; Ground.news and Google News cluster but lock sources. Source: https://next.lemm.ee/post/10653841
8. **Clustering done badly is also a complaint.** NewsBlur's clustering launch drew reports of unrelated items grouped (comics, academic papers on the same topic), read items reappearing inside clusters, and a request for collapsing by default. Source: https://forum.newsblur.com/t/story-clustering-automatically-group-duplicate-stories-across-your-feeds/13512

---

## 4. Interop: Fever, GReader and Miniflux API

Client coverage, as listed by the projects themselves:

- **GReader (Google Reader API)**: the broadest client set. FreshRSS README lists, for GReader: Android (Capy Reader, Readrops, FeedMe, FocusReader, Read You, Your News, Fluent Reader Lite, FeedFlow, FriRSS); Windows/Linux/macOS (Fluent Reader, RSS Guard, NewsFlash, Newsboat, FluentFlame); Apple (Vienna RSS, Readkit, Reeder Classic, lire, NetNewsWire). The README calls GReader the "best" option. Source: https://github.com/FreshRSS/FreshRSS
  - Also from the FreshRSS GReader doc: Newsboat 2.24+, News+ (Android), FreshRSS-Notify (Firefox). Source: https://freshrss.github.io/FreshRSS/en/developers/06_GoogleReader_API.html (via https://github.com/FreshRSS/FreshRSS/blob/edge/docs/en/developers/06_GoogleReader_API.md)
  - Core endpoints seen in the FreshRSS doc: `ClientLogin`, `subscription/list`, `subscription/edit`, `unread-count`, `tag/list`, `token`, `stream/contents/reading-list`.
- **Fever API**: smaller and simpler, JSON only. Clients listed by FreshRSS as Fever: Unread (iOS), Fiery Feeds (iOS). Miniflux lists Fever clients: Reeder Classic, Unread, FeedMe, FocusReader, NewsFlash, Fluent Reader, Lire. Read You also lists Fever support in its repo. Caveat from FreshRSS: clients "understand the Fever API a bit differently". Sources: https://github.com/FreshRSS/FreshRSS, https://miniflux.app/docs/fever.html, https://github.com/ReadYouApp/ReadYou. Fever auth detail (md5 of user:password) was not verified; feedafever.com was blocked by robots for this crawler. UNVERIFIED.
- **Miniflux native API**: REST under `/v1/`, auth via basic auth or per-app API keys in `X-Auth-Token`; OPML import/export included. Official clients for Go and Python. Third-party clients are few: Lire, FluxNews, PoweReader, Panda Reader (desktop). Source: https://miniflux.app/docs/api.html and https://miniflux.app/docs/apps.html
- **Read You** (Android): supports Fever, Google Reader and FreshRSS; Miniflux, TT-RSS, Inoreader, Feedly, Feedbin listed as not yet supported. Source: https://github.com/ReadYouApp/ReadYou
- **Reeder**: Reeder Classic (iOS/macOS) appears under GReader and Fever in the lists above. Reeder 5 (current) sync support: UNVERIFIED.
- **NetNewsWire**: listed under GReader in the FreshRSS README. Its own help pages could not be fetched. Current supported services: UNVERIFIED beyond the FreshRSS listing.
- **Stringer** (Rails): Fever at `/fever`. Source: https://railway.com/deploy/stringer.md

Answer to "which one API gives the most client coverage for least effort?": GReader gives the most coverage (about 19 apps listed in the FreshRSS README across Android, desktop and Apple platforms, including Reeder Classic, NetNewsWire, Read You, FeedMe, Newsboat; the GReader doc lists a few more). Fever is cheaper to implement and gives a handful of iOS-centric clients (Unread, Fiery Feeds, Lire, Reeder Classic). Effort estimate for GReader versus Fever is UNVERIFIED; GReader needs stable item IDs, continuation tokens and a token endpoint, and Fever needs one endpoint family with md5 auth. Recommendation: implement GReader first for coverage, Fever second as a cheap add-on for iOS apps. Ship our own native API later.

---

## 5. Packaging norms for self-hosted Rust apps

- **Single static binary**: standard for Rust readers. Static musl builds let you use a `scratch` image. Image size figures from a blog post (not a benchmark, UNVERIFIED): Debian-slim about 150MB; Alpine about 25MB; distroless `cc-debian12` about 22MB; scratch with static musl about 8MB. Source: https://oneuptime.com/blog/post/2026-01-07-rust-minimal-docker-images/markdown. Measure our own before publishing numbers.
- **Config**: Miniflux (Go) accepts both env vars and a `KEY=VALUE` config file, with env overriding file values, and `_FILE` suffixes for secrets. Source: https://miniflux.app/docs/configuration.html. Stringer uses Rails conventions. TOML is not a norm in this set of projects; it is a Rust-ecosystem habit. UNVERIFIED as a norm.
- **Single-user default**: Stringer and Ember both target a single user with SQLite or Postgres, and split the worker from the web process (Stringer runs a worker and a scheduler, per its Railway template).
- **Licenses seen**:
  - AGPL-3.0: FreshRSS (per README), JARR, RSSbrew, russ (AGPL-3.0-or-later), Lemmy, NewsBucket (with PolyForm NC).
  - GPL-3.0: TT-RSS, Read You.
  - Apache-2.0: Miniflux.
  - MIT: newsboat, Feedbin, Ember, Stringer, Horizon, AIHOT, Meridian, Pace, rss.rs.
  - Mixed: Kagi Kite (code MIT, data CC BY-NC).
  - Why: reasoning below is general knowledge, not from a fetched source. AGPL keeps hosted forks of a self-hosted product sharing their changes, and is common among self-hosted web apps. MIT and Apache maximise reuse, including in other clients and libraries. Read You and FreshRSS sit under copyleft even though their clients are native apps.

---

## Implications for our project

- Build the differentiator on the event-level design in AIHOT (embedding candidate search, same-event check, source counted once per window) rather than on Miniflux-style URL and title dedup. Ember's "Also in N feeds" is the minimal baseline to beat.
- Keep clustering and ranking optional and inspectable: Pace's transform pipeline and rfrenoy's novelty score show a path where non-AI embeddings alone do most of the work, with an LLM only for summaries or edge cases.
- Expose dedup as a user-chosen policy, with a per-feed or global toggle. TT-RSS users want duplicates shown per feed; Miniflux users want them collapsed. NewsBlur's false positives show that a strict threshold and a visible "why grouped" view matter.
- Implement GReader first for coverage (Reeder Classic, NetNewsWire, Read You, FeedMe, Newsboat and more), then Fever as a cheap add-on for iOS clients. Skip a bespoke API until the core is stable.
- The Rust gap is real in this search: the only Rust readers are small TUIs with no dedup or AI. A web-first Rust reader with axum, SQLite and optional local embeddings would be distinct; the closest prior art to learn from is Ember (Go) and AIHOT (TypeScript), with russ (Rust, SQLite) for the data layer.
- Use a permissive license (MIT or Apache-2.0) if the goal is wide reuse and client adoption; use AGPL-3.0 if the priority is keeping a hosted fork from taking the work closed. Our choice should be made deliberately, since the niche has both.
- Avoid the NewsBucket-style PolyForm Noncommercial license and Kagi-style CC BY-NC data: both block reuse of the feed lists and code that competitors would want.
- Ship with a no-AI default that still clusters (embeddings, or even TF-IDF as JARR does), so the product works without any model. Document the cost and privacy trade-off of hosted LLM providers, which Ember and rfrenoy both do.
- Measure binary and image size on our own build before claiming numbers; the only figures found are from one blog post (UNVERIFIED).

---

## Caveats

- Most GitHub pages did not show last-commit dates, so "last activity" is UNVERIFIED for almost all projects. Star counts are approximate and fetched on 2026-10-22.
- Several Rust crates (feedr, tuifeed, reedy, rss-tui) were found only on crates.io and not checked further.
- Condensit was not located as a project.
- Stars for NewsBucket and rfrenoy are very low; their presence in the list is for design ideas, not adoption.
- Reddit and Hacker News were searched through a general search engine; direct thread access was blocked for some pages (for example, Anubis protection on one Lemmy post). Complaints are quoted from the pages that could be read.
- Fever auth details, Reeder 5 and NetNewsWire current sync lists, and GReader vs Fever effort estimates are UNVERIFIED.

## Sources (main)

- https://github.com/newsboat/newsboat
- https://github.com/ckampfe/russ
- https://github.com/ntsk/rss.rs
- https://github.com/Thysrael/Horizon
- https://github.com/KKKKhazix/AIHOT
- https://github.com/brandonhon/ember
- https://github.com/SegoCode/NewsBucket
- https://github.com/CartesianXR7/Meridian
- https://github.com/rfrenoy/rss-reader
- https://github.com/av/pace
- https://github.com/yinan-c/RSSbrew
- https://github.com/jaesivsm/JARR
- https://github.com/stringer-rss/stringer (via https://railway.com/deploy/stringer.md)
- https://github.com/kagisearch/kite-public
- https://www.niemanlab.org/2025/10/kagi-news-is-an-ai-powered-app-for-keeping-up-with-the-world/
- https://forum.newsblur.com/t/story-clustering-automatically-group-duplicate-stories-across-your-feeds/13512
- https://github.com/miniflux/v2/issues/797
- https://next.lemm.ee/post/4361321
- https://lemmy.linuxuserspace.show/post/128556
- https://next.lemm.ee/post/10653841
- https://discourse.tt-rss.org/t/article-duplication-desired/1967
- https://github.com/FreshRSS/FreshRSS
- https://freshrss.github.io/FreshRSS/en/developers/06_GoogleReader_API.html
- https://miniflux.app/docs/api.html
- https://miniflux.app/docs/apps.html
- https://miniflux.app/docs/fever.html
- https://miniflux.app/docs/configuration.html
- https://github.com/ReadYouApp/ReadYou
- https://github.com/tt-rss/tt-rss
- https://github.com/feedbin/feedbin
- https://oneuptime.com/blog/post/2026-01-07-rust-minimal-docker-images/markdown
