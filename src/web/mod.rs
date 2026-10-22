use crate::{
    App,
    db::{Disagreement, StoryListRow, StorySentence},
};
use askama::Template;
use axum::{
    Form, Router,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};
use time::OffsetDateTime;
use time_tz::{OffsetDateTimeExt, timezones};

#[derive(Template)]
#[template(path = "list.html")]
struct ListTemplate {
    title: String,
    note: String,
    status: String,
    query: String,
    is_for_you: bool,
    is_discover: bool,
    is_saved: bool,
    rows: Vec<StoryListItem>,
    lower_rows: Vec<StoryListItem>,
}

#[derive(Clone)]
struct StoryListItem {
    id: i64,
    title: String,
    score: i64,
    meta: String,
    read: bool,
}

#[derive(Template)]
#[template(path = "story.html")]
struct StoryTemplate {
    id: i64,
    title: String,
    first_seen: String,
    updated: String,
    score_line: String,
    vote: i64,
    saved: bool,
    mode: String,
    sentences: Vec<SentenceItem>,
    sources: Vec<StorySource>,
    outlets: Vec<OutletSource>,
    disagreements: Vec<Disagreement>,
    has_disagreements: bool,
    has_new: bool,
    topic: String,
}

struct SentenceItem {
    text: String,
    href: Option<String>,
    article_url: String,
    outlet: String,
    original: String,
    verified: bool,
    is_new: bool,
}

struct StorySource {
    feed_id: i64,
    outlet: String,
}
struct OutletSource {
    outlet: String,
    url: Option<String>,
}

#[derive(Template)]
#[template(path = "settings.html")]
struct Settings {
    topics: Vec<SettingsTopic>,
    muted_topics: Vec<SettingsTopic>,
    feeds: Vec<SettingsFeed>,
    muted_feeds: Vec<SettingsFeed>,
    has_muted: bool,
    article_count: i64,
    data_size: String,
    last_fetch: String,
    version: &'static str,
    ai_enabled: bool,
    ai_model: String,
}

#[derive(Clone)]
struct SettingsTopic {
    id: i64,
    name: String,
    muted: bool,
}
#[derive(Clone)]
struct SettingsFeed {
    id: i64,
    url: String,
    url_safe: bool,
    title: String,
    error: Option<String>,
    last_ok: bool,
    disabled: bool,
    muted: bool,
    error_count: i64,
}

#[derive(Deserialize, Default)]
struct SearchQuery {
    q: Option<String>,
}
#[derive(Deserialize)]
struct FeedAction {
    url: Option<String>,
    delete_id: Option<i64>,
    retry_id: Option<i64>,
    unmute_id: Option<i64>,
}
#[derive(Deserialize)]
struct TopicAction {
    name: Option<String>,
    delete_id: Option<i64>,
    unmute_id: Option<i64>,
}
#[derive(Deserialize)]
struct VoteAction {
    v: i8,
}
#[derive(Deserialize, Default)]
struct SaveAction {
    saved: Option<bool>,
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to("/for-you") }))
        .route("/healthz", get(health))
        .route("/for-you", get(for_you))
        .route("/discover", get(discover))
        .route("/saved", get(saved))
        .route("/search", get(search))
        .route("/s/{id}", get(story))
        .route("/s/{id}/vote", post(vote))
        .route("/s/{id}/save", post(save))
        .route("/mute/topic", post(mute_topic))
        .route("/mute/feed/{id}", post(mute_feed))
        .route("/settings", get(settings))
        .route("/settings/topics", post(topics))
        .route("/settings/feeds", post(feeds))
        .route("/settings/opml", get(export).post(upload))
        .route("/refresh", post(refresh))
        .route(
            "/static/app.css",
            get(|| async {
                static_asset("text/css; charset=utf-8", include_str!("static/app.css"))
            }),
        )
        .route(
            "/static/app.js",
            get(|| async {
                static_asset(
                    "text/javascript; charset=utf-8",
                    include_str!("static/app.js"),
                )
            }),
        )
        .route(
            "/static/favicon.svg",
            get(|| async { static_asset("image/svg+xml", include_str!("static/favicon.svg")) }),
        )
        .route(
            "/static/icon-192.svg",
            get(|| async { static_asset("image/svg+xml", include_str!("static/icon.svg")) }),
        )
        .route(
            "/static/icon-512.svg",
            get(|| async { static_asset("image/svg+xml", include_str!("static/icon.svg")) }),
        )
        .route(
            "/manifest.webmanifest",
            get(|| async {
                static_asset(
                    "application/manifest+json; charset=utf-8",
                    include_str!("static/manifest.webmanifest"),
                )
            }),
        )
        .layer(DefaultBodyLimit::max(5 * 1024 * 1024))
        .with_state(app)
}

fn static_asset(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000"),
        ],
        body,
    )
}

