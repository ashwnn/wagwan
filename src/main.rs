mod cluster;
mod config;
mod db;
mod dedup;
mod embed;
mod eval;
mod extract;
mod facts;
mod fetch;
mod llm;
mod opml;
mod pipeline;
mod score;
mod storytext;
mod tuning;
mod web;

use anyhow::{Context, Result};
use config::Config;
use db::{ArticleInput, Db, Feed, SentenceInput};
use fetch::Fetcher;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Notify;

#[derive(Clone)]
pub struct App {
    config: Arc<Config>,
    db: Db,
    refresh: Arc<Notify>,
    work: Arc<Notify>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env().map_err(anyhow::Error::msg)?;
    tracing_subscriber::fmt()
        .with_env_filter(&config.log)
        .init();
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("healthcheck") {
        let ip = match config.bind.ip() {
            std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
            }
            std::net::IpAddr::V6(ip) if ip.is_unspecified() => {
                std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
            }
            ip => ip,
        };
        let address = std::net::SocketAddr::new(ip, config.bind.port());
        reqwest::Client::builder()
            .no_proxy()
            .build()?
            .get(format!("http://{address}/healthz"))
            .timeout(Duration::from_secs(4))
            .send()
            .await?
            .error_for_status()?;
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("eval") {
        let path = args
            .get(1)
            .context("usage: wagwan eval PATH_TO_SEMEVAL_DATA")?;
        let pairs = eval::load_pairs(std::path::Path::new(path))?;
        let model =
            embed::Embedder::load(&config.data_dir.join("models"), &config.model_url).await?;
        let report = tokio::task::spawn_blocking(move || eval::evaluate(&pairs, &model)).await??;
        eval::print_report(&report);
        return Ok(());
    }
    std::fs::create_dir_all(&config.data_dir)?;
    let db = Db::open(config.data_dir.join("wagwan.sqlite3"))?;
    match args.first().map(String::as_str) {
        Some("import-opml") => {
            let path = args.get(1).context("usage: wagwan import-opml FILE")?;
            let count = import(&db, &std::fs::read_to_string(path)?)?;
            println!("Imported {count} feeds");
            return Ok(());
        }
        None | Some("serve") => (),
        Some(command) => {
            anyhow::bail!(
                "unknown command: {command}; use serve, healthcheck, import-opml, or eval"
            )
        }
    }
    if db.is_empty()? {
        if let Some(path) = &config.opml_seed {
            import(&db, &std::fs::read_to_string(path)?)?;
        }
        for topic in &config.topics_seed {
            db.add_topic(topic, &[], false, now())?;
        }
    }
    let bind = config.bind;
    let app = App {
        config: Arc::new(config),
        db,
        refresh: Arc::new(Notify::new()),
        work: Arc::new(Notify::new()),
    };
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let fetch_task = tokio::spawn(scheduler(app.clone()));
    let processing_task = tokio::spawn(processing_worker(app.clone()));
    let maintenance_task = tokio::spawn(maintenance_worker(app.clone()));
    tracing::info!(%bind, "Wagwan listening");
    axum::serve(listener, web::router(app))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    fetch_task.abort();
    processing_task.abort();
    maintenance_task.abort();
    let _ = fetch_task.await;
    let _ = processing_task.await;
    let _ = maintenance_task.await;
    Ok(())
}

fn import(db: &Db, xml: &str) -> Result<usize> {
    let feeds = opml::parse_opml(xml)?;
    db.import_feeds(&feeds)
}

async fn scheduler(app: App) {
    let fetcher = match Fetcher::new(
        app.config.fetch_concurrency,
        concat!(
            "Wagwan/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/ashwnn/wagwan)"
        ),
    ) {
        Ok(fetcher) => Arc::new(fetcher),
        Err(error) => {
            tracing::error!(%error, "Could not initialize fetcher");
            return;
        }
    };
    loop {
        match app.db.due_feeds(now(), usize::MAX) {
            Ok(feeds) => {
                let had_due_feeds = !feeds.is_empty();
                let mut tasks = tokio::task::JoinSet::new();
                let mut pending = feeds.into_iter();
                loop {
                    while tasks.len() < app.config.fetch_concurrency {
                        let Some(feed) = pending.next() else { break };
                        let app = app.clone();
                        let fetcher = fetcher.clone();
                        tasks.spawn(async move { process_feed(&app, &fetcher, feed).await });
                    }
                    match tasks.join_next().await {
                        Some(Ok(Err(error))) => tracing::warn!(%error, "Feed processing failed"),
                        Some(Err(error)) => tracing::error!(%error, "Feed task failed"),
                        Some(Ok(Ok(()))) => (),
                        None => break,
                    }
                }
                if had_due_feeds {
                    if let Err(error) = app.db.record_cycle(now()) {
                        tracing::warn!(%error, "Could not record fetch cycle");
                    }
                    app.work.notify_one();
                    tracing::info!("Fetch cycle completed");
                }
            }
            Err(error) => tracing::error!(%error, "Could not load due feeds"),
        }
        tokio::select! { _ = tokio::time::sleep(Duration::from_secs(tuning::SCHEDULER_TICK_SECS)) => (), _ = app.refresh.notified() => () }
    }
}

async fn process_feed(app: &App, fetcher: &Fetcher, feed: Feed) -> Result<()> {
    tracing::debug!(feed_id = feed.id, site_url = ?feed.site_url, muted = feed.muted,
        next_fetch_at = feed.next_fetch_at, "Processing due feed");
    let result = ingest(app, fetcher, &feed).await;
    if let Err(error) = &result {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let jitter = (f64::from(nanos % 2001) / 10000.0) - tuning::FETCH_BACKOFF_JITTER;
        let delay = fetch::failure_backoff(
            feed.interval_secs.max(1) as u64,
            (feed.error_count + 1) as u32,
            jitter,
        );
        app.db
            .record_fetch_failure(feed.id, now() + delay as i64, &error.to_string())?;
    }
    result
}

