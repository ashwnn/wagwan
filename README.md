# Wagwan

Wagwan is a self-hosted, phone-first RSS reader that groups related coverage into stories. **It has no built-in login. Never publish port 8080 directly to the internet.** Keep it on localhost or put an authentication proxy such as Cloudflare Access in front of it.

## Quick start

The default compose file keeps the web port on localhost and stores data in a named volume:

```sh
docker compose up -d
```

Open [http://localhost:8080](http://localhost:8080). Compose builds the local source. The release workflow is configured to publish amd64 and arm64 images to GHCR when a version tag is pushed; a release has not been published from this workspace.

You can import feeds from Settings, or provide an OPML file at first start:

```yaml
services:
  wagwan:
    environment:
      WAGWAN_TZ: America/Vancouver
      WAGWAN_OPML_SEED: /config/feeds.opml
    volumes:
      - wagwan-data:/data
      - ./feeds.opml:/config/feeds.opml:ro
```

To reach Wagwan remotely, configure a Cloudflare Tunnel and Access policy as described in [docs/cloudflare.md](docs/cloudflare.md), then start the `tunnel` profile. Do not bind the app to a public interface.

## Reader

For you ranks stories against your topics and thumbs. Discover shows unusual reporting outside your topics. Open a story to read sentences from its original reporting, tap a sentence for its source, and save stories for later. Search covers story headlines and text. Feed and topic management, mutes, and OPML import/export are in Settings.

The phone layout follows the [clickable reader preview](docs/mockup.html). The application uses server-rendered pages; native form actions work without JavaScript, with sheets and optimistic actions added when JavaScript is available.

## Local development

```sh
cargo build
WAGWAN_DATA_DIR=./data cargo run -- serve
```

The local Potion model downloads on first start and is verified against pinned SHA-256 hashes. For offline operation, prepopulate the six model assets listed in `src/embed.rs` under `data/models/`. Story processing retries if model loading fails; feed management and ingestion remain available.

Implementation of the reader, processing pipeline, optional rewriting and deployment artifacts is present. The development build, Clippy, JavaScript syntax check and 69 offline tests pass. A live 50-feed smoke run also passes with checksum-verified Potion assets. CI now checks mobile browser interactions and native amd64/arm64 container startup and persistence. Real-model clustering quality, deployment and long-duration resource gates remain pending. See [implementation status](docs/progress.md) for outstanding quality and deployment gates.

## Configuration

All configuration uses environment variables. A variable can also use a `_FILE` suffix to read its value from a mounted secret file.

| Variable | Default | Purpose |
|---|---|---|
| `WAGWAN_BIND` | `0.0.0.0:8080` | Address inside the container |
| `WAGWAN_DATA_DIR` | `/data` | Database and model directory |
| `WAGWAN_TZ` | `UTC` | Time zone used for display |
| `WAGWAN_RETENTION_DAYS` | `90` | Article and story retention |
| `WAGWAN_MIN_SCORE` | `40` | Fold lower scoring For you stories |
| `WAGWAN_FETCH_CONCURRENCY` | `16` | Maximum concurrent fetches |
| `WAGWAN_FETCH_MIN_INTERVAL` | `15m` | Minimum feed polling interval |
| `WAGWAN_FETCH_MAX_INTERVAL` | `6h` | Maximum feed polling interval |
| `WAGWAN_OPML_SEED` | unset | OPML file imported on an empty database |
| `WAGWAN_TOPICS_SEED` | unset | Comma-separated starter topics |
| `WAGWAN_MODEL_URL` | potion-base-8M | Embedding model source |
| `WAGWAN_LLM_BASE_URL` | unset | OpenAI-compatible endpoint; unset disables AI |
| `WAGWAN_LLM_API_KEY` | unset | Optional LLM bearer token |
| `WAGWAN_LLM_MODEL` | unset | Required when the LLM endpoint is set |
| `WAGWAN_LLM_CONCURRENCY` | `2` | Maximum concurrent LLM calls |
| `WAGWAN_LOG` | `info` | `tracing` filter |

Durations use integer seconds, minutes, hours, or days, such as `30s`, `15m`, `6h`, or `1d`.

An optional local Ollama service is available through the `llm` profile. Configure `WAGWAN_LLM_BASE_URL=http://ollama:11434/v1` and `WAGWAN_LLM_MODEL=llama3.2:3b` in the Wagwan service before starting that profile:

```sh
docker compose --profile llm up -d
docker compose exec ollama ollama pull llama3.2:3b
```

## Commands

```sh
wagwan serve
wagwan import-opml feeds.opml
wagwan healthcheck
wagwan eval tests/eval
```

## License

MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
docker compose config --quiet
```

The test suite makes no network requests. It includes real template rendering and an in-memory SQLite pipeline with a tiny deterministic model2vec fixture; that fixture does not establish Potion model quality.

To capture the actual rendered fixture reader, lists and Settings at phone width:

```sh
scripts/capture-ui.sh
```

This requires Firefox with working sandbox support and writes PNGs to `docs/screenshots/`. Set `CAPTURE_BROWSER` to a Firefox executable if needed. The command first generates self-contained app HTML under `.build/ui/`; it never substitutes a hand-drawn image for a screenshot.

### Browser and live verification

CI uploads mobile screenshots and live-smoke artifacts under `verification`.
To run the same browser checks locally (development tooling only):

```sh
WAGWAN_UI_ARTIFACT_DIR=.build/ui cargo test --locked
npm install --no-save --package-lock=false playwright@1.62.1
npx playwright install --with-deps firefox
node scripts/browser-smoke.cjs
```

The browser checks render the real Askama templates; their POST responses are mocked.
Rust route tests separately verify persistence and native forms. For a live ingestion check:

```sh
cargo build --locked
python3 scripts/verify-m1.py --smoke --smoke-seconds 65
# Reuse model assets if already downloaded:
python3 scripts/verify-m1.py --smoke --smoke-seconds 65 --models-dir data/models
```

Omit `--smoke` for the full one-hour ingestion gate. A short smoke run does not
satisfy the one-hour or 24-hour resource requirements.