struct WebError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for WebError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        tracing::error!(error=%self.0, "Request failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not complete this request",
        )
            .into_response()
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn health(State(app): State<App>) -> Response {
    match app.db.health() {
        Ok(()) => (StatusCode::OK, "ok").into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response(),
    }
}

async fn for_you(State(app): State<App>) -> Result<Html<String>, WebError> {
    render_list(&app, "for-you", None)
}
async fn discover(State(app): State<App>) -> Result<Html<String>, WebError> {
    render_list(&app, "discover", None)
}
async fn saved(State(app): State<App>) -> Result<Html<String>, WebError> {
    render_list(&app, "saved", None)
}
async fn search(
    State(app): State<App>,
    Query(params): Query<SearchQuery>,
) -> Result<Html<String>, WebError> {
    render_list(&app, "search", params.q)
}

fn render_list(app: &App, tab: &str, query: Option<String>) -> Result<Html<String>, WebError> {
    let q = query.unwrap_or_default();
    let query_arg = if q.trim().is_empty() {
        None
    } else {
        Some(q.trim())
    };
    let stories = app.db.story_list(tab, query_arg, app.config.min_score)?;
    let mut rows = Vec::new();
    let mut lower_rows = Vec::new();
    for row in stories {
        let item = list_item(&row, tab);
        if tab == "for-you" && row.for_you < i64::from(app.config.min_score) {
            lower_rows.push(item);
        } else {
            rows.push(item);
        }
    }
    let (title, note) = match tab {
        "discover" => (
            "Discover",
            "Not in your topics, but new, unusual or widely covered across your feeds.",
        ),
        "saved" => ("Saved", "Stories you saved for later."),
        "search" => ("Search", "Search story headlines and text."),
        _ => (
            "For you",
            "Ranked by how well each story matches your topics and thumbs.",
        ),
    };
    let last = app.db.last_fetch_at()?;
    let status = match last {
        Some(t) => format!("Updated {}", local_clock(t, &app.config.tz)),
        None => "Waiting for first fetch".into(),
    };
    let page = ListTemplate {
        title: title.into(),
        note: note.into(),
        status,
        query: q,
        is_for_you: tab == "for-you",
        is_discover: tab == "discover",
        is_saved: tab == "saved",
        rows,
        lower_rows,
    };
    Ok(Html(page.render()?))
}

fn list_item(row: &StoryListRow, tab: &str) -> StoryListItem {
    let score = if tab == "discover" {
        row.discover
    } else {
        row.for_you
    };
    let first = if row.updated_at > row.read_at.unwrap_or(0) {
        "Updated · ".to_owned()
    } else {
        String::new()
    };
    let detail = if tab == "discover" {
        match row.discover_reason.as_deref() {
            Some("widely_covered") => format!("Widely covered, {} feeds", row.coverage),
            Some("unusual") => "Unusual".into(),
            _ => "Lots of new detail".into(),
        }
    } else {
        first.trim_end_matches(" · ").to_owned()
    };
    let label = row.label.clone().unwrap_or_default();
    let age_text = age(now().saturating_sub(row.updated_at));
    let meta = match (detail.is_empty(), label.is_empty()) {
        (true, true) => age_text,
        (true, false) => format!("{label} · {age_text}"),
        (false, true) => format!("{detail} · {age_text}"),
        (false, false) => format!("{detail} · {label} · {age_text}"),
    };
    StoryListItem {
        id: row.id,
        title: row.title.clone(),
        score,
        meta,
        read: row.read_at.is_some(),
    }
}

fn age(seconds: i64) -> String {
    let secs = seconds.max(0);
    if secs < 60 {
        return "just now".into();
    }
    if secs < 3600 {
        return format!("{}m ago", secs / 60);
    }
    if secs < 86400 {
        return format!("{}h ago", secs / 3600);
    }
    format!("{}d ago", secs / 86400)
}

fn local_clock(timestamp: i64, zone_name: &str) -> String {
    let Some(zone) = timezones::get_by_name(zone_name) else {
        return "UTC".into();
    };
    let Ok(utc) = OffsetDateTime::from_unix_timestamp(timestamp) else {
        return "UTC".into();
    };
    let local = utc.to_timezone(zone);
    let hour = local.hour();
    let suffix = if hour < 12 { "am" } else { "pm" };
    let twelve_hour = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{twelve_hour}:{:02} {suffix}", local.minute())
}