async fn ingest(app: &App, fetcher: &Fetcher, feed: &Feed) -> Result<()> {
    let fetched = fetcher
        .fetch_feed(
            &feed.url,
            feed.etag.as_deref(),
            feed.last_modified.as_deref(),
        )
        .await?;
    if fetched.not_modified {
        tracing::info!(feed_id = feed.id, "Feed not modified (304)");
    }
    for item in fetched.items {
        if feed.last_ok_at.is_none()
            && item
                .published_at
                .is_some_and(|date| date < now() - tuning::FEED_INITIAL_IMPORT_AGE_SECS as i64)
        {
            continue;
        }
        if app.db.article_exists(feed.id, &item.guid)? {
            continue;
        }
        let Some(url) = item.url else { continue };
        let extracted = fetcher
            .extract_article(&url, item.content.as_deref())
            .await?;
        let text_source = if extracted.text_source == extract::TextSource::Page {
            "page"
        } else {
            "feed"
        };
        let sentences = extracted
            .sentences
            .into_iter()
            .enumerate()
            .map(|(position, text)| SentenceInput {
                position: position as i64,
                text,
                embedding: Vec::new(),
            })
            .collect();
        app.db.insert_article(&ArticleInput {
            feed_id: feed.id,
            guid: item.guid,
            url,
            title: item.title,
            published_at: item.published_at,
            seen_at: now(),
            text: Some(extracted.text),
            text_source: text_source.into(),
            embedding: None,
            minhash: None,
            sentences,
        })?;
    }
    let count = app
        .db
        .recent_article_count(feed.id, now() - tuning::FETCH_RATE_WINDOW_SECS as i64)?;
    let mut interval = fetch::adaptive_interval_with_bounds(
        count as u64,
        fetched.ttl_secs,
        app.config.fetch_min_interval.as_secs(),
        app.config.fetch_max_interval.as_secs(),
    );
    // A 304 has no feed body and therefore no TTL. Keep the last effective
    // cadence so a declared TTL remains respected until the next 200 response.
    if fetched.not_modified {
        interval = interval.max(feed.interval_secs.max(1) as u64);
    }
    app.db.record_fetch_success(
        feed.id,
        now(),
        fetched.etag.as_deref(),
        fetched.last_modified.as_deref(),
        interval as i64,
        fetched.title.as_deref(),
        fetched.site_url.as_deref(),
    )?;
    Ok(())
}

/// Handle the signal used by Docker as well as an interactive Ctrl-C.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => (),
                    _ = terminate.recv() => (),
                }
            }
            Err(error) => {
                tracing::warn!(%error, "Could not register termination signal");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Keep the HTTP server available while downloading/loading the optional local
/// assets. All article embedding and clustering CPU work stays off Tokio threads.
async fn processing_worker(app: App) {
    let model = loop {
        match embed::Embedder::load(&app.config.data_dir.join("models"), &app.config.model_url)
            .await
        {
            Ok(model) => break Arc::new(model),
            Err(error) => {
                tracing::error!(%error, "Could not load embedding model; processing will retry");
                tokio::time::sleep(Duration::from_secs(tuning::MODEL_RETRY_INTERVAL_SECS)).await;
            }
        }
    };
    let pipeline = match pipeline::Pipeline::new(app.db.clone(), app.config.clone(), model) {
        Ok(pipeline) => Arc::new(pipeline),
        Err(error) => {
            tracing::error!(%error, "Could not initialize story processing");
            return;
        }
    };
    let mut ai_task: Option<tokio::task::JoinHandle<()>> = None;
    loop {
        let run = pipeline.clone();
        match tokio::task::spawn_blocking(move || run.process()).await {
            Ok(Ok(())) => (),
            Ok(Err(error)) => tracing::warn!(%error, "Story processing failed"),
            Err(error) => tracing::error!(%error, "Story processing worker failed"),
        }
        if ai_task.as_ref().is_none_or(|task| task.is_finished()) {
            let run = pipeline.clone();
            ai_task = Some(tokio::spawn(async move {
                if let Err(error) = run.generate_ai().await {
                    tracing::warn!(%error, "Story rewriting failed; using original sentences");
                }
            }));
        }
        tokio::select! {
            _ = app.work.notified() => (),
            _ = tokio::time::sleep(Duration::from_secs(tuning::SCORE_REFRESH_INTERVAL_SECS)) => (),
        }
    }
}

/// Retention remains independent of model download or provider availability.
async fn maintenance_worker(app: App) {
    let mut last_retention: Option<i64> = None;
    loop {
        let current = now();
        let run_retention =
            last_retention.is_none_or(|last| current - last >= tuning::RETENTION_INTERVAL_SECS);
        let db = app.db.clone();
        let days = app.config.retention_days;
        match tokio::task::spawn_blocking(move || {
            if run_retention {
                db.close_and_retain(current, days)
            } else {
                db.close_stories(current)
            }
        })
        .await
        {
            Ok(Ok(())) => {
                if run_retention {
                    last_retention = Some(current);
                    app.work.notify_one();
                }
            }
            Ok(Err(error)) => tracing::warn!(%error, "Maintenance failed"),
            Err(error) => tracing::error!(%error, "Maintenance worker failed"),
        }
        tokio::time::sleep(Duration::from_secs(tuning::SCORE_REFRESH_INTERVAL_SECS)).await;
    }
}
