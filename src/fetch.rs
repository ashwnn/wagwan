//! HTTP fetching and feed parsing for the ingest pipeline.

use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use feed_rs::{model::Feed, parser};
use quick_xml::{Reader, events::Event};
use reqwest::{Client, Response, StatusCode, Url, header, redirect::Policy};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::tuning;

pub use crate::tuning::FETCH_MAX_BODY_BYTES as MAX_BODY_BYTES;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(tuning::FETCH_TIMEOUT_SECS);

#[derive(Debug, Clone)]
pub struct FeedItem {
    pub guid: String,
    pub url: Option<String>,
    pub title: String,
    pub published_at: Option<i64>,
    pub content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FeedFetch {
    pub not_modified: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub ttl_secs: Option<u64>,
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub items: Vec<FeedItem>,
}

pub fn http_client(user_agent: &str) -> Result<Client> {
    Ok(Client::builder()
        .user_agent(user_agent)
        .timeout(REQUEST_TIMEOUT)
        .redirect(Policy::none())
        .build()?)
}

async fn fetch_feed_limited(
    client: &Client,
    limits: &FetchLimits,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<FeedFetch> {
    let (response, permit) = get_limited(client, limits, url, etag, last_modified)
        .await
        .with_context(|| format!("fetching feed {url}"))?;
    let result = parse_feed_response(response, etag, last_modified).await;
    drop(permit);
    result
}

async fn parse_feed_response(
    response: Response,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<FeedFetch> {
    let new_etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    let new_modified = response
        .headers()
        .get(header::LAST_MODIFIED)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(not_modified_result(
            new_etag,
            new_modified,
            etag,
            last_modified,
        ));
    }
    anyhow::ensure!(
        response.status().is_success(),
        "feed returned HTTP {}",
        response.status()
    );
    let body = read_limited(response).await?;
    parse_feed_body(&body, new_etag, new_modified)
}

fn not_modified_result(
    response_etag: Option<String>,
    response_last_modified: Option<String>,
    stored_etag: Option<&str>,
    stored_last_modified: Option<&str>,
) -> FeedFetch {
    FeedFetch {
        not_modified: true,
        etag: response_etag.or_else(|| stored_etag.map(str::to_owned)),
        last_modified: response_last_modified.or_else(|| stored_last_modified.map(str::to_owned)),
        ttl_secs: None,
        title: None,
        site_url: None,
        items: Vec::new(),
    }
}

pub(crate) fn parse_feed_body(
    body: &[u8],
    etag: Option<String>,
    last_modified: Option<String>,
) -> Result<FeedFetch> {
    let feed = parser::parse(body).context("parsing feed")?;
    Ok(feed_result(feed, etag, last_modified, parse_ttl_secs(body)))
}

/// GET a resource while enforcing global and per-host request limits for every
/// hop. Cache validators are resent only to the original origin.
pub(crate) async fn get_limited(
    client: &Client,
    limits: &FetchLimits,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<(Response, FetchPermit)> {
    let original = Url::parse(url)?;
    let mut current = original.clone();
    let mut redirect_count = 0usize;
    loop {
        let permit = limits.acquire(current.as_str()).await?;
        let same_origin = same_origin(&original, &current);
        let mut request = client.get(current.clone());
        if same_origin {
            if let Some(value) = etag.filter(|s| !s.is_empty()) {
                request = request.header(header::IF_NONE_MATCH, value);
            }
            if let Some(value) = last_modified.filter(|s| !s.is_empty()) {
                request = request.header(header::IF_MODIFIED_SINCE, value);
            }
        }
        let response = request.send().await?;
        if response.status() == StatusCode::NOT_MODIFIED
            || !is_followable_redirect(response.status())
        {
            return Ok((response, permit));
        }
        let Some(location) = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
        else {
            return Ok((response, permit));
        };
        anyhow::ensure!(
            !redirect_limit_reached(redirect_count),
            "HTTP redirect limit exceeded"
        );
        let next = resolve_redirect(&current, location)?;
        drop(response);
        drop(permit);
        current = next;
        redirect_count += 1;
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str()
            .zip(b.host_str())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        && a.port_or_known_default() == b.port_or_known_default()
}

fn resolve_redirect(current: &Url, location: &str) -> Result<Url> {
    Ok(current.join(location)?)
}

fn redirect_limit_reached(redirect_count: usize) -> bool {
    redirect_count >= tuning::FETCH_MAX_REDIRECTS
}

fn is_followable_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

pub(crate) fn can_append_chunk(current_len: usize, chunk_len: usize) -> bool {
    current_len.saturating_add(chunk_len) <= MAX_BODY_BYTES
}

pub(crate) fn validate_content_length(length: Option<u64>) -> Result<()> {
    anyhow::ensure!(
        length.is_none_or(|length| length <= MAX_BODY_BYTES as u64),
        "HTTP body exceeds 5 MB limit"
    );
    Ok(())
}

fn feed_result(
    feed: Feed,
    etag: Option<String>,
    last_modified: Option<String>,
    ttl_secs: Option<u64>,
) -> FeedFetch {
    let title = feed.title.map(|title| title.content);
    let site_url = alternate_url(
        feed.links
            .iter()
            .map(|link| (link.href.as_str(), link.rel.as_deref())),
    )
    .map(str::to_owned);
    let items = feed
        .entries
        .into_iter()
        .map(|entry| {
            let url = alternate_url(
                entry
                    .links
                    .iter()
                    .map(|link| (link.href.as_str(), link.rel.as_deref())),
            )
            .map(str::to_owned);
            let title = entry.title.map(|t| t.content).unwrap_or_default();
            let content = entry
                .content
                .and_then(|c| c.body)
                .or_else(|| entry.summary.map(|s| s.content));
            let guid = if entry.id.trim().is_empty() {
                url.clone().unwrap_or_default()
            } else {
                entry.id
            };
            let published_at = entry.published.or(entry.updated).map(|d| d.timestamp());
            FeedItem {
                guid,
                url,
                title,
                published_at,
                content,
            }
        })
        .filter(|i| !i.guid.is_empty())
        .collect();
    FeedFetch {
        not_modified: false,
        etag,
        last_modified,
        ttl_secs,
        title,
        site_url,
        items,
    }
}

/// RSS `<ttl>` is specified in minutes. Read it from the original document so
/// feed-rs versions that omit this RSS extension still preserve the value.
fn parse_ttl_secs(body: &[u8]) -> Option<u64> {
    let mut reader = Reader::from_reader(body);
    let mut buffer = Vec::new();
    let mut depth = 0usize;
    let mut channel_depth = None;
    let mut reading_ttl = false;
    let mut ttl_text = Vec::new();

    loop {
        match reader.read_event_into(&mut buffer).ok()? {
            Event::Start(element) => {
                let name = element.local_name();
                let name = name.as_ref();
                if channel_depth.is_none() && name.eq_ignore_ascii_case(b"channel") {
                    channel_depth = Some(depth + 1);
                } else if channel_depth == Some(depth) && name.eq_ignore_ascii_case(b"ttl") {
                    reading_ttl = true;
                    ttl_text.clear();
                }
                depth += 1;
            }
            Event::Text(text) if reading_ttl => ttl_text.extend_from_slice(text.as_ref()),
            Event::CData(text) if reading_ttl => ttl_text.extend_from_slice(text.as_ref()),
            Event::End(element) => {
                depth = depth.saturating_sub(1);
                let name = element.local_name();
                let name = name.as_ref();
                if reading_ttl && name.eq_ignore_ascii_case(b"ttl") {
                    reading_ttl = false;
                    if let Ok(value) = std::str::from_utf8(&ttl_text)
                        && let Ok(minutes) = value.trim().parse::<u64>()
                    {
                        return Some(minutes.saturating_mul(60));
                    }
                }
                if channel_depth == Some(depth + 1) && name.eq_ignore_ascii_case(b"channel") {
                    channel_depth = None;
                }
            }
            Event::Eof => break,
            _ => (),
        }
        buffer.clear();
    }
    None
}

fn alternate_url<'a>(
    mut links: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> Option<&'a str> {
    links.find_map(|(href, rel)| {
        rel.filter(|relation| relation.is_empty() || is_alternate_relation(relation))
            .or_else(|| rel.is_none().then_some("alternate"))
            .map(|_| href)
    })
}

fn is_alternate_relation(relation: &str) -> bool {
    relation.eq_ignore_ascii_case("alternate")
        || relation.eq_ignore_ascii_case("http://www.iana.org/assignments/relation/alternate")
        || relation.eq_ignore_ascii_case("https://www.iana.org/assignments/relation/alternate")
}

async fn read_limited(mut response: Response) -> Result<Vec<u8>> {
    validate_content_length(response.content_length())?;
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            can_append_chunk(data.len(), chunk.len()),
            "HTTP body exceeds 5 MB limit"
        );
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

/// Per-process request limiter: at most `global_limit` active requests overall and
/// two requests to any one host. Construct once and share across the scheduler.
#[derive(Clone)]
pub struct FetchLimits {
    global: Arc<Semaphore>,
    hosts: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
}

impl FetchLimits {
    pub fn new(global_limit: usize) -> Self {
        Self {
            global: Arc::new(Semaphore::new(global_limit.max(1))),
            hosts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn acquire(&self, url: &str) -> Result<FetchPermit> {
        let host = reqwest::Url::parse(url)?
            .host_str()
            .ok_or_else(|| anyhow!("URL has no host"))?
            .to_ascii_lowercase();
        let host_sem = {
            let mut hosts = self.hosts.lock().await;
            hosts
                .entry(host)
                .or_insert_with(|| Arc::new(Semaphore::new(tuning::FETCH_PER_HOST_CONCURRENCY)))
                .clone()
        };
        // Acquire global first so waiting for a busy host does not consume a host slot.
        let global = self.global.clone().acquire_owned().await?;
        let host = host_sem.acquire_owned().await?;
        Ok(FetchPermit {
            _global: global,
            _host: host,
        })
    }
}

pub struct FetchPermit {
    _global: OwnedSemaphorePermit,
    _host: OwnedSemaphorePermit,
}

/// Shared client and concurrency limits used by the scheduler for both feed
/// requests and article page extraction.
#[derive(Clone)]
pub struct Fetcher {
    pub client: Client,
    pub limits: FetchLimits,
}

impl Fetcher {
    pub fn new(global_limit: usize, user_agent: &str) -> Result<Self> {
        Ok(Self {
            client: http_client(user_agent)?,
            limits: FetchLimits::new(global_limit),
        })
    }

    pub async fn fetch_feed(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<FeedFetch> {
        tokio::time::timeout(
            REQUEST_TIMEOUT,
            fetch_feed_limited(&self.client, &self.limits, url, etag, last_modified),
        )
        .await
        .context("feed request exceeded the 20 second timeout")?
    }

    pub async fn extract_article(
        &self,
        url: &str,
        feed_content: Option<&str>,
    ) -> Result<crate::extract::ExtractedArticle> {
        crate::extract::extract_article(&self.client, &self.limits, url, feed_content).await
    }
}

/// Adaptive polling cadence, in seconds, from the number of items seen in 7 days.
#[cfg(test)]
pub fn adaptive_interval(item_count_last_7_days: u64, ttl_secs: Option<u64>) -> u64 {
    adaptive_interval_with_bounds(
        item_count_last_7_days,
        ttl_secs,
        tuning::FETCH_MIN_INTERVAL_SECS,
        tuning::FETCH_MAX_INTERVAL_SECS,
    )
}

/// Estimate the polling cadence using bounds supplied by runtime configuration.
/// Callers should validate that `min_interval_secs <= max_interval_secs` at startup.
pub fn adaptive_interval_with_bounds(
    item_count_last_7_days: u64,
    ttl_secs: Option<u64>,
    min_interval_secs: u64,
    max_interval_secs: u64,
) -> u64 {
    let base = (tuning::FETCH_RATE_WINDOW_SECS / item_count_last_7_days.max(1))
        .clamp(min_interval_secs, max_interval_secs);
    base.max(ttl_secs.unwrap_or(0))
}

/// Exponential retry delay with caller-supplied jitter in [-0.1, 0.1].
pub fn failure_backoff(interval_secs: u64, error_count: u32, jitter: f64) -> u64 {
    let exponent = error_count.min(63);
    let exponential = interval_secs
        .saturating_mul(1u64 << exponent)
        .min(tuning::FETCH_MAX_BACKOFF_SECS);
    let jitter_bound = tuning::FETCH_BACKOFF_JITTER;
    let factor = (1.0 + jitter.clamp(-jitter_bound, jitter_bound)).max(0.0);
    ((exponential as f64 * factor).round() as u64).min(tuning::FETCH_MAX_BACKOFF_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_response(
        status: StatusCode,
        headers: &[(&str, &str)],
        body: impl Into<reqwest::Body>,
    ) -> Response {
        let mut builder = axum::http::Response::builder().status(status);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        Response::from(builder.body(body).unwrap())
    }

    #[test]
    fn adaptive_interval_clamps_and_respects_ttl() {
        assert_eq!(adaptive_interval(0, None), 6 * 3600);
        assert_eq!(adaptive_interval(1, None), 6 * 3600);
        assert_eq!(adaptive_interval(2, None), 6 * 3600);
        assert_eq!(adaptive_interval(28, None), 6 * 3600);
        assert_eq!(adaptive_interval(29, None), 604_800 / 29);
        assert_eq!(adaptive_interval(336, None), 30 * 60);
        assert_eq!(adaptive_interval(672, None), 15 * 60);
        assert_eq!(adaptive_interval(1000, None), 15 * 60);
        assert_eq!(adaptive_interval(1000, Some(3600)), 3600);
        assert_eq!(adaptive_interval(1, Some(8 * 3600)), 8 * 3600);
        assert_eq!(adaptive_interval_with_bounds(29, None, 60, 3600), 3600);
        assert_eq!(adaptive_interval_with_bounds(1000, None, 1200, 7200), 1200);
        assert_eq!(
            adaptive_interval_with_bounds(1000, Some(2400), 1200, 7200),
            2400
        );
    }

    #[test]
    fn backoff_caps_and_applies_bounded_jitter() {
        assert_eq!(failure_backoff(1800, 0, 0.0), 1800);
        assert_eq!(failure_backoff(1800, 1, 0.0), 3600);
        assert_eq!(failure_backoff(1800, 2, 0.0), 7200);
        assert_eq!(failure_backoff(1800, 3, -0.1), 12960);
        assert_eq!(failure_backoff(1800, 3, 0.1), 15840);
        assert_eq!(failure_backoff(1800, 10, 0.0), 24 * 3600);
        assert_eq!(failure_backoff(1800, 10, 0.1), 24 * 3600);
        assert_eq!(failure_backoff(1800, 1, 2.0), 3960);
    }

    #[test]
    fn reads_rss_ttl_in_minutes_case_insensitively() {
        assert_eq!(
            parse_ttl_secs(
                b"<rss><!-- <ttl>999</ttl> --><channel><description><![CDATA[<ttl>888</ttl>]]></description><item><ttl>7</ttl></item><ttl>45</ttl></channel></rss>"
            ),
            Some(2700)
        );
        assert_eq!(
            parse_ttl_secs(b"<RSS><CHANNEL><TTL>60</TTL></CHANNEL></RSS>"),
            Some(3600)
        );
        assert_eq!(
            parse_ttl_secs(b"<rss><channel><title>120</title></channel></rss>"),
            None
        );
    }

    #[test]
    fn resolves_redirect_locations_and_checks_origin() {
        let source = Url::parse("https://example.com/news/old.xml").unwrap();
        assert_eq!(
            resolve_redirect(&source, "../feed.xml?full=1")
                .unwrap()
                .as_str(),
            "https://example.com/feed.xml?full=1"
        );
        let same = Url::parse("https://example.com/other").unwrap();
        let different_host = Url::parse("https://cdn.example.com/feed").unwrap();
        let different_scheme = Url::parse("http://example.com/feed").unwrap();
        assert!(same_origin(&source, &same));
        assert!(!same_origin(&source, &different_host));
        assert!(!same_origin(&source, &different_scheme));
    }

    #[test]
    fn redirect_loop_stops_after_configured_limit() {
        assert!(!redirect_limit_reached(0));
        assert!(!redirect_limit_reached(tuning::FETCH_MAX_REDIRECTS - 1));
        assert!(redirect_limit_reached(tuning::FETCH_MAX_REDIRECTS));
        assert!(is_followable_redirect(StatusCode::MOVED_PERMANENTLY));
        assert!(is_followable_redirect(StatusCode::FOUND));
        assert!(is_followable_redirect(StatusCode::TEMPORARY_REDIRECT));
        assert!(!is_followable_redirect(StatusCode::NOT_MODIFIED));
        assert!(!is_followable_redirect(StatusCode::MULTIPLE_CHOICES));
    }

    #[test]
    fn picks_an_alternate_link_instead_of_self_or_enclosure() {
        assert_eq!(
            alternate_url(
                [
                    ("https://example.com/item", Some("self")),
                    ("https://example.com/audio.mp3", Some("enclosure")),
                    (
                        "https://example.com/story",
                        Some("https://www.iana.org/assignments/relation/alternate"),
                    ),
                ]
                .into_iter()
            ),
            Some("https://example.com/story")
        );
        assert_eq!(
            alternate_url(
                [
                    ("https://example.com/rss-item", None),
                    ("https://example.com/self", Some("self")),
                ]
                .into_iter()
            ),
            Some("https://example.com/rss-item")
        );
        assert_eq!(
            alternate_url([("https://example.com/self", Some("self"))].into_iter()),
            None
        );
    }

    #[test]
    fn parses_saved_rss_fixture_with_ttl_and_item_fields() {
        let result = parse_feed_body(
            include_bytes!("../tests/fixtures/rss.xml"),
            Some("rss-tag".into()),
            Some("Tue, 20 Oct 2026 10:00:00 GMT".into()),
        )
        .unwrap();
        assert_eq!(result.title.as_deref(), Some("Fixture RSS Desk"));
        assert_eq!(
            result.site_url.as_deref(),
            Some("https://rss.example.test/")
        );
        assert_eq!(result.ttl_secs, Some(45 * 60));
        assert_eq!(result.items.len(), 1);
        let item = &result.items[0];
        assert_eq!(item.guid, "rss-harbour-1");
        assert_eq!(
            item.url.as_deref(),
            Some("https://rss.example.test/harbour-repairs")
        );
        assert_eq!(item.title, "Harbour repairs begin Monday");
        assert!(
            item.content
                .as_deref()
                .unwrap()
                .contains("eastern harbour wall")
        );
        assert_eq!(result.etag.as_deref(), Some("rss-tag"));
    }

    #[test]
    fn parses_saved_atom_fixture_and_selects_alternate_entry_link() {
        let result =
            parse_feed_body(include_bytes!("../tests/fixtures/atom.xml"), None, None).unwrap();
        assert_eq!(result.title.as_deref(), Some("Fixture Atom Desk"));
        assert_eq!(
            result.site_url.as_deref(),
            Some("https://atom.example.test/")
        );
        assert_eq!(result.items.len(), 1);
        let item = &result.items[0];
        assert_eq!(item.guid, "urn:fixture:atom:bridge-1");
        assert_eq!(
            item.url.as_deref(),
            Some("https://atom.example.test/bridge-inspection")
        );
        assert!(
            item.content
                .as_deref()
                .unwrap()
                .contains("Inspectors will examine")
        );
        assert!(result.ttl_secs.is_none());
    }

    #[test]
    fn parses_saved_json_feed_fixture() {
        let result =
            parse_feed_body(include_bytes!("../tests/fixtures/feed.json"), None, None).unwrap();
        assert_eq!(result.title.as_deref(), Some("Fixture JSON Desk"));
        assert_eq!(
            result.site_url.as_deref(),
            Some("https://json.example.test/")
        );
        assert_eq!(result.items.len(), 1);
        let item = &result.items[0];
        assert_eq!(item.guid, "json-transit-1");
        assert_eq!(
            item.url.as_deref(),
            Some("https://json.example.test/transit-update")
        );
        assert_eq!(item.title, "Transit service update");
        assert!(
            item.content
                .as_deref()
                .unwrap()
                .contains("Evening service will resume")
        );
    }

    #[test]
    fn handles_not_modified_validators_without_network() {
        let unchanged = not_modified_result(None, None, Some("stored-etag"), Some("stored-date"));
        assert!(unchanged.not_modified);
        assert!(unchanged.items.is_empty());
        assert_eq!(unchanged.etag.as_deref(), Some("stored-etag"));
        assert_eq!(unchanged.last_modified.as_deref(), Some("stored-date"));
        let updated = not_modified_result(
            Some("response-etag".into()),
            Some("response-date".into()),
            Some("old-etag"),
            Some("old-date"),
        );
        assert_eq!(updated.etag.as_deref(), Some("response-etag"));
        assert_eq!(updated.last_modified.as_deref(), Some("response-date"));
    }

    #[tokio::test]
    async fn parses_real_response_headers_and_rss_body_without_network() {
        let response = build_response(
            StatusCode::OK,
            &[
                ("etag", "\"saved-rss-tag\""),
                ("last-modified", "Tue, 20 Oct 2026 10:00:00 GMT"),
            ],
            reqwest::Body::from(include_bytes!("../tests/fixtures/rss.xml").to_vec()),
        );
        let parsed = parse_feed_response(response, None, None).await.unwrap();
        assert!(!parsed.not_modified);
        assert_eq!(parsed.etag.as_deref(), Some("\"saved-rss-tag\""));
        assert_eq!(
            parsed.last_modified.as_deref(),
            Some("Tue, 20 Oct 2026 10:00:00 GMT")
        );
        assert_eq!(parsed.ttl_secs, Some(45 * 60));
        assert_eq!(parsed.title.as_deref(), Some("Fixture RSS Desk"));
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].guid, "rss-harbour-1");
    }

    #[tokio::test]
    async fn actual_304_response_preserves_missing_validators_and_accepts_new_ones() {
        let response = build_response(StatusCode::NOT_MODIFIED, &[], reqwest::Body::from(""));
        let parsed = parse_feed_response(response, Some("stored-tag"), Some("stored-date"))
            .await
            .unwrap();
        assert!(parsed.not_modified);
        assert!(parsed.items.is_empty());
        assert_eq!(parsed.etag.as_deref(), Some("stored-tag"));
        assert_eq!(parsed.last_modified.as_deref(), Some("stored-date"));

        let response = build_response(
            StatusCode::NOT_MODIFIED,
            &[("etag", "new-tag"), ("last-modified", "new-date")],
            reqwest::Body::from(""),
        );
        let parsed = parse_feed_response(response, Some("old-tag"), Some("old-date"))
            .await
            .unwrap();
        assert_eq!(parsed.etag.as_deref(), Some("new-tag"));
        assert_eq!(parsed.last_modified.as_deref(), Some("new-date"));
    }

    #[tokio::test]
    async fn actual_error_status_and_known_oversize_response_are_rejected() {
        let response = build_response(
            StatusCode::BAD_GATEWAY,
            &[],
            reqwest::Body::from("upstream error"),
        );
        let error = parse_feed_response(response, None, None).await.unwrap_err();
        assert!(error.to_string().contains("502"));

        let oversized = vec![b'x'; MAX_BODY_BYTES + 1];
        let response = build_response(StatusCode::OK, &[], reqwest::Body::from(oversized));
        let error = parse_feed_response(response, None, None).await.unwrap_err();
        assert!(format!("{error:#}").contains("5 MB limit"), "{error:#}");
    }

    #[test]
    fn enforces_body_size_at_known_length_and_chunk_boundaries() {
        assert!(validate_content_length(Some(MAX_BODY_BYTES as u64)).is_ok());
        assert!(validate_content_length(Some(MAX_BODY_BYTES as u64 + 1)).is_err());
        assert!(validate_content_length(None).is_ok());
        assert!(can_append_chunk(MAX_BODY_BYTES - 1, 1));
        assert!(!can_append_chunk(MAX_BODY_BYTES, 1));
        assert!(!can_append_chunk(usize::MAX, 1));
    }
}