async fn story(
    State(app): State<App>,
    AxumPath(id): AxumPath<i64>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, WebError> {
    let page = match app
        .db
        .story_page(id, params.get("mode").map(String::as_str), now())?
    {
        Some(page) => page,
        None => return Ok(StatusCode::NOT_FOUND.into_response()),
    };
    let is_discover = page.topic_match.is_none() && page.story.vote != 1;
    let score = if is_discover {
        page.story.discover
    } else {
        page.story.for_you
    };
    let score_line = if is_discover {
        format!("{score} pts · {}", discover_reason(&page.story))
    } else {
        match page.topic_match.as_deref() {
            Some(topic) => format!("{score} pts · Matches {topic}"),
            None => format!("{score} pts"),
        }
    };
    let sentences = page.sentences.iter().map(sentence_item).collect();
    let mut sources = Vec::new();
    let mut seen_feeds = std::collections::HashSet::new();
    let mut outlets: Vec<OutletSource> = Vec::new();
    let mut outlet_positions: HashMap<String, usize> = HashMap::new();
    for source in &page.sources {
        if seen_feeds.insert(source.feed_id) {
            sources.push(StorySource {
                feed_id: source.feed_id,
                outlet: source.outlet.clone(),
            });
        }
        let outlet_key = source.outlet.trim().to_lowercase();
        let safe_url = safe_http_url(&source.url).then(|| source.url.clone());
        if let Some(index) = outlet_positions.get(&outlet_key).copied() {
            if outlets[index].url.is_none() && safe_url.is_some() {
                outlets[index].url = safe_url;
            }
        } else {
            outlet_positions.insert(outlet_key, outlets.len());
            outlets.push(OutletSource {
                outlet: source.outlet.clone(),
                url: safe_url,
            });
        }
    }
    let topic = page.topic_match.clone().unwrap_or_default();
    let output = StoryTemplate {
        id,
        title: page.story.title.clone(),
        first_seen: age(now().saturating_sub(page.story.first_seen_at)),
        updated: age(now().saturating_sub(page.story.updated_at)),
        score_line,
        vote: page.story.vote,
        saved: page.story.saved_at.is_some(),
        mode: page.text_mode.clone(),
        sentences,
        sources,
        outlets,
        disagreements: page.disagreements.clone(),
        has_disagreements: !page.disagreements.is_empty(),
        has_new: page.sentences.iter().any(|sentence| sentence.is_new),
        topic,
    };
    Ok(Html(output.render()?).into_response())
}

fn discover_reason(row: &StoryListRow) -> String {
    match row.discover_reason.as_deref() {
        Some("widely_covered") => format!("Widely covered, {} feeds", row.coverage),
        Some("unusual") => "Unusual".into(),
        _ => "Lots of new detail".into(),
    }
}

fn sentence_item(sentence: &StorySentence) -> SentenceItem {
    let article_url = sentence.url.clone();
    let safe = safe_http_url(&article_url);
    let href = safe.then(|| crate::storytext::fragment_url(&article_url, &sentence.original));
    SentenceItem {
        text: sentence.text.clone(),
        href,
        article_url,
        outlet: sentence.outlet.clone(),
        original: sentence.original.clone(),
        verified: sentence.verified,
        is_new: sentence.is_new,
    }
}

fn safe_http_url(value: &str) -> bool {
    reqwest::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn redirect_back(headers: &axum::http::HeaderMap, fallback: &str) -> Redirect {
    headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.starts_with('/') && !v.starts_with("//"))
        .map(Redirect::to)
        .unwrap_or_else(|| Redirect::to(fallback))
}

async fn vote(
    State(app): State<App>,
    AxumPath(id): AxumPath<i64>,
    headers: axum::http::HeaderMap,
    Form(action): Form<VoteAction>,
) -> Result<Response, WebError> {
    if !(-1..=1).contains(&action.v) {
        return Ok((StatusCode::BAD_REQUEST, "Invalid vote").into_response());
    }
    app.db.set_vote(id, i64::from(action.v))?;
    app.work.notify_one();
    if is_fetch_request(&headers) {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(redirect_back(&headers, &format!("/s/{id}")).into_response())
    }
}

async fn save(
    State(app): State<App>,
    AxumPath(id): AxumPath<i64>,
    headers: axum::http::HeaderMap,
    Form(action): Form<SaveAction>,
) -> Result<Response, WebError> {
    if let Some(saved) = action.saved {
        app.db.set_saved(id, saved, now())?;
    } else {
        app.db.toggle_saved(id, now())?;
    }
    app.work.notify_one();
    if is_fetch_request(&headers) {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(redirect_back(&headers, "/saved").into_response())
    }
}

fn is_fetch_request(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-requested-with")
        .is_some_and(|h| h == "fetch")
        || headers.get(header::ACCEPT).is_some_and(|h| {
            h.to_str()
                .ok()
                .is_some_and(|s| s.contains("application/json"))
        })
}

async fn mute_topic(
    State(app): State<App>,
    Form(action): Form<TopicAction>,
) -> Result<Response, WebError> {
    let name = action.name.unwrap_or_default().trim().to_owned();
    if name.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "Enter a topic name").into_response());
    }
    let topics = app.db.list_topics()?;
    if let Some(topic) = topics
        .into_iter()
        .find(|topic| topic.name.eq_ignore_ascii_case(&name))
    {
        app.db.set_topic_muted(topic.id, true)?;
    } else {
        app.db.add_topic(&name, &[], true, now())?;
    }
    app.work.notify_one();
    Ok(Redirect::to("/settings").into_response())
}

