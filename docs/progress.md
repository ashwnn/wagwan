# Implementation status

Updated 2026-10-22.

The owner first authorized continuing through all milestones with external gates pending, then requested implementation without testing. After the development build succeeded, the owner explicitly authorized verification and screenshot capture. The results below supersede the earlier testing deferral.

## Implemented source

| Area | Implementation |
|---|---|
| Ingestion | RSS/Atom/JSON Feed parsing, conditional requests, adaptive polling, backoff, per-host and global HTTP limits, manual redirect accounting, page extraction and feed fallback |
| Storage | SQLite WAL, migrations, read pool, transactional imports, per-sentence/source storage, FTS5 index, saved-story retention, incremental vacuum |
| Embeddings | Local model2vec Potion loader with a pinned revision and SHA-256 asset verification, normalized article/topic/sentence vectors, asynchronous retry when assets are unavailable |
| Stories | Permanent online assignment with time decay and fact-token gate, centroids, story closing, MinHash copies and containment, normalized facts and uniqueness |
| Ranking | For you and Discover formulas, taste from votes, topic and feed mutes, labels/reasons, hourly decay and work-triggered recomputation |
| Reader | For you, Discover, Saved, search, sentence-linked story reader, source sheets, new-since-read highlighting, original article links, native forms, optimistic actions and undo |
| Settings | Topics, feeds, retries, OPML import/export, mutes/unmute, optional model status, version, data size and fetch time in the configured IANA zone |
| Story text | Extractive lede/MMR selection, cached original-article hash, text fragments, optional cited rewrites with source/number/name/vector verification and disagreements |
| Background work | CPU work in blocking workers, ordered pending processing with per-article failures isolated, bounded active article cache and paged scoring, model-independent retention |
| Evaluation command | Native SemEval annotation tables and recursively scraped article JSON, English filtering, title plus first 120 words, threshold precision/recall/F1 report |
| Deployment | Local source Compose build, musl/scratch Dockerfile, non-root data ownership, binary healthcheck, optional tunnel and Ollama, manifest/icons, CI and multi-architecture tagged release workflow |

## Implementation decisions

- Migration 0002 uses a contentful FTS5 index so individual story updates, feed deletion and retention can remove index rows safely.
- Migration 0003 records vote timestamps so newly voting on an older saved story contributes to the 90-day taste window.
- Changed-source cleanup invalidates cached text and repairs centroid, coverage, title and search index. In-flight rewrites are stored only when the original-article hash still matches.
- Full article text is never exposed by the web routes. Only selected story sentences and source links are rendered.
- Extractive text is built by the background pipeline after original-article changes, rather than running embedding/selection work in a request handler.
- `time-tz` is an additional production dependency for IANA zone rules inside a scratch image. `tower` is a pre-existing development dependency for the earlier in-process M1 route tests.
- model2vec-rs is configured for local loading with its optional Hub client disabled. Wagwan downloads and checks the assets itself. Upstream license/source references: [model2vec-rs](https://github.com/MinishLab/model2vec-rs), [Potion model](https://huggingface.co/minishlab/potion-base-8M).
- Compose builds the checked-out source by default; the configured GHCR release has not been published from this workspace.

## Build and deferred validation

Source files have been formatted. The owner downloaded the model and time-zone dependencies. The expanded application now compiles with `cargo build --offline` after fixing the outlet list type annotation, an evaluator string borrow, and an unstable `str::as_str` call.

The expanded development build has succeeded. Offline verification now passes as detailed below. An actual browser screenshot, real-model evaluation, live HTTP, container execution and duration/resource gates remain unverified; attempts encountered the environment restrictions listed below.

The original gates remain listed in PLAN.md for future validation: the one-hour 50-feed run, calibrated SemEval F1, copy accuracy, day-long real-data/manual cluster review, mobile layouts and Lighthouse, taste behavior, optional Ollama behavior, clean Compose startup, image size, 24-hour 400-feed memory/CPU observations, and an end-to-end Cloudflare Access setup. Their status is pending, not passed.

## Integration follow-up

- Fixed the Settings template's feed-title type mismatch and prevented folder labels from being offered as mute-topic actions. Unsafe legacy feed URLs render as text.
- Original-article assignment now invalidates cached text and its source hash atomically, preventing an in-flight rewrite from installing stale content. Migration 0002 rebuilds existing story FTS entries when changing index storage.
- Extractive MMR now preserves only the original lede sentence IDs when ordering later selections.
- The healthcheck uses the matching IPv4/IPv6 loopback address for wildcard binds and bypasses proxy environment settings for its local request.
- The dependency blocker was subsequently resolved by the owner; the expanded development build now succeeds. No tests were added or run.

## Authorized verification results

Passed commands:

- `cargo build --offline`: successful development build.
- `cargo fmt` and the final `cargo clippy --offline --all-targets --all-features -- -D warnings`: successful, with no Clippy warnings.
- `cargo test --offline`: 67 passed (64 unit/in-process tests and 3 CLI integration tests), 0 failed.
- `node --check src/web/static/app.js`: successful.
- `docker compose config --quiet`: successful configuration validation; this does not prove a container can run.
- `bash -n scripts/capture-ui.sh`: successful screenshot-helper syntax validation.

The expanded tests cover algorithm boundaries and formulas, synthetic copy pairs, normalized facts, native SemEval parsing, source fragments/MMR ordering, actual reqwest response parsing for 200/304/error/oversized bodies without sockets, schema upgrade/search, stale AI rejection, save/retention, votes, native reader and Settings actions.

The full offline integration uses actual feed parsing, cleanup, the model2vec tokenizer/pooling engine with tiny controlled 256-dimensional fixture weights, and shared in-memory SQLite. It verifies two event clusters, one copy, original-only source text, new-detail selection, search, mutually exclusive list membership, idempotence and isolation of an unembeddable article. The fixture weights are not the Potion model and provide no SemEval quality evidence.

A canned response claiming 7000 against a source saying 3200 goes through production verification, persists in SQLite, and is returned by the reader with `verified=false`. This does not prove an actual Ollama server works.

Real template-rendered fixture pages are available in `.build/ui/` (`for-you.html`, `discover.html`, `saved.html`, `search.html`, `story.html`, `story-discover.html`, `settings.html`). CSS and JS are embedded for file-based viewing. The producing in-process route test passed.

Blocked checks, with observed evidence:

- Local listener creation: `Operation not permitted`, so the one-hour HTTP run cannot start here.
- Docker daemon: permission denied connecting to `/var/run/docker.sock`, so no image build, clean Compose startup or container footprint measurement is claimed.
- Firefox: snap wrapper paths are read-only; directly invoking the installed binary with a workspace profile still fails namespace/sandbox setup with `EPERM`. No PNG was created. Firefox's security sandbox was left enabled.
- Browser screenshot, 360/390px layout inspection, light/dark matching and Lighthouse remain pending. No alternate browser/HTML renderer is installed. `scripts/capture-ui.sh` provides a repeatable capture path when Firefox can run.
- Potion assets and SemEval evaluation data are absent locally. No real-model F1 calibration, real-publisher copy benchmark, day-long feed review or 24-hour 400-feed measurement has run.
- No Cloudflare tunnel token is configured, so the Access guide has not been followed end to end in an account.

The application is not claimed fully verified or released.

The CLI integration tests launch the actual binary with isolated child environments. They verify `_FILE` precedence (including CRLF secret files), repeated import counts, rejection of invalid startup configuration before any database is opened, and failure rather than silently falling back when a secret file is missing. A polling-interval error was corrected to identify the offending environment variable.

## Release-readiness follow-up (2026-10-22)

This section supersedes the earlier environment limitations where new evidence is available.

- Fixed the initial GitHub CI failure on stable Rust 1.99: vector decoding now uses fixed-size array chunks. Formatting, Clippy with all targets/features and warnings denied, and 69 Rust tests pass locally.
- Fixed the mobile More sheet: its menu is in normal flow, height is bounded with scrolling, it receives keyboard focus, and it has an explicit Close action.
- Unversioned CSS, JS and manifest responses now require cache revalidation, so an upgrade cannot leave the browser using year-old assets.
- Corrected generated fixture HTML to run JavaScript after the document body, matching the production deferred script. The previous fixture capture did not exercise initialized sheet/toast references.
- Added `scripts/browser-smoke.cjs`: seven actual Askama views at 360px and 390px in light/dark, overflow checks, source and More sheets, focus restoration, save and undo, and screenshots. Browser POST responses are mocked; the Rust route tests cover native actions and persistence. GitHub CI runs it and retains the artifacts.
- CI builds native amd64 and arm64 images, checks container health, imports feeds, restarts and verifies the import remains idempotent. Tagged image publishing depends on these checks. Docker builds use Cargo.lock.
- Live local HTTP smoke passed with 50 synthetic feeds. A 65-second run using all six checksum-verified Potion assets stored 50 articles, assigned all 50 to stories, stored 100 story-text rows and observed conditional 304 requests. These fixtures prove plumbing, not clustering quality. `scripts/verify-m1.py --models-dir PATH` can reuse downloaded assets in an isolated run.
- The actual Zenodo `final_eval_data.csv` exposed an evaluator bug: published `link1`/`link2` and `url1_lang`/`url2_lang` headers were not recognized. These are now supported, with a regression test covering both cross-language directions and missing article records.

Still required before claiming all PLAN.md milestones complete:

1. Full one-hour ingestion run and 24-hour 400-feed RAM/CPU measurement.
2. Complete SemEval article JSON alongside the downloaded annotation table, then measured F1 and threshold calibration. An annotation CSV alone contains URLs, not article text. No F1 score is claimed.
3. Real-publisher duplicate accuracy and manual event-cluster review.
4. Successful browser/container CI evidence, Lighthouse >=95, and optional Ollama integration.
5. An end-to-end Cloudflare Access deployment in the owner's account.

Local Firefox could launch but content processes failed OS sandbox namespace setup (`uid_map: EPERM`). No sandbox was disabled and no local browser pass is claimed. CI browser and image results must be read from the specific run; adding checks does not mean they passed.