async fn mute_feed(
    State(app): State<App>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Redirect, WebError> {
    app.db.set_feed_muted(id, true)?;
    app.work.notify_one();
    Ok(Redirect::to("/settings"))
}

async fn settings(State(app): State<App>) -> Result<Html<String>, WebError> {
    let ai_enabled = app.config.llm_base_url.is_some();
    let topics = app
        .db
        .list_topics()?
        .into_iter()
        .map(|topic| SettingsTopic {
            id: topic.id,
            name: topic.name,
            muted: topic.muted,
        })
        .collect::<Vec<_>>();
    let muted_topics = topics
        .iter()
        .filter(|topic| topic.muted)
        .cloned()
        .collect::<Vec<_>>();
    let feeds = app
        .db
        .list_feeds()?
        .into_iter()
        .map(|feed| SettingsFeed {
            id: feed.id,
            url: feed.url.clone(),
            url_safe: safe_http_url(&feed.url),
            title: feed.title.unwrap_or(feed.url),
            error: feed.last_error,
            last_ok: feed.last_ok_at.is_some(),
            disabled: feed.disabled,
            muted: feed.muted,
            error_count: feed.error_count,
        })
        .collect::<Vec<_>>();
    let muted_feeds = feeds
        .iter()
        .filter(|feed| feed.muted)
        .cloned()
        .collect::<Vec<_>>();
    let has_muted = !muted_topics.is_empty() || !muted_feeds.is_empty();
    let page = Settings {
        topics,
        muted_topics,
        feeds,
        muted_feeds,
        has_muted,
        article_count: app.db.article_count()?,
        data_size: format_bytes(app.db.data_size()?),
        last_fetch: app
            .db
            .last_fetch_at()?
            .map(|t| {
                format!(
                    "{} ({})",
                    local_clock(t, &app.config.tz),
                    age(now().saturating_sub(t))
                )
            })
            .unwrap_or_else(|| "Never".into()),
        version: env!("CARGO_PKG_VERSION"),
        ai_enabled,
        ai_model: if ai_enabled {
            app.config
                .llm_model
                .clone()
                .unwrap_or_else(|| "Configured".into())
        } else {
            String::new()
        },
    };
    Ok(Html(page.render()?))
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

async fn topics(
    State(app): State<App>,
    Form(action): Form<TopicAction>,
) -> Result<Response, WebError> {
    if let Some(id) = action.delete_id {
        app.db.delete_topic(id)?;
    } else if let Some(id) = action.unmute_id {
        app.db.set_topic_muted(id, false)?;
    } else if let Some(name) = action
        .name
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
    {
        app.db.add_topic(&name, &[], false, now())?;
    } else {
        return Ok((StatusCode::BAD_REQUEST, "Choose a topic action").into_response());
    }
    app.work.notify_one();
    Ok(Redirect::to("/settings").into_response())
}

async fn feeds(
    State(app): State<App>,
    Form(action): Form<FeedAction>,
) -> Result<Response, WebError> {
    if let Some(id) = action.delete_id {
        app.db.delete_feed(id)?;
    } else if let Some(id) = action.retry_id {
        app.db.retry_feed(id, now())?;
    } else if let Some(id) = action.unmute_id {
        app.db.set_feed_muted(id, false)?;
    } else if let Some(url) = action.url {
        if !valid_feed_url(&url) {
            return Ok((StatusCode::BAD_REQUEST, "Enter an http or https feed URL").into_response());
        }
        app.db.add_feed(url.trim())?;
    } else {
        return Ok((StatusCode::BAD_REQUEST, "Choose a feed action").into_response());
    }
    app.refresh.notify_one();
    app.work.notify_one();
    Ok(Redirect::to("/settings").into_response())
}

fn valid_feed_url(value: &str) -> bool {
    reqwest::Url::parse(value.trim())
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

async fn export(State(app): State<App>) -> Result<Response, WebError> {
    let feeds = app
        .db
        .list_feeds()?
        .into_iter()
        .map(|f| crate::opml::OpmlFeed {
            url: f.url,
            title: f.title,
            folder: f.folder,
        })
        .collect::<Vec<_>>();
    Ok((
        [
            (header::CONTENT_TYPE, "application/xml; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=wagwan.opml",
            ),
        ],
        crate::opml::export_opml(&feeds)?,
    )
        .into_response())
}

async fn upload(State(app): State<App>, mut multipart: Multipart) -> Result<Response, WebError> {
    while let Some(field) = multipart.next_field().await? {
        if field.name() == Some("opml") {
            let xml = field.text().await?;
            let parsed = match crate::opml::parse_opml(&xml) {
                Ok(feeds) => feeds,
                Err(_) => {
                    return Ok(
                        (StatusCode::BAD_REQUEST, "Choose a valid OPML file").into_response()
                    );
                }
            };
            if parsed.iter().any(|feed| !valid_feed_url(&feed.url)) {
                return Ok(
                    (StatusCode::BAD_REQUEST, "Feed URLs must use http or https").into_response(),
                );
            }
            app.db.import_feeds(&parsed)?;
            app.refresh.notify_one();
            return Ok(Redirect::to("/settings").into_response());
        }
    }
    Ok((StatusCode::BAD_REQUEST, "Choose an OPML file").into_response())
}

async fn refresh(State(app): State<App>) -> Redirect {
    app.refresh.notify_one();
    Redirect::to("/for-you")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        db::{ArticleInput, Db, SentenceInput, StoredText},
    };
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };
    use tower::ServiceExt;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct Harness {
        app: App,
        _dir: PathBuf,
    }

    impl Harness {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "wagwan-web-test-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let db = Db::open(dir.join("wagwan.sqlite3")).unwrap();
            let config = Config {
                bind: "127.0.0.1:0".parse().unwrap(),
                data_dir: dir.clone(),
                tz: "UTC".into(),
                retention_days: 90,
                min_score: 40,
                fetch_concurrency: 16,
                fetch_min_interval: Duration::from_secs(15 * 60),
                fetch_max_interval: Duration::from_secs(6 * 60 * 60),
                opml_seed: None,
                topics_seed: Vec::new(),
                model_url: "https://example.test/model".into(),
                llm_base_url: None,
                llm_api_key: None,
                llm_model: None,
                llm_concurrency: 2,
                log: "info".into(),
            };
            Self {
                app: App {
                    config: Arc::new(config),
                    db,
                    refresh: Arc::new(tokio::sync::Notify::new()),
                    work: Arc::new(tokio::sync::Notify::new()),
                },
                _dir: dir,
            }
        }

        fn router(&self) -> Router {
            super::router(self.app.clone())
        }

        fn add_story(&self, title: &str, topic: Option<&str>, score: u8) -> (i64, Vec<i64>, i64) {
            let now = crate::now();
            if let Some(topic) = topic {
                self.app.db.add_topic(topic, &[], false, now).unwrap();
            }
            let mut feed_ids = Vec::new();
            let mut article_ids = Vec::new();
            let mut sentence_ids = Vec::new();
            let mut story_id = None;
            for index in 0..2 {
                let feed_id = self
                    .app
                    .db
                    .add_feed(&format!("https://source{index}.example.test/rss"))
                    .unwrap();
                feed_ids.push(feed_id);
                let original = if index == 0 {
                    "The council approved a new coastal flood plan."
                } else {
                    "The plan allocates funding for shoreline protection."
                };
                let url = format!("https://source{index}.example.test/story/{index}");
                let article = ArticleInput {
                    feed_id,
                    guid: format!("story-{title}-{index}"),
                    url,
                    title: title.to_owned(),
                    published_at: Some(now),
                    seen_at: now.saturating_sub(60 - index),
                    text: Some(original.to_owned()),
                    text_source: "page".into(),
                    embedding: Some(vec![1, 2, 3]),
                    minhash: Some(vec![4, 5, 6]),
                    sentences: vec![SentenceInput {
                        position: 0,
                        text: original.to_owned(),
                        embedding: vec![1, 2, 3],
                    }],
                };
                let article_id = self.app.db.insert_article(&article).unwrap().unwrap();
                article_ids.push(article_id);
                let pending = self
                    .app
                    .db
                    .pending_articles_after(10, None, None)
                    .unwrap()
                    .into_iter()
                    .find(|row| row.id == article_id)
                    .unwrap();
                sentence_ids.push(pending.sentences[0].0);
                let assigned = self
                    .app
                    .db
                    .assign_article(
                        &pending,
                        &[1.0, 0.0],
                        &[0; 16],
                        &[(pending.sentences[0].0, vec![1.0, 0.0])],
                        story_id,
                        None,
                        0.8,
                        &[1.0, 0.0],
                    )
                    .unwrap();
                story_id = Some(assigned);
            }
            let story_id = story_id.unwrap();
            self.app
                .db
                .store_text(
                    story_id,
                    "extractive",
                    &sentence_ids
                        .iter()
                        .enumerate()
                        .map(|(i, sentence_id)| StoredText {
                            text: if i == 0 {
                                "The council approved a new coastal flood plan.".into()
                            } else {
                                "The plan allocates funding for shoreline protection.".into()
                            },
                            sentence_id: *sentence_id,
                            extra_sources: Vec::new(),
                            verified: true,
                        })
                        .collect::<Vec<_>>(),
                    Some("coastal-flood-plan-v1"),
                    None,
                    &[],
                )
                .unwrap();
            self.app
                .db
                .store_text(
                    story_id,
                    "ai",
                    &[StoredText {
                        text: "The council approved a plan to protect the coast.".into(),
                        sentence_id: sentence_ids[0],
                        extra_sources: Vec::new(),
                        verified: true,
                    }],
                    Some("coastal-flood-plan-ai-v1"),
                    None,
                    &[crate::db::Disagreement {
                        subject: "Funding".into(),
                        detail: "Sources report different funding amounts.".into(),
                    }],
                )
                .unwrap();
            self.app
                .db
                .apply_score(
                    story_id,
                    score,
                    72,
                    "widely_covered",
                    topic,
                    topic.is_some(),
                    false,
                    2,
                )
                .unwrap();
            (story_id, feed_ids, article_ids[1])
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self._dir);
        }
    }

    async fn request(
        app: Router,
        method: &str,
        uri: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> Response {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(content_type) = content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        app.oneshot(builder.body(body).unwrap()).await.unwrap()
    }

    async fn response_text(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn health_feed_add_failure_retry_and_delete_work_through_router() {
        let harness = Harness::new();
        let health = request(harness.router(), "GET", "/healthz", None, Body::empty()).await;
        assert_eq!(health.status(), StatusCode::OK);
        assert_eq!(response_text(health).await, "ok");

        let added = request(
            harness.router(),
            "POST",
            "/settings/feeds",
            Some("application/x-www-form-urlencoded"),
            Body::from("url=https%3A%2F%2Ffeeds.example.test%2Frss"),
        )
        .await;
        assert_eq!(added.status(), StatusCode::SEE_OTHER);
        assert_eq!(harness.app.db.list_feeds().unwrap().len(), 1);
        let feed_id = harness.app.db.list_feeds().unwrap()[0].id;

        assert_eq!(
            request(
                harness.router(),
                "POST",
                "/settings/feeds",
                Some("application/x-www-form-urlencoded"),
                Body::from("url=javascript%3Aalert(1)")
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST,
        );

        let raw_error = "<connection & timeout>";
        harness
            .app
            .db
            .record_fetch_failure(feed_id, crate::now() + 60, raw_error)
            .unwrap();
        let settings =
            response_text(request(harness.router(), "GET", "/settings", None, Body::empty()).await)
                .await;
        assert!(settings.contains("Failing:"));
        assert!(settings.contains("&#60;connection &#38; timeout&#62;"));
        assert!(!settings.contains(raw_error));
        assert!(settings.contains(&format!("name=\"retry_id\" value=\"{feed_id}\"")));

        let retried = request(
            harness.router(),
            "POST",
            "/settings/feeds",
            Some("application/x-www-form-urlencoded"),
            Body::from(format!("retry_id={feed_id}")),
        )
        .await;
        assert_eq!(retried.status(), StatusCode::SEE_OTHER);
        let feed = harness.app.db.list_feeds().unwrap().remove(0);
        assert_eq!(feed.error_count, 0);
        assert_eq!(feed.last_error, None);
        assert!(!feed.disabled);

        let deleted = request(
            harness.router(),
            "POST",
            "/settings/feeds",
            Some("application/x-www-form-urlencoded"),
            Body::from(format!("delete_id={feed_id}")),
        )
        .await;
        assert_eq!(deleted.status(), StatusCode::SEE_OTHER);
        assert!(harness.app.db.list_feeds().unwrap().is_empty());
    }

    #[tokio::test]
    async fn opml_import_and_export_work_with_plain_html_form_requests() {
        let harness = Harness::new();
        let settings_page =
            response_text(request(harness.router(), "GET", "/settings", None, Body::empty()).await)
                .await;
        assert!(settings_page.contains("enctype=\"multipart/form-data\""));
        assert!(settings_page.contains("href=\"/settings/opml\">Export OPML</a>"));
        let boundary = "wagwan-test-boundary";
        let opml = include_str!("../../tests/fixtures/nested-opml.xml");
        let multipart = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"opml\"; filename=\"feeds.opml\"\r\nContent-Type: application/xml\r\n\r\n{opml}\r\n--{boundary}--\r\n"
        );
        let imported = request(
            harness.router(),
            "POST",
            "/settings/opml",
            Some(&format!("multipart/form-data; boundary={boundary}")),
            Body::from(multipart),
        )
        .await;
        assert_eq!(imported.status(), StatusCode::SEE_OTHER);
        let feeds = harness.app.db.list_feeds().unwrap();
        assert_eq!(feeds.len(), 3);
        assert_eq!(feeds[0].folder.as_deref(), Some("World & Politics"));
        assert_eq!(
            feeds[1].folder.as_deref(),
            Some("World & Politics/Regional")
        );

        let exported = request(
            harness.router(),
            "GET",
            "/settings/opml",
            None,
            Body::empty(),
        )
        .await;
        assert_eq!(exported.status(), StatusCode::OK);
        assert!(
            exported.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("application/xml")
        );
        assert_eq!(
            exported.headers()[header::CONTENT_DISPOSITION],
            "attachment; filename=wagwan.opml"
        );
        let xml = response_text(exported).await;
        let exported_feeds = crate::opml::parse_opml(&xml).unwrap();
        assert_eq!(exported_feeds.len(), 3);
        let mut expected =
            crate::opml::parse_opml(include_str!("../../tests/fixtures/nested-opml.xml")).unwrap();
        let mut actual = exported_feeds.clone();
        expected.sort_by(|a, b| a.url.cmp(&b.url));
        actual.sort_by(|a, b| a.url.cmp(&b.url));
        assert_eq!(actual, expected);
        assert!(
            exported_feeds
                .iter()
                .any(|feed| feed.folder.as_deref() == Some("World & Politics/Regional"))
        );
    }

    #[tokio::test]
    async fn populated_story_routes_and_native_forms_work_without_sockets() {
        let harness = Harness::new();
        let (story_id, feed_ids, _) =
            harness.add_story("Coastal flood plan approved", Some("Coastal planning"), 87);
        let (discover_id, _, _) = harness.add_story("Rail service resumes", None, 74);

        let for_you =
            response_text(request(harness.router(), "GET", "/for-you", None, Body::empty()).await)
                .await;
        assert!(for_you.contains("Coastal flood plan approved"));
        assert!(!for_you.contains("Rail service resumes"));
        let discover =
            response_text(request(harness.router(), "GET", "/discover", None, Body::empty()).await)
                .await;
        assert!(discover.contains("Rail service resumes"));
        let search = response_text(
            request(
                harness.router(),
                "GET",
                "/search?q=coastal",
                None,
                Body::empty(),
            )
            .await,
        )
        .await;
        assert!(search.contains("Coastal flood plan approved"));

        let ai_page = response_text(
            request(
                harness.router(),
                "GET",
                &format!("/s/{story_id}"),
                None,
                Body::empty(),
            )
            .await,
        )
        .await;
        assert!(ai_page.contains("The council approved a plan to protect the coast."));
        assert!(ai_page.contains("Sources differ"));
        assert!(ai_page.contains("https://source0.example.test/story/0"));
        assert!(ai_page.contains("https://source1.example.test/story/1"));
        assert!(ai_page.contains("#:~:text="));
        assert!(ai_page.contains("Show original sentences"));
        let extractive = response_text(
            request(
                harness.router(),
                "GET",
                &format!("/s/{story_id}?mode=extractive"),
                None,
                Body::empty(),
            )
            .await,
        )
        .await;
        assert!(extractive.contains("The council approved a new coastal flood plan."));

        let new_feed = harness
            .app
            .db
            .add_feed("https://newsource.example.test/rss")
            .unwrap();
        let new_article = ArticleInput {
            feed_id: new_feed,
            guid: "coastal-plan-update".into(),
            url: "https://newsource.example.test/updates/coastal-plan".into(),
            title: "Coastal flood plan update".into(),
            published_at: Some(crate::now()),
            seen_at: crate::now().saturating_add(60),
            text: Some("The first seawall project will begin in June.".into()),
            text_source: "page".into(),
            embedding: Some(vec![1, 2, 3]),
            minhash: Some(vec![4, 5, 6]),
            sentences: vec![SentenceInput {
                position: 0,
                text: "The first seawall project will begin in June.".into(),
                embedding: vec![1, 2, 3],
            }],
        };
        let new_article_id = harness
            .app
            .db
            .insert_article(&new_article)
            .unwrap()
            .unwrap();
        let pending = harness
            .app
            .db
            .pending_articles_after(10, None, None)
            .unwrap()
            .into_iter()
            .find(|row| row.id == new_article_id)
            .unwrap();
        let new_sentence_id = pending.sentences[0].0;
        harness
            .app
            .db
            .assign_article(
                &pending,
                &[1.0, 0.0],
                &[0; 16],
                &[(new_sentence_id, vec![1.0, 0.0])],
                Some(story_id),
                None,
                0.8,
                &[1.0, 0.0],
            )
            .unwrap();
        harness
            .app
            .db
            .store_text(
                story_id,
                "extractive",
                &[StoredText {
                    text: "The first seawall project will begin in June.".into(),
                    sentence_id: new_sentence_id,
                    extra_sources: Vec::new(),
                    verified: true,
                }],
                Some("coastal-plan-new-v2"),
                None,
                &[],
            )
            .unwrap();
        let highlighted = response_text(
            request(
                harness.router(),
                "GET",
                &format!("/s/{story_id}?mode=extractive"),
                None,
                Body::empty(),
            )
            .await,
        )
        .await;
        assert!(highlighted.contains("class=\"s new\""));
        assert!(
            highlighted.contains(
                "data-article-url=\"https://newsource.example.test/updates/coastal-plan\""
            )
        );

        let vote = request(
            harness.router(),
            "POST",
            &format!("/s/{story_id}/vote"),
            Some("application/x-www-form-urlencoded"),
            Body::from("v=-1"),
        )
        .await;
        assert_eq!(vote.status(), StatusCode::SEE_OTHER);
        assert_eq!(vote.headers()[header::LOCATION], format!("/s/{story_id}"));
        assert_eq!(
            harness
                .app
                .db
                .story_list("for-you", None, 40)
                .unwrap()
                .iter()
                .find(|row| row.id == story_id)
                .unwrap()
                .vote,
            -1
        );

        let save = request(
            harness.router(),
            "POST",
            &format!("/s/{story_id}/save"),
            Some("application/x-www-form-urlencoded"),
            Body::from("saved=true"),
        )
        .await;
        assert_eq!(save.status(), StatusCode::SEE_OTHER);
        let saved =
            response_text(request(harness.router(), "GET", "/saved", None, Body::empty()).await)
                .await;
        assert!(saved.contains("Coastal flood plan approved"));

        let topic_id = harness
            .app
            .db
            .list_topics()
            .unwrap()
            .into_iter()
            .find(|topic| topic.name == "Coastal planning")
            .unwrap()
            .id;
        let mute_topic = request(
            harness.router(),
            "POST",
            "/mute/topic",
            Some("application/x-www-form-urlencoded"),
            Body::from("name=Coastal+planning"),
        )
        .await;
        assert_eq!(mute_topic.status(), StatusCode::SEE_OTHER);
        assert!(
            harness
                .app
                .db
                .list_topics()
                .unwrap()
                .into_iter()
                .find(|topic| topic.id == topic_id)
                .unwrap()
                .muted
        );
        let unmute_topic = request(
            harness.router(),
            "POST",
            "/settings/topics",
            Some("application/x-www-form-urlencoded"),
            Body::from(format!("unmute_id={topic_id}")),
        )
        .await;
        assert_eq!(unmute_topic.status(), StatusCode::SEE_OTHER);
        assert!(
            !harness
                .app
                .db
                .list_topics()
                .unwrap()
                .into_iter()
                .find(|topic| topic.id == topic_id)
                .unwrap()
                .muted
        );

        let mute_feed = request(
            harness.router(),
            "POST",
            &format!("/mute/feed/{}", feed_ids[0]),
            None,
            Body::empty(),
        )
        .await;
        assert_eq!(mute_feed.status(), StatusCode::SEE_OTHER);
        assert!(
            harness
                .app
                .db
                .list_feeds()
                .unwrap()
                .into_iter()
                .find(|feed| feed.id == feed_ids[0])
                .unwrap()
                .muted
        );
        let unmute_feed = request(
            harness.router(),
            "POST",
            "/settings/feeds",
            Some("application/x-www-form-urlencoded"),
            Body::from(format!("unmute_id={}", feed_ids[0])),
        )
        .await;
        assert_eq!(unmute_feed.status(), StatusCode::SEE_OTHER);
        assert!(
            !harness
                .app
                .db
                .list_feeds()
                .unwrap()
                .into_iter()
                .find(|feed| feed.id == feed_ids[0])
                .unwrap()
                .muted
        );

        let settings =
            response_text(request(harness.router(), "GET", "/settings", None, Body::empty()).await)
                .await;
        assert!(settings.contains("Coastal planning"));
        assert!(settings.contains("source0.example.test"));

        // Test routes and capture fixture-backed output as actual HTML for a local browser preview.
        if let Some(output_dir) = std::env::var_os("WAGWAN_UI_ARTIFACT_DIR") {
            let output_dir = PathBuf::from(output_dir);
            std::fs::create_dir_all(&output_dir).unwrap();
            for (name, uri) in [
                ("for-you", "/for-you".to_owned()),
                ("discover", "/discover".to_owned()),
                ("saved", "/saved".to_owned()),
                ("search", "/search?q=coastal".to_owned()),
                ("story", format!("/s/{story_id}")),
                ("settings", "/settings".to_owned()),
                ("story-discover", format!("/s/{discover_id}")),
            ] {
                let html = response_text(
                    request(harness.router(), "GET", &uri, None, Body::empty()).await,
                )
                .await;
                std::fs::write(
                    output_dir.join(format!("{name}.html")),
                    inline_assets(&html),
                )
                .unwrap();
            }
        }
    }

    fn inline_assets(html: &str) -> String {
        html.replace(
            "<link rel=\"stylesheet\" href=\"/static/app.css\">",
            &format!("<style>{}</style>", include_str!("static/app.css")),
        )
        .replace(
            "<script src=\"/static/app.js\" defer></script>",
            &format!("<script>{}</script>", include_str!("static/app.js")),
        )
    }
}
