use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

const READ_CONNECTIONS: usize = 3;
const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_init.sql");

#[derive(Clone)]
pub struct Db(Arc<DbInner>);

struct DbInner {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
}

#[derive(Clone, Debug)]
pub struct Feed {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub folder: Option<String>,
    pub muted: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub interval_secs: i64,
    pub next_fetch_at: i64,
    pub last_ok_at: Option<i64>,
    pub error_count: i64,
    pub last_error: Option<String>,
    pub disabled: bool,
}

#[derive(Clone, Debug)]
pub struct SentenceInput {
    pub position: i64,
    pub text: String,
    pub embedding: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ArticleInput {
    pub feed_id: i64,
    pub guid: String,
    pub url: String,
    pub title: String,
    pub published_at: Option<i64>,
    pub seen_at: i64,
    pub text: Option<String>,
    pub text_source: String,
    pub embedding: Option<Vec<u8>>,
    pub minhash: Option<Vec<u8>>,
    pub sentences: Vec<SentenceInput>,
}

impl Db {
    /// Open the database, apply migrations, and create a small pool of read-only
    /// connections. The write connection is serialized behind a mutex.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating database directory {}", parent.display()))?;
        }
        let writer = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        writer.busy_timeout(Duration::from_secs(5))?;
        let auto_vacuum: i64 = writer.query_row("PRAGMA auto_vacuum", [], |row| row.get(0))?;
        if auto_vacuum != 2 {
            // Switching an existing database to incremental vacuum requires a
            // one-time VACUUM. Run this before migrations create tables.
            writer.execute_batch("PRAGMA auto_vacuum=INCREMENTAL; VACUUM;")?;
        }
        writer.pragma_update(None, "foreign_keys", "ON")?;
        writer.pragma_update(None, "journal_mode", "WAL")?;
        writer.pragma_update(None, "synchronous", "NORMAL")?;
        Self::migrate(&writer)?;

        let mut readers = Vec::with_capacity(READ_CONNECTIONS);
        for _ in 0..READ_CONNECTIONS {
            let reader = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            reader.busy_timeout(Duration::from_secs(5))?;
            reader.pragma_update(None, "foreign_keys", "ON")?;
            readers.push(Mutex::new(reader));
        }
        Ok(Self(Arc::new(DbInner {
            writer: Mutex::new(writer),
            readers,
            next_reader: AtomicUsize::new(0),
        })))
    }

    fn migrate(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);\n\
             INSERT INTO schema_version(version) SELECT 0 WHERE NOT EXISTS (SELECT 1 FROM schema_version);",
        )?;
        let version: i64 =
            conn.query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))?;
        if version < 1 {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(INITIAL_MIGRATION)?;
            tx.execute("INSERT INTO schema_version(version) VALUES (1)", [])?;
            tx.commit()?;
        }
        if version < 2 {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(include_str!("../migrations/0002_reader.sql"))?;
            tx.execute("INSERT INTO schema_version(version) VALUES(2)", [])?;
            tx.commit()?;
        }
        if version < 3 {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(include_str!("../migrations/0003_votes.sql"))?;
            tx.execute("INSERT INTO schema_version(version) VALUES(3)", [])?;
            tx.commit()?;
        }
        Ok(())
    }

    fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let index = self.0.next_reader.fetch_add(1, Ordering::Relaxed) % self.0.readers.len();
        let conn = self.0.readers[index]
            .lock()
            .map_err(|_| anyhow::anyhow!("database read connection mutex poisoned"))?;
        f(&conn)
    }

    fn write<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self
            .0
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("database write connection mutex poisoned"))?;
        f(&mut conn)
    }

    pub fn health(&self) -> Result<()> {
        self.read(|conn| {
            conn.query_row("SELECT 1", [], |_| Ok(()))?;
            Ok(())
        })
    }

    pub fn is_empty(&self) -> Result<bool> {
        self.read(|conn| {
            let feeds: i64 = conn.query_row("SELECT COUNT(*) FROM feeds", [], |r| r.get(0))?;
            let topics: i64 = conn.query_row("SELECT COUNT(*) FROM topics", [], |r| r.get(0))?;
            Ok(feeds == 0 && topics == 0)
        })
    }

    pub fn list_feeds(&self) -> Result<Vec<Feed>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id,url,title,site_url,folder,muted,etag,last_modified,interval_secs,\
                 next_fetch_at,last_ok_at,error_count,last_error,disabled FROM feeds ORDER BY id",
            )?;
            let rows = stmt.query_map([], map_feed)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn add_feed(&self, url: &str) -> Result<i64> {
        self.add_feed_with_folder(url, None)
    }

    pub fn add_feed_with_folder(&self, url: &str, folder: Option<&str>) -> Result<i64> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO feeds(url,folder,created_at) VALUES (?1,?2,?3) \
                 ON CONFLICT(url) DO NOTHING",
                params![url, folder, unix_now()],
            )?;
            let id: i64 =
                conn.query_row("SELECT id FROM feeds WHERE url=?1", [url], |r| r.get(0))?;
            Ok(id)
        })
    }

    /// Import OPML feeds atomically. All URLs are validated before opening the
    /// write transaction, and duplicate URLs leave their existing metadata
    /// untouched. Returns the number of feeds newly inserted.
    pub fn import_feeds(&self, feeds: &[crate::opml::OpmlFeed]) -> Result<usize> {
        let urls = feeds
            .iter()
            .map(|feed| {
                let url = feed.url.trim();
                let parsed = reqwest::Url::parse(url)
                    .with_context(|| format!("invalid feed URL in OPML: {url}"))?;
                if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                    anyhow::bail!("feed URL in OPML must use http or https: {url}");
                }
                Ok(url.to_owned())
            })
            .collect::<Result<Vec<_>>>()?;

        self.write(|conn| {
            let tx = conn.transaction()?;
            let mut inserted = 0;
            for (feed, url) in feeds.iter().zip(urls.iter()) {
                inserted += tx.execute(
                    "INSERT INTO feeds(url,title,folder,created_at) VALUES (?1,?2,?3,?4) \
                     ON CONFLICT(url) DO NOTHING",
                    params![url, feed.title, feed.folder, unix_now()],
                )?;
            }
            tx.commit()?;
            Ok(inserted)
        })
    }

    /// Return true only while the story still points at this input hash and an
    /// AI text row exists for it.
    pub fn has_ai_for(&self, story_id: i64, hash: &str) -> Result<bool> {
        self.read(|conn| {
            Ok(conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM stories s WHERE s.id=?1 AND s.text_built_for=?2 \
                 AND EXISTS(SELECT 1 FROM story_text t WHERE t.story_id=s.id AND t.mode='ai'))",
                params![story_id, hash],
                |row| row.get(0),
            )?)
        })
    }

    pub fn delete_feed(&self, id: i64) -> Result<bool> {
        self.write(|conn| {
            let tx = conn.transaction()?;
            let affected_stories = {
                let mut statement = tx.prepare(
                    "SELECT DISTINCT story_id FROM articles WHERE story_id IS NOT NULL AND \
                     (feed_id=?1 OR copy_of IN (SELECT id FROM articles WHERE feed_id=?1))",
                )?;
                statement
                    .query_map([id], |row| row.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            // Story text can point at sentences from an article being removed.
            // Clear those references before the feed/article/sentence cascades.
            tx.execute(
                "DELETE FROM story_text WHERE sentence_id IN (\
                   SELECT s.id FROM sentences s JOIN articles a ON a.id=s.article_id WHERE a.feed_id=?1\
                 )",
                [id],
            )?;
            // If an original disappears because its feed is removed, its direct
            // copies become independent surviving sources. This also prevents
            // the self-referential article FK from blocking the feed cascade.
            tx.execute(
                "UPDATE articles SET copy_of=NULL WHERE copy_of IN \
                 (SELECT id FROM articles WHERE feed_id=?1)",
                [id],
            )?;
            let deleted = tx.execute("DELETE FROM feeds WHERE id=?1", [id])? != 0;
            if deleted {
                for story_id in affected_stories {
                    repair_story_after_source_change(&tx, story_id)?;
                }
            }
            tx.commit()?;
            Ok(deleted)
        })
    }

    pub fn retry_feed(&self, id: i64, now: i64) -> Result<bool> {
        self.write(|conn| {
            Ok(conn.execute(
                "UPDATE feeds SET disabled=0,error_count=0,last_error=NULL,next_fetch_at=?2 WHERE id=?1",
                params![id, now],
            )? != 0)
        })
    }

    pub fn due_feeds(&self, now: i64, limit: usize) -> Result<Vec<Feed>> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id,url,title,site_url,folder,muted,etag,last_modified,interval_secs,\
                 next_fetch_at,last_ok_at,error_count,last_error,disabled FROM feeds \
                 WHERE next_fetch_at<=?1 AND disabled=0 ORDER BY next_fetch_at LIMIT ?2",
            )?;
            let rows =
                stmt.query_map(params![now, limit.min(i64::MAX as usize) as i64], map_feed)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    // Clippy allow: these are independent response metadata and scheduler inputs
    // that are persisted together after a successful conditional fetch.
    #[allow(clippy::too_many_arguments)]
    pub fn record_fetch_success(
        &self,
        feed_id: i64,
        now: i64,
        etag: Option<&str>,
        last_modified: Option<&str>,
        interval_secs: i64,
        title: Option<&str>,
        site_url: Option<&str>,
    ) -> Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE feeds SET etag=?2,last_modified=?3,interval_secs=?4,next_fetch_at=?5,\
                 last_ok_at=?6,error_count=0,last_error=NULL, \
                 title=COALESCE(?7,title),site_url=COALESCE(?8,site_url) WHERE id=?1",
                params![
                    feed_id,
                    etag,
                    last_modified,
                    interval_secs,
                    now + interval_secs,
                    now,
                    title,
                    site_url
                ],
            )?;
            Ok(())
        })
    }

    /// Persist a failure and its already-calculated next fetch time. Ten
    /// consecutive failures disable a feed until the user retries it.
    pub fn record_fetch_failure(
        &self,
        feed_id: i64,
        next_fetch_at: i64,
        error: &str,
    ) -> Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE feeds SET error_count=error_count+1,last_error=?2,next_fetch_at=?3,\
                 disabled=CASE WHEN error_count+1>=10 THEN 1 ELSE disabled END WHERE id=?1",
                params![feed_id, error, next_fetch_at],
            )?;
            Ok(())
        })
    }

    pub fn article_exists(&self, feed_id: i64, guid: &str) -> Result<bool> {
        self.read(|conn| {
            Ok(conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM articles WHERE feed_id=?1 AND guid=?2)",
                params![feed_id, guid],
                |r| r.get(0),
            )?)
        })
    }

    pub fn recent_article_count(&self, feed_id: i64, since: i64) -> Result<i64> {
        self.read(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM articles WHERE feed_id=?1 AND seen_at>=?2",
                params![feed_id, since],
                |r| r.get(0),
            )?)
        })
    }

    pub fn article_count(&self) -> Result<i64> {
        self.read(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM articles", [], |r| r.get(0))?))
    }

    /// Insert a newly fetched article and all its sentences atomically. Returns
    /// `None` when `(feed_id, guid)` already exists.
    pub fn insert_article(&self, article: &ArticleInput) -> Result<Option<i64>> {
        self.write(|conn| {
            let tx = conn.transaction()?;
            let inserted = tx.execute(
                "INSERT INTO articles(feed_id,guid,url,title,published_at,seen_at,text,text_source,embedding,minhash) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(feed_id,guid) DO NOTHING",
                params![article.feed_id, article.guid, article.url, article.title, article.published_at,
                    article.seen_at, article.text, article.text_source, article.embedding, article.minhash],
            )?;
            if inserted == 0 {
                tx.commit()?;
                return Ok(None);
            }
            let id = tx.last_insert_rowid();
            for sentence in &article.sentences {
                tx.execute(
                    "INSERT INTO sentences(article_id,position,text,embedding) VALUES (?1,?2,?3,?4)",
                    params![id, sentence.position, sentence.text, sentence.embedding],
                )?;
            }
            tx.commit()?;
            Ok(Some(id))
        })
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn map_feed(row: &rusqlite::Row<'_>) -> rusqlite::Result<Feed> {
    Ok(Feed {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        site_url: row.get(3)?,
        folder: row.get(4)?,
        muted: row.get::<_, i64>(5)? != 0,
        etag: row.get(6)?,
        last_modified: row.get(7)?,
        interval_secs: row.get(8)?,
        next_fetch_at: row.get(9)?,
        last_ok_at: row.get(10)?,
        error_count: row.get(11)?,
        last_error: row.get(12)?,
        disabled: row.get::<_, i64>(13)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_DB: AtomicU64 = AtomicU64::new(0);

    struct TestDb {
        path: std::path::PathBuf,
        db: Db,
    }

    impl TestDb {
        fn new() -> Self {
            let id = NEXT_DB.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("wagwan-db-test-{}-{id}.sqlite", std::process::id()));
            let _ = fs::remove_file(&path);
            let db = Db::open(&path).expect("open test database");
            Self { path, db }
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_file(self.path.with_extension("sqlite-wal"));
            let _ = fs::remove_file(self.path.with_extension("sqlite-shm"));
        }
    }

    fn article(feed_id: i64) -> ArticleInput {
        ArticleInput {
            feed_id,
            guid: "entry-1".into(),
            url: "https://example.test/story".into(),
            title: "Example story".into(),
            published_at: Some(100),
            seen_at: 200,
            text: Some("One sentence.".into()),
            text_source: "feed".into(),
            embedding: None,
            minhash: None,
            sentences: vec![SentenceInput {
                position: 0,
                text: "One sentence.".into(),
                embedding: Vec::new(),
            }],
        }
    }

    #[test]
    fn migration_persists_and_is_safe_to_apply_again() {
        let test_db = TestDb::new();
        let id = test_db.db.add_feed("https://example.test/feed").unwrap();
        let reopened = Db::open(&test_db.path).unwrap();
        let feed = reopened
            .list_feeds()
            .unwrap()
            .into_iter()
            .find(|feed| feed.id == id)
            .unwrap();
        assert_eq!(feed.url, "https://example.test/feed");
        assert!(reopened.health().is_ok());
        assert_eq!(
            reopened.0.readers[0]
                .lock()
                .unwrap()
                .query_row("SELECT MAX(version) FROM schema_version", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            3
        );
    }

    #[test]
    fn upgrading_v1_rebuilds_contentful_story_search_index() {
        let id = NEXT_DB.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("wagwan-db-test-{}-{id}.sqlite", std::process::id()));
        let _ = fs::remove_file(&path);
        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(INITIAL_MIGRATION).unwrap();
        legacy.execute_batch(
            "CREATE TABLE schema_version(version INTEGER NOT NULL);\
             INSERT INTO schema_version VALUES(1);\
             INSERT INTO feeds(id,url,created_at) VALUES(1,'https://example.test/feed',1);\
             INSERT INTO stories(id,title,centroid,first_seen_at,updated_at) VALUES(1,'Migration title',X'',1,1);\
             INSERT INTO articles(id,feed_id,guid,url,title,seen_at,text_source,story_id)\
               VALUES(1,1,'g1','https://example.test/a','Article',1,'feed',1);\
             INSERT INTO sentences(id,article_id,position,text,embedding)\
               VALUES(1,1,0,'Migration searchable body',X'');\
             INSERT INTO story_text(story_id,mode,position,text,sentence_id)\
               VALUES(1,'extractive',0,'Migration searchable body',1);",
        ).unwrap();
        drop(legacy);

        let db = Db::open(&path).unwrap();
        let found: i64 = db.0.readers[0]
            .lock()
            .unwrap()
            .query_row(
                "SELECT rowid FROM stories_fts WHERE stories_fts MATCH 'searchable'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1);
        drop(db);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn duplicate_article_does_not_add_duplicate_sentences_and_insert_is_atomic() {
        let test_db = TestDb::new();
        let feed_id = test_db.db.add_feed("https://example.test/feed").unwrap();
        let first_id = test_db
            .db
            .insert_article(&article(feed_id))
            .unwrap()
            .unwrap();

        let mut duplicate = article(feed_id);
        duplicate.title = "Changed title on duplicate".into();
        duplicate.sentences.push(SentenceInput {
            position: 1,
            text: "Must not be inserted.".into(),
            embedding: Vec::new(),
        });
        assert_eq!(test_db.db.insert_article(&duplicate).unwrap(), None);

        {
            let conn = test_db.db.0.readers[0].lock().unwrap();
            let sentence_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sentences WHERE article_id=?1",
                    [first_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sentence_count, 1);
        }
        {
            let conn = test_db.db.0.writer.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER reject_test_sentence BEFORE INSERT ON sentences \
                 BEGIN SELECT RAISE(ABORT, 'intentional test failure'); END;",
            )
            .unwrap();
        }

        let mut rejected = article(feed_id);
        rejected.guid = "entry-2".into();
        assert!(test_db.db.insert_article(&rejected).is_err());
        assert_eq!(test_db.db.article_count().unwrap(), 1);
        let remaining: i64 = test_db.db.0.readers[1]
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM sentences", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn repeated_fetch_failures_disable_feed_until_retry() {
        let test_db = TestDb::new();
        let feed_id = test_db.db.add_feed("https://example.test/feed").unwrap();
        for failure in 1..=10 {
            test_db
                .db
                .record_fetch_failure(feed_id, 100 + failure, "network error")
                .unwrap();
        }
        let feed = test_db.db.list_feeds().unwrap().remove(0);
        assert_eq!(feed.error_count, 10);
        assert!(feed.disabled);
        assert_eq!(feed.last_error.as_deref(), Some("network error"));
        assert!(test_db.db.due_feeds(i64::MAX, 10).unwrap().is_empty());

        assert!(test_db.db.retry_feed(feed_id, 500).unwrap());
        let feed = test_db.db.list_feeds().unwrap().remove(0);
        assert_eq!(feed.error_count, 0);
        assert!(!feed.disabled);
        assert_eq!(feed.last_error, None);
        assert_eq!(test_db.db.due_feeds(500, 10).unwrap().len(), 1);
    }

    #[test]
    fn successful_fetch_stores_success_time_separately_from_next_due_time() {
        let test_db = TestDb::new();
        let feed_id = test_db.db.add_feed("https://example.test/feed").unwrap();
        test_db
            .db
            .record_fetch_failure(feed_id, 999, "temporary error")
            .unwrap();

        test_db
            .db
            .record_fetch_success(
                feed_id,
                1_000,
                Some("etag-1"),
                Some("Wed, 21 Oct 2026 07:28:00 GMT"),
                1_800,
                Some("Example feed"),
                Some("https://example.test"),
            )
            .unwrap();

        let feed = test_db.db.list_feeds().unwrap().remove(0);
        assert_eq!(feed.etag.as_deref(), Some("etag-1"));
        assert_eq!(
            feed.last_modified.as_deref(),
            Some("Wed, 21 Oct 2026 07:28:00 GMT")
        );
        assert_eq!(feed.interval_secs, 1_800);
        assert_eq!(feed.next_fetch_at, 2_800);
        assert_eq!(feed.last_ok_at, Some(1_000));
        assert_eq!(feed.error_count, 0);
        assert_eq!(feed.last_error, None);
        assert_eq!(feed.title.as_deref(), Some("Example feed"));
        assert_eq!(feed.site_url.as_deref(), Some("https://example.test"));
    }

    #[test]
    fn opml_import_validates_all_urls_before_writing_any_feeds() {
        let test_db = TestDb::new();
        let feeds = vec![
            crate::opml::OpmlFeed {
                url: "https://example.test/feed".into(),
                title: Some("Example".into()),
                folder: Some("News".into()),
            },
            crate::opml::OpmlFeed {
                url: "file:///etc/passwd".into(),
                title: Some("Invalid scheme".into()),
                folder: None,
            },
        ];

        assert!(test_db.db.import_feeds(&feeds).is_err());
        assert!(test_db.db.list_feeds().unwrap().is_empty());
    }

    #[test]
    fn opml_import_persists_title_folder_and_preserves_existing_duplicate_metadata() {
        let test_db = TestDb::new();
        let feeds = vec![
            crate::opml::OpmlFeed {
                url: "https://example.test/feed".into(),
                title: Some("Imported title".into()),
                folder: Some("Imported folder".into()),
            },
            crate::opml::OpmlFeed {
                url: "https://example.test/second".into(),
                title: Some("Second feed".into()),
                folder: None,
            },
        ];
        assert_eq!(test_db.db.import_feeds(&feeds).unwrap(), 2);

        let duplicate = [crate::opml::OpmlFeed {
            url: "https://example.test/feed".into(),
            title: Some("Replacement title".into()),
            folder: Some("Replacement folder".into()),
        }];
        assert_eq!(test_db.db.import_feeds(&duplicate).unwrap(), 0);

        let feeds = test_db.db.list_feeds().unwrap();
        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].title.as_deref(), Some("Imported title"));
        assert_eq!(feeds[0].folder.as_deref(), Some("Imported folder"));
        assert_eq!(feeds[1].title.as_deref(), Some("Second feed"));
    }

    #[test]
    fn deleting_feed_cascades_articles_sentences_and_dependent_story_text() {
        let test_db = TestDb::new();
        let feed_id = test_db.db.add_feed("https://example.test/feed").unwrap();
        let article_id = test_db
            .db
            .insert_article(&article(feed_id))
            .unwrap()
            .unwrap();
        let conn = test_db.db.0.writer.lock().unwrap();
        conn.execute(
            "INSERT INTO stories(id,title,centroid,first_seen_at,updated_at) VALUES (1,'Story',X'',0,0)",
            [],
        )
        .unwrap();
        conn.execute("UPDATE articles SET story_id=1 WHERE id=?1", [article_id])
            .unwrap();
        let sentence_id: i64 = conn
            .query_row(
                "SELECT id FROM sentences WHERE article_id=?1",
                [article_id],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO story_text(story_id,mode,position,text,sentence_id) \
             VALUES (1,'extractive',0,'One sentence.',?1)",
            [sentence_id],
        )
        .unwrap();
        drop(conn);

        assert!(test_db.db.delete_feed(feed_id).unwrap());
        let conn = test_db.db.0.readers[0].lock().unwrap();
        let article_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM articles", [], |r| r.get(0))
            .unwrap();
        let sentence_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sentences", [], |r| r.get(0))
            .unwrap();
        let story_text_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM story_text", [], |r| r.get(0))
            .unwrap();
        assert_eq!((article_count, sentence_count, story_text_count), (0, 0, 0));
        let story_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM stories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(story_count, 0);
    }

    #[test]
    fn assignment_invalidates_stale_ai_and_retention_keeps_saved_story() {
        let test_db = TestDb::new();
        let first_feed = test_db.db.add_feed("https://example.test/first").unwrap();
        let copy_feed = test_db.db.add_feed("https://example.test/copy").unwrap();
        let new_feed = test_db.db.add_feed("https://example.test/new").unwrap();

        let first_id = test_db
            .db
            .insert_article(&article(first_feed))
            .unwrap()
            .unwrap();
        let first = test_db
            .db
            .pending_articles_after(10, None, None)
            .unwrap()
            .remove(0);
        let first_sentence = first.sentences[0].0;
        let story_id = test_db
            .db
            .assign_article(
                &first,
                &[1.0, 0.0],
                &[],
                &[(first_sentence, vec![1.0, 0.0])],
                None,
                None,
                1.0,
                &[1.0, 0.0],
            )
            .unwrap();
        let stored = [StoredText {
            text: "The report confirms verified facts.".into(),
            sentence_id: first_sentence,
            extra_sources: Vec::new(),
            verified: true,
        }];
        test_db
            .db
            .store_text(
                story_id,
                "extractive",
                &stored,
                Some("originals-v1"),
                Some("Cached title"),
                &[],
            )
            .unwrap();
        assert!(
            test_db
                .db
                .store_ai_text_if_current(story_id, "originals-v1", &stored, None, &[])
                .unwrap()
        );
        assert!(test_db.db.has_ai_for(story_id, "originals-v1").unwrap());
        assert_eq!(
            test_db
                .db
                .story_list("search", Some("verified facts"), 0)
                .unwrap()
                .len(),
            1
        );

        let mut copy = article(copy_feed);
        copy.guid = "copy".into();
        copy.title = "Syndicated copy".into();
        let copy_id = test_db.db.insert_article(&copy).unwrap().unwrap();
        let pending_copy = test_db
            .db
            .pending_articles_after(10, None, None)
            .unwrap()
            .remove(0);
        test_db
            .db
            .assign_article(
                &pending_copy,
                &[1.0, 0.0],
                &[],
                &[],
                Some(story_id),
                Some(first_id),
                0.0,
                &[1.0, 0.0],
            )
            .unwrap();
        let copy_of: Option<i64> = test_db.db.0.readers[0]
            .lock()
            .unwrap()
            .query_row(
                "SELECT copy_of FROM articles WHERE id=?1",
                [copy_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(copy_of, Some(first_id));
        assert!(test_db.db.has_ai_for(story_id, "originals-v1").unwrap());

        let mut additional = article(new_feed);
        additional.guid = "new-original".into();
        test_db.db.insert_article(&additional).unwrap().unwrap();
        let pending_new = test_db
            .db
            .pending_articles_after(10, None, None)
            .unwrap()
            .remove(0);
        let new_sentence = pending_new.sentences[0].0;
        test_db
            .db
            .assign_article(
                &pending_new,
                &[0.0, 1.0],
                &[],
                &[(new_sentence, vec![0.0, 1.0])],
                Some(story_id),
                None,
                1.0,
                &[0.7, 0.7],
            )
            .unwrap();
        assert!(
            !test_db
                .db
                .store_ai_text_if_current(story_id, "originals-v1", &stored, None, &[])
                .unwrap()
        );
        assert!(!test_db.db.has_ai_for(story_id, "originals-v1").unwrap());
        assert!(
            test_db
                .db
                .story_list("search", Some("verified facts"), 0)
                .unwrap()
                .is_empty()
        );

        test_db
            .db
            .apply_score(
                story_id,
                77,
                12,
                "new_detail",
                Some("News"),
                false,
                false,
                2,
            )
            .unwrap();
        test_db.db.set_vote(story_id, 1).unwrap();
        let voted_at: i64 = test_db.db.0.readers[1]
            .lock()
            .unwrap()
            .query_row(
                "SELECT voted_at FROM stories WHERE id=?1",
                [story_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(voted_at > 0);
        assert!(
            test_db
                .db
                .voted_centroids(voted_at)
                .unwrap()
                .iter()
                .any(|(id, _, vote)| *id == story_id && *vote == 1)
        );
        assert!(test_db.db.voted_centroids(voted_at + 1).unwrap().is_empty());
        let for_you = test_db.db.story_list("for-you", None, 0).unwrap();
        assert_eq!(
            for_you
                .iter()
                .find(|story| story.id == story_id)
                .unwrap()
                .for_you,
            77
        );

        let fresh = [StoredText {
            text: "The report confirms verified facts.".into(),
            sentence_id: new_sentence,
            extra_sources: Vec::new(),
            verified: true,
        }];
        test_db
            .db
            .store_text(
                story_id,
                "extractive",
                &fresh,
                Some("originals-v2"),
                None,
                &[],
            )
            .unwrap();
        assert!(
            test_db
                .db
                .store_ai_text_if_current(story_id, "originals-v2", &fresh, None, &[])
                .unwrap()
        );
        assert!(test_db.db.delete_feed(first_feed).unwrap());
        assert!(
            !test_db
                .db
                .store_ai_text_if_current(story_id, "originals-v2", &fresh, None, &[])
                .unwrap()
        );
        assert!(!test_db.db.has_ai_for(story_id, "originals-v2").unwrap());
        let promoted_copy: Option<i64> = test_db.db.0.readers[0]
            .lock()
            .unwrap()
            .query_row(
                "SELECT copy_of FROM articles WHERE id=?1",
                [copy_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(promoted_copy, None);

        test_db.db.set_saved(story_id, true, 5_000).unwrap();
        test_db.db.close_and_retain(100_000, 1).unwrap();
        let saved_count: i64 = test_db.db.0.readers[2]
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM stories WHERE id=?1 AND saved_at IS NOT NULL",
                [story_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved_count, 1);
        assert_eq!(test_db.db.article_count().unwrap(), 2);
    }
}

// Reader and pipeline queries are kept together so web handlers contain no SQL.
#[derive(Clone, Debug)]
pub struct StoryListRow {
    pub id: i64,
    pub title: String,
    pub label: Option<String>,
    pub first_seen_at: i64,
    pub updated_at: i64,
    pub read_at: Option<i64>,
    pub for_you: i64,
    pub discover: i64,
    pub discover_reason: Option<String>,
    pub vote: i64,
    pub saved_at: Option<i64>,
    pub coverage: i64,
    pub topic_match: bool,
}
#[derive(Clone, Debug)]
pub struct TopicRow {
    pub id: i64,
    pub name: String,
    pub muted: bool,
}
#[derive(Clone, Debug)]
pub struct StorySentence {
    pub text: String,
    pub url: String,
    pub outlet: String,
    pub original: String,
    pub verified: bool,
    pub is_new: bool,
}
#[derive(Clone, Debug)]
pub struct ArticleSource {
    pub feed_id: i64,
    pub url: String,
    pub outlet: String,
}
#[derive(Clone, Debug)]
pub struct Disagreement {
    pub subject: String,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub struct StoryPage {
    pub story: StoryListRow,
    pub topic_match: Option<String>,
    pub text_mode: String,
    pub sentences: Vec<StorySentence>,
    pub sources: Vec<ArticleSource>,
    pub disagreements: Vec<Disagreement>,
}
#[derive(Clone, Debug)]
pub struct PendingArticle {
    pub id: i64,
    pub feed_id: i64,
    pub title: String,
    pub text: String,
    pub seen_at: i64,
    pub folder: Option<String>,
    pub outlet: String,
    pub feed_muted: bool,
    pub sentences: Vec<(i64, String)>,
}
#[derive(Clone, Debug)]
pub struct MemberArticle {
    pub id: i64,
    pub feed_id: i64,
    pub title: String,
    pub text: String,
    pub seen_at: i64,
    pub embedding: Vec<f32>,
    pub copy_of: Option<i64>,
    pub uniqueness: f32,
    pub folder: Option<String>,
    pub outlet: String,
    pub feed_muted: bool,
}
#[derive(Clone, Debug)]
pub struct StoryState {
    pub id: i64,
    pub centroid: Vec<f32>,
    pub first_seen_at: i64,
    pub updated_at: i64,
    pub closed: bool,
    pub vote: i64,
    pub text_built_for: Option<String>,
    pub members: Vec<MemberArticle>,
}
#[derive(Clone, Debug)]
pub struct EmbeddedTopic {
    pub id: i64,
    pub name: String,
    pub muted: bool,
    pub embedding: Vec<f32>,
}
#[derive(Clone, Debug)]
pub struct TextSentence {
    pub id: i64,
    pub article_id: i64,
    pub position: i64,
    pub text: String,
    pub embedding: Vec<f32>,
    pub article_seen_at: i64,
    pub uniqueness: f32,
    pub outlet: String,
}
#[derive(Clone, Debug)]
pub struct StoredText {
    pub text: String,
    pub sentence_id: i64,
    pub extra_sources: Vec<i64>,
    pub verified: bool,
}

pub fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|n| n.to_le_bytes()).collect()
}
pub fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|n| f32::from_le_bytes(*n))
        .collect()
}
fn map_story(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoryListRow> {
    Ok(StoryListRow {
        id: row.get(0)?,
        title: row.get(1)?,
        label: row.get(2)?,
        first_seen_at: row.get(3)?,
        updated_at: row.get(4)?,
        read_at: row.get(5)?,
        for_you: row.get(6)?,
        discover: row.get(7)?,
        discover_reason: row.get(8)?,
        vote: row.get(9)?,
        saved_at: row.get(10)?,
        coverage: row.get(11)?,
        topic_match: row.get(12)?,
    })
}
const STORY_COLUMNS: &str = "s.id,s.title,s.label,s.first_seen_at,s.updated_at,s.read_at,s.for_you,s.discover,s.discover_reason,s.vote,s.saved_at,s.coverage,s.topic_match";
impl Db {
    pub fn story_list(
        &self,
        tab: &str,
        query: Option<&str>,
        _min_score: u8,
    ) -> Result<Vec<StoryListRow>> {
        self.read(|conn| {
            let (suffix,argument)=match tab {
                "for-you"=>("WHERE s.muted=0 AND (s.topic_match=1 OR s.vote=1) ORDER BY s.for_you DESC,s.updated_at DESC",None),
                "discover"=>("WHERE s.muted=0 AND s.topic_match=0 AND s.vote<>1 AND s.updated_at>=?1 ORDER BY s.discover DESC,s.updated_at DESC LIMIT 30",Some((unix_now()-48*3600).to_string())),
                "saved"=>("WHERE s.muted=0 AND s.saved_at IS NOT NULL ORDER BY s.saved_at DESC",None),
                "search"=>{
                    let query=query.unwrap_or_default().split_whitespace().map(|word|format!("\"{}\"",word.replace('"',"\"\""))).collect::<Vec<_>>().join(" AND ");
                    if query.is_empty(){return Ok(Vec::new())}
                    ("JOIN stories_fts ON stories_fts.rowid=s.id WHERE s.muted=0 AND stories_fts MATCH ?1 ORDER BY bm25(stories_fts),s.updated_at DESC",Some(query))
                },
                _=>return Ok(Vec::new()),
            };
            let mut statement=conn.prepare(&format!("SELECT {STORY_COLUMNS} FROM stories s {suffix}"))?;
            let rows=if let Some(arg)=argument {statement.query_map([arg],map_story)?.collect::<rusqlite::Result<Vec<_>>>()?} else {statement.query_map([],map_story)?.collect::<rusqlite::Result<Vec<_>>>()?};
            Ok(rows)
        })
    }
    pub fn story_page(&self, id: i64, mode: Option<&str>, now: i64) -> Result<Option<StoryPage>> {
        self.write(|conn| {
            let tx=conn.transaction()?;
            let story=tx.query_row(&format!("SELECT {STORY_COLUMNS} FROM stories s WHERE s.id=?1 AND s.muted=0"),[id],map_story).optional()?;
            let Some(story)=story else{return Ok(None)};
            let ai:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM story_text WHERE story_id=?1 AND mode='ai')",[id],|r|r.get(0))?;
            let text_mode=if ai && mode!=Some("extractive"){"ai"}else{"extractive"};
            let mut statement=tx.prepare("SELECT t.text,a.url,COALESCE(f.title,f.url),n.text,t.verified,a.seen_at FROM story_text t JOIN sentences n ON n.id=t.sentence_id JOIN articles a ON a.id=n.article_id JOIN feeds f ON f.id=a.feed_id WHERE t.story_id=?1 AND t.mode=?2 AND a.copy_of IS NULL ORDER BY t.position")?;
            let sentences=statement.query_map(params![id,text_mode],|r|{
                let seen:i64=r.get(5)?;
                Ok(StorySentence{text:r.get(0)?,url:r.get(1)?,outlet:r.get(2)?,original:r.get(3)?,verified:r.get(4)?,is_new:story.read_at.is_some_and(|last|seen>last)})
            })?.collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            let mut statement=tx.prepare("SELECT a.feed_id,a.url,COALESCE(f.title,f.url) FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.story_id=?1 AND a.copy_of IS NULL ORDER BY a.seen_at,a.id")?;
            let sources=statement.query_map([id],|r|Ok(ArticleSource{feed_id:r.get(0)?,url:r.get(1)?,outlet:r.get(2)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            let mut statement=tx.prepare("SELECT subject,detail FROM disagreements WHERE story_id=?1 ORDER BY position")?;
            let disagreements=if text_mode=="ai" {statement.query_map([id],|r|Ok(Disagreement{subject:r.get(0)?,detail:r.get(1)?}))?.collect::<rusqlite::Result<Vec<_>>>()?} else{Vec::new()};
            drop(statement);
            tx.execute("UPDATE stories SET read_at=?2 WHERE id=?1",params![id,now])?;
            tx.commit()?;
            let topic_match=story.topic_match.then(||story.label.clone()).flatten();
            Ok(Some(StoryPage{story,topic_match,text_mode:text_mode.into(),sentences,sources,disagreements}))
        })
    }
    pub fn list_topics(&self) -> Result<Vec<TopicRow>> {
        self.read(|conn| {
            let mut statement =
                conn.prepare("SELECT id,name,muted FROM topics ORDER BY name COLLATE NOCASE")?;
            Ok(statement
                .query_map([], |r| {
                    Ok(TopicRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        muted: r.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }
    pub fn add_topic(&self, name: &str, embedding: &[f32], muted: bool, now: i64) -> Result<()> {
        anyhow::ensure!(
            !name.trim().is_empty() && name.chars().count() <= 200,
            "Enter a topic of 1 to 200 characters"
        );
        self.write(|conn|{conn.execute("INSERT INTO topics(name,embedding,muted,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(name) DO UPDATE SET muted=excluded.muted",params![name.trim(),encode_vector(embedding),muted,now])?;Ok(())})
    }
    pub fn delete_topic(&self, id: i64) -> Result<()> {
        self.write(|conn| {
            conn.execute("DELETE FROM topics WHERE id=?1", [id])?;
            Ok(())
        })
    }
    pub fn set_topic_muted(&self, id: i64, muted: bool) -> Result<()> {
        self.write(|conn| {
            conn.execute("UPDATE topics SET muted=?2 WHERE id=?1", params![id, muted])?;
            Ok(())
        })
    }
    pub fn set_feed_muted(&self, id: i64, muted: bool) -> Result<()> {
        self.write(|conn| {
            conn.execute("UPDATE feeds SET muted=?2 WHERE id=?1", params![id, muted])?;
            Ok(())
        })
    }
    pub fn set_vote(&self, id: i64, vote: i64) -> Result<()> {
        anyhow::ensure!((-1..=1).contains(&vote), "Invalid vote");
        self.write(|conn|{conn.execute("UPDATE stories SET vote=?2,voted_at=CASE WHEN ?2=0 THEN NULL ELSE ?3 END WHERE id=?1",params![id,vote,unix_now()])?;Ok(())})
    }
    pub fn toggle_saved(&self, id: i64, now: i64) -> Result<()> {
        self.write(|conn|{conn.execute("UPDATE stories SET saved_at=CASE WHEN saved_at IS NULL THEN ?2 ELSE NULL END WHERE id=?1",params![id,now])?;Ok(())})
    }
    pub fn set_saved(&self, id: i64, saved: bool, now: i64) -> Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE stories SET saved_at=?2 WHERE id=?1",
                params![id, if saved { Some(now) } else { None }],
            )?;
            Ok(())
        })
    }
    pub fn last_fetch_at(&self) -> Result<Option<i64>> {
        self.read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT value FROM app_state WHERE key='last_fetch_at'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .and_then(|v| v.parse().ok()))
        })
    }
    pub fn record_cycle(&self, now: i64) -> Result<()> {
        self.write(|conn|{conn.execute("INSERT INTO app_state(key,value) VALUES('last_fetch_at',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now.to_string()])?;Ok(())})
    }
    pub fn data_size(&self) -> Result<u64> {
        self.read(|conn| {
            let pages: u64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
            let size: u64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
            Ok(pages * size)
        })
    }
    pub fn embedded_topics(&self) -> Result<Vec<EmbeddedTopic>> {
        self.read(|conn| {
            let mut statement =
                conn.prepare("SELECT id,name,muted,embedding FROM topics ORDER BY id")?;
            Ok(statement
                .query_map([], |r| {
                    Ok(EmbeddedTopic {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        muted: r.get(2)?,
                        embedding: decode_vector(&r.get::<_, Vec<u8>>(3)?),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }
    pub fn update_topic_embedding(&self, id: i64, embedding: &[f32]) -> Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE topics SET embedding=?2 WHERE id=?1",
                params![id, encode_vector(embedding)],
            )?;
            Ok(())
        })
    }
    pub fn pending_articles_after(
        &self,
        limit: i64,
        after_seen_at: Option<i64>,
        after_id: Option<i64>,
    ) -> Result<Vec<PendingArticle>> {
        self.read(|conn|{
        let mut statement=conn.prepare("SELECT a.id,a.feed_id,a.title,COALESCE(a.text,''),a.seen_at,f.folder,COALESCE(f.title,f.url),f.muted FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.story_id IS NULL AND (?2 IS NULL OR a.seen_at>?2 OR (a.seen_at=?2 AND a.id>COALESCE(?3,0))) ORDER BY a.seen_at,a.id LIMIT ?1")?;
        let mut articles=statement.query_map(params![limit.max(0),after_seen_at,after_id],|r|Ok(PendingArticle{id:r.get(0)?,feed_id:r.get(1)?,title:r.get(2)?,text:r.get(3)?,seen_at:r.get(4)?,folder:r.get(5)?,outlet:r.get(6)?,feed_muted:r.get(7)?,sentences:Vec::new()}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut sentences=conn.prepare("SELECT id,text FROM sentences WHERE article_id=?1 ORDER BY position")?;
        for article in &mut articles{article.sentences=sentences.query_map([article.id],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;}
        Ok(articles)
    })
    }
    pub fn story_states(&self, since: i64) -> Result<Vec<StoryState>> {
        self.read(|conn|{
        let mut statement=conn.prepare("SELECT id,centroid,first_seen_at,updated_at,closed,vote,text_built_for FROM stories WHERE updated_at>=?1 ORDER BY first_seen_at,id")?;
        let mut stories=statement.query_map([since],|r|Ok(StoryState{id:r.get(0)?,centroid:decode_vector(&r.get::<_,Vec<u8>>(1)?),first_seen_at:r.get(2)?,updated_at:r.get(3)?,closed:r.get(4)?,vote:r.get(5)?,text_built_for:r.get(6)?,members:Vec::new()}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut members=conn.prepare("SELECT a.id,a.feed_id,a.title,COALESCE(a.text,''),a.seen_at,a.embedding,a.copy_of,COALESCE(a.uniqueness,0),f.folder,COALESCE(f.title,f.url),f.muted FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.story_id=?1 ORDER BY a.seen_at,a.id")?;
        for story in &mut stories {story.members=members.query_map([story.id],|r|Ok(MemberArticle{id:r.get(0)?,feed_id:r.get(1)?,title:r.get(2)?,text:r.get(3)?,seen_at:r.get(4)?,embedding:decode_vector(&r.get::<_,Option<Vec<u8>>>(5)?.unwrap_or_default()),copy_of:r.get(6)?,uniqueness:r.get(7)?,folder:r.get(8)?,outlet:r.get(9)?,feed_muted:r.get(10)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;}
        Ok(stories)
    })
    }
    /// Return embedding centroids for stories active in the supplied window.
    pub fn recent_centroids(&self, since: i64) -> Result<Vec<(i64, Vec<f32>)>> {
        self.read(|conn| {
            let mut statement =
                conn.prepare("SELECT id,centroid FROM stories WHERE updated_at>=?1 ORDER BY id")?;
            Ok(statement
                .query_map([since], |row| {
                    Ok((row.get(0)?, decode_vector(&row.get::<_, Vec<u8>>(1)?)))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }
    /// Return recent voted story centroids; positive votes are 1 and negative votes -1.
    pub fn voted_centroids(&self, since: i64) -> Result<Vec<(i64, Vec<f32>, i8)>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id,centroid,vote FROM stories WHERE voted_at>=?1 AND vote<>0 ORDER BY id",
            )?;
            Ok(statement
                .query_map([since], |row| {
                    Ok((
                        row.get(0)?,
                        decode_vector(&row.get::<_, Vec<u8>>(1)?),
                        row.get(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }
    /// Load one ID-ordered score batch without article text or embeddings.
    pub fn score_states_page(&self, after_id: i64, limit: i64) -> Result<Vec<StoryState>> {
        self.read(|conn|{
        let mut statement=conn.prepare("SELECT id,centroid,first_seen_at,updated_at,closed,vote,text_built_for FROM stories WHERE id>?1 ORDER BY id LIMIT ?2")?;
        let mut stories=statement.query_map(params![after_id,limit.max(1)],|row|Ok(StoryState{id:row.get(0)?,centroid:decode_vector(&row.get::<_,Vec<u8>>(1)?),first_seen_at:row.get(2)?,updated_at:row.get(3)?,closed:row.get(4)?,vote:row.get(5)?,text_built_for:row.get(6)?,members:Vec::new()}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut members=conn.prepare("SELECT a.id,a.feed_id,a.title,a.seen_at,a.copy_of,COALESCE(a.uniqueness,0),f.folder,COALESCE(f.title,f.url),f.muted FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.story_id=?1 ORDER BY a.seen_at,a.id")?;
        for story in &mut stories {story.members=members.query_map([story.id],|row|Ok(MemberArticle{id:row.get(0)?,feed_id:row.get(1)?,title:row.get(2)?,text:String::new(),seen_at:row.get(3)?,embedding:Vec::new(),copy_of:row.get(4)?,uniqueness:row.get(5)?,folder:row.get(6)?,outlet:row.get(7)?,feed_muted:row.get(8)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;}
        Ok(stories)
    })
    }
    pub fn stories_needing_text_page(&self, after_id: i64, limit: i64) -> Result<Vec<StoryState>> {
        self.read(|conn|{
        let mut statement=conn.prepare("SELECT id,centroid,first_seen_at,updated_at,closed,vote,text_built_for FROM stories WHERE text_built_for IS NULL AND id>?1 ORDER BY id LIMIT ?2")?;
        let mut stories=statement.query_map(params![after_id,limit.max(1)],|row|Ok(StoryState{id:row.get(0)?,centroid:decode_vector(&row.get::<_,Vec<u8>>(1)?),first_seen_at:row.get(2)?,updated_at:row.get(3)?,closed:row.get(4)?,vote:row.get(5)?,text_built_for:row.get(6)?,members:Vec::new()}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut members=conn.prepare("SELECT a.id,a.feed_id,a.title,a.seen_at,a.copy_of,COALESCE(a.uniqueness,0),f.folder,COALESCE(f.title,f.url),f.muted FROM articles a JOIN feeds f ON f.id=a.feed_id WHERE a.story_id=?1 ORDER BY a.seen_at,a.id")?;
        for story in &mut stories {story.members=members.query_map([story.id],|row|Ok(MemberArticle{id:row.get(0)?,feed_id:row.get(1)?,title:row.get(2)?,text:String::new(),seen_at:row.get(3)?,embedding:Vec::new(),copy_of:row.get(4)?,uniqueness:row.get(5)?,folder:row.get(6)?,outlet:row.get(7)?,feed_muted:row.get(8)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;}
        Ok(stories)
    })
    }
    // Article assignment persists independent embedding, deduplication, and
    // clustering outputs atomically with the fetched article.
    #[allow(clippy::too_many_arguments)]
    pub fn assign_article(
        &self,
        article: &PendingArticle,
        embedding: &[f32],
        minhash: &[u8],
        sentence_embeddings: &[(i64, Vec<f32>)],
        story_id: Option<i64>,
        copy_of: Option<i64>,
        uniqueness: f32,
        centroid: &[f32],
    ) -> Result<i64> {
        self.write(|conn|{
            let tx=conn.transaction()?;
            // Feeds can be deleted while CPU work is in flight.
            if !tx.query_row("SELECT EXISTS(SELECT 1 FROM articles WHERE id=?1)",[article.id],|r|r.get::<_,bool>(0))?{return Ok(0)}
            let id=if let Some(id)=story_id{id}else{
                tx.execute("INSERT INTO stories(title,centroid,first_seen_at,updated_at) VALUES(?1,?2,?3,?3)",params![article.title,encode_vector(centroid),article.seen_at])?;
                tx.last_insert_rowid()
            };
            tx.execute("UPDATE articles SET embedding=?2,minhash=?3,story_id=?4,copy_of=?5,uniqueness=?6 WHERE id=?1",params![article.id,encode_vector(embedding),minhash,id,copy_of,uniqueness])?;
            for (sentence_id,vector) in sentence_embeddings {tx.execute("UPDATE sentences SET embedding=?2 WHERE id=?1",params![sentence_id,encode_vector(vector)])?;}
            tx.execute("UPDATE stories SET centroid=?2,updated_at=CASE WHEN ?3 IS NULL THEN MAX(updated_at,?4) ELSE updated_at END,text_built_for=CASE WHEN ?3 IS NULL THEN NULL ELSE text_built_for END,text_version=CASE WHEN ?3 IS NULL THEN text_version+1 ELSE text_version END WHERE id=?1",params![id,encode_vector(centroid),copy_of,article.seen_at])?;
            if copy_of.is_none() {
                // Invalidate cached text in the same transaction that adds a
                // new original. In-flight AI results then fail their hash check.
                tx.execute("DELETE FROM story_text WHERE story_id=?1",[id])?;
                tx.execute("DELETE FROM disagreements WHERE story_id=?1",[id])?;
                update_fts(&tx,id)?;
            }
            tx.commit()?;Ok(id)
        })
    }
    pub fn text_candidates(&self, story_id: i64) -> Result<Vec<TextSentence>> {
        self.read(|conn|{
        let mut statement=conn.prepare("SELECT n.id,n.article_id,n.position,n.text,n.embedding,a.seen_at,COALESCE(a.uniqueness,0),COALESCE(f.title,f.url) FROM sentences n JOIN articles a ON a.id=n.article_id JOIN feeds f ON f.id=a.feed_id WHERE a.story_id=?1 AND a.copy_of IS NULL ORDER BY a.seen_at,a.id,n.position")?;
        Ok(statement.query_map([story_id],|r|Ok(TextSentence{id:r.get(0)?,article_id:r.get(1)?,position:r.get(2)?,text:r.get(3)?,embedding:decode_vector(&r.get::<_,Vec<u8>>(4)?),article_seen_at:r.get(5)?,uniqueness:r.get(6)?,outlet:r.get(7)?}))?.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    }
    pub fn text_candidates_limited(
        &self,
        story_id: i64,
        limit: usize,
    ) -> Result<Vec<TextSentence>> {
        self.read(|conn|{
        let mut statement=conn.prepare("WITH candidates AS (SELECT n.id,n.article_id,n.position,n.text,n.embedding,a.seen_at,a.uniqueness,COALESCE(f.title,f.url) AS outlet,ROW_NUMBER() OVER(PARTITION BY a.id ORDER BY n.position,n.id) AS source_rank FROM sentences n JOIN articles a ON a.id=n.article_id JOIN feeds f ON f.id=a.feed_id WHERE a.story_id=?1 AND a.copy_of IS NULL) SELECT id,article_id,position,text,embedding,seen_at,COALESCE(uniqueness,0),outlet FROM candidates WHERE source_rank<=?2 ORDER BY COALESCE(uniqueness,0) DESC,seen_at DESC,article_id,position LIMIT ?3")?;
        Ok(statement.query_map(params![story_id,crate::tuning::LLM_MAX_SENTENCES_PER_SOURCE.min(i64::MAX as usize) as i64,limit.min(i64::MAX as usize) as i64],|r|Ok(TextSentence{id:r.get(0)?,article_id:r.get(1)?,position:r.get(2)?,text:r.get(3)?,embedding:decode_vector(&r.get::<_,Vec<u8>>(4)?),article_seen_at:r.get(5)?,uniqueness:r.get(6)?,outlet:r.get(7)?}))?.collect::<rusqlite::Result<Vec<_>>>()?)
    })
    }
    pub fn store_text(
        &self,
        story_id: i64,
        mode: &str,
        text: &[StoredText],
        hash: Option<&str>,
        title: Option<&str>,
        disagreements: &[Disagreement],
    ) -> Result<()> {
        self.write(|conn|{
        let tx=conn.transaction()?;
        tx.execute("DELETE FROM story_text WHERE story_id=?1 AND mode=?2",params![story_id,mode])?;
        for (position,sentence) in text.iter().enumerate(){tx.execute("INSERT INTO story_text(story_id,mode,position,text,sentence_id,extra_sources,verified) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![story_id,mode,position as i64,sentence.text,sentence.sentence_id,serde_json::to_string(&sentence.extra_sources)?,sentence.verified])?;}
        if mode=="ai" {
            tx.execute("DELETE FROM disagreements WHERE story_id=?1",[story_id])?;
            for (position,d) in disagreements.iter().enumerate(){tx.execute("INSERT INTO disagreements(story_id,position,subject,detail) VALUES(?1,?2,?3,?4)",params![story_id,position as i64,d.subject,d.detail])?;}
        } else {
            // A changed set of originals invalidates the cached rewrite.
            tx.execute("DELETE FROM story_text WHERE story_id=?1 AND mode='ai'",[story_id])?;
            tx.execute("DELETE FROM disagreements WHERE story_id=?1",[story_id])?;
        }
        tx.execute("UPDATE stories SET title=COALESCE(?2,title),text_version=text_version+1,text_built_for=COALESCE(?3,text_built_for) WHERE id=?1",params![story_id,title,hash])?;
        update_fts(&tx,story_id)?;
        tx.commit()?;Ok(())
    })
    }
    /// Store a completed LLM result only if the story still has the source hash
    /// used to create it. This closes the race with an original joining while
    /// a slow model request is in flight.
    pub fn store_ai_text_if_current(
        &self,
        story_id: i64,
        expected_hash: &str,
        text: &[StoredText],
        title: Option<&str>,
        disagreements: &[Disagreement],
    ) -> Result<bool> {
        self.write(|conn|{
        let tx=conn.transaction()?;
        let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM stories WHERE id=?1 AND text_built_for=?2)",params![story_id,expected_hash],|row|row.get(0))?;
        if !current {tx.commit()?;return Ok(false)}
        tx.execute("DELETE FROM story_text WHERE story_id=?1 AND mode='ai'",[story_id])?;
        for (position,sentence) in text.iter().enumerate(){
            tx.execute("INSERT INTO story_text(story_id,mode,position,text,sentence_id,extra_sources,verified) VALUES(?1,'ai',?2,?3,?4,?5,?6)",params![story_id,position as i64,sentence.text,sentence.sentence_id,serde_json::to_string(&sentence.extra_sources)?,sentence.verified])?;
        }
        tx.execute("DELETE FROM disagreements WHERE story_id=?1",[story_id])?;
        for (position,d) in disagreements.iter().enumerate(){tx.execute("INSERT INTO disagreements(story_id,position,subject,detail) VALUES(?1,?2,?3,?4)",params![story_id,position as i64,d.subject,d.detail])?;}
        tx.execute("UPDATE stories SET title=COALESCE(?2,title),text_version=text_version+1 WHERE id=?1",params![story_id,title])?;
        update_fts(&tx,story_id)?;
        tx.commit()?;Ok(true)
    })
    }
    // Score components and display metadata are written as one story snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_score(
        &self,
        id: i64,
        for_you: u8,
        discover: u8,
        reason: &str,
        label: Option<&str>,
        topic_match: bool,
        muted: bool,
        coverage: usize,
    ) -> Result<()> {
        self.write(|conn|{
        conn.execute("UPDATE stories SET for_you=?2,discover=?3,discover_reason=?4,label=?5,topic_match=?6,muted=?7,coverage=?8 WHERE id=?1",params![id,for_you,discover,reason,label,topic_match,muted,coverage as i64])?;Ok(())
    })
    }
    pub fn close_and_retain(&self, now: i64, retention_days: u64) -> Result<()> {
        self.write(|conn|{
        let tx=conn.transaction()?;
        tx.execute("UPDATE stories SET closed=1 WHERE closed=0 AND (updated_at<=?1 OR first_seen_at<?2)",params![now-72*3600,now-10*86400])?;
        let cutoff=now.saturating_sub((retention_days.min(i64::MAX as u64/86400)*86400) as i64);
        tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS purge_article_ids(id INTEGER PRIMARY KEY); DELETE FROM purge_article_ids;")?;
        // Expired originals pull their copy descendants with them, regardless
        // of age. Otherwise a surviving copy would silently become an original.
        tx.execute(
            "WITH RECURSIVE doomed(id) AS (\
               SELECT a.id FROM articles a LEFT JOIN stories s ON s.id=a.story_id \
               WHERE a.seen_at<?1 AND (a.story_id IS NULL OR s.saved_at IS NULL) \
               UNION \
               SELECT child.id FROM articles child JOIN doomed d ON child.copy_of=d.id \
               LEFT JOIN stories s ON s.id=child.story_id WHERE s.saved_at IS NULL\
             ) INSERT OR IGNORE INTO purge_article_ids(id) SELECT id FROM doomed",
            [cutoff],
        )?;
        let affected_stories = {
            let mut statement=tx.prepare(
                "SELECT DISTINCT story_id FROM articles WHERE story_id IS NOT NULL AND id IN (SELECT id FROM purge_article_ids) \
                 UNION SELECT DISTINCT story_id FROM articles WHERE story_id IS NOT NULL AND copy_of IN (SELECT id FROM purge_article_ids)"
            )?;
            statement.query_map([],|row|row.get::<_,i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        // A saved story is kept forever. In the unusual case it points at a
        // source outside that story, detach it before the expired source goes.
        tx.execute(
            "UPDATE articles SET copy_of=NULL WHERE copy_of IN (SELECT id FROM purge_article_ids) \
             AND id NOT IN (SELECT id FROM purge_article_ids)",
            [],
        )?;
        tx.execute(
            "DELETE FROM story_text WHERE sentence_id IN (\
               SELECT n.id FROM sentences n JOIN articles a ON a.id=n.article_id \
               WHERE a.id IN (SELECT id FROM purge_article_ids)\
             )",
            [],
        )?;
        tx.execute("DELETE FROM articles WHERE id IN (SELECT id FROM purge_article_ids)",[])?;
        for story_id in affected_stories {
            repair_story_after_source_change(&tx, story_id)?;
        }
        // Covers old empty stories that predate this retention pass and keeps
        // their contentful FTS rows from becoming ghost search results.
        tx.execute("DELETE FROM stories_fts WHERE rowid IN (SELECT s.id FROM stories s WHERE NOT EXISTS(SELECT 1 FROM articles a WHERE a.story_id=s.id) AND s.saved_at IS NULL)",[])?;
        tx.execute("DELETE FROM stories WHERE saved_at IS NULL AND NOT EXISTS(SELECT 1 FROM articles WHERE story_id=stories.id)",[])?;
        tx.commit()?;
        conn.execute_batch("PRAGMA incremental_vacuum")?;Ok(())
    })
    }
}

/// Discard cached text derived from removed sources, recalculate story vectors
/// and coverage from all remaining members, and keep the contentful FTS row in
/// sync. An empty saved story is retained as an explicit-feed-delete exception.
fn repair_story_after_source_change(conn: &Connection, story_id: i64) -> Result<()> {
    let saved: Option<bool> = conn
        .query_row(
            "SELECT saved_at IS NOT NULL FROM stories WHERE id=?1",
            [story_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(saved) = saved else {
        return Ok(());
    };
    conn.execute("DELETE FROM story_text WHERE story_id=?1", [story_id])?;
    conn.execute("DELETE FROM disagreements WHERE story_id=?1", [story_id])?;
    let member_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM articles WHERE story_id=?1",
        [story_id],
        |row| row.get(0),
    )?;
    if member_count == 0 && !saved {
        conn.execute("DELETE FROM stories_fts WHERE rowid=?1", [story_id])?;
        conn.execute("DELETE FROM stories WHERE id=?1", [story_id])?;
        return Ok(());
    }

    let (embeddings, coverage) = {
        let mut statement = conn.prepare(
            "SELECT embedding FROM articles WHERE story_id=?1 AND embedding IS NOT NULL",
        )?;
        let vectors = statement
            .query_map([story_id], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|bytes| decode_vector(&bytes))
            .filter(|vector| !vector.is_empty())
            .collect::<Vec<_>>();
        let coverage: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT feed_id) FROM articles WHERE story_id=?1 AND copy_of IS NULL",
            [story_id],
            |row| row.get(0),
        )?;
        (vectors, coverage)
    };
    let centroid = crate::cluster::updated_centroid(&embeddings).unwrap_or_default();
    let title: Option<String> = conn
        .query_row(
            "SELECT title FROM articles WHERE story_id=?1 ORDER BY (copy_of IS NOT NULL),seen_at,id LIMIT 1",
            [story_id],
            |row| row.get(0),
        )
        .optional()?;
    conn.execute(
        "UPDATE stories SET title=COALESCE(?2,title),centroid=?3,coverage=?4, \
         text_built_for=NULL,text_version=text_version+1 WHERE id=?1",
        params![story_id, title, encode_vector(&centroid), coverage],
    )?;
    update_fts(conn, story_id)?;
    Ok(())
}

fn update_fts(conn: &Connection, story_id: i64) -> Result<()> {
    conn.execute("DELETE FROM stories_fts WHERE rowid=?1", [story_id])?;
    conn.execute("INSERT INTO stories_fts(rowid,title,body) SELECT s.id,s.title,COALESCE((SELECT group_concat(t.text,' ') FROM story_text t WHERE t.story_id=s.id AND t.mode=CASE WHEN EXISTS(SELECT 1 FROM story_text a WHERE a.story_id=s.id AND a.mode='ai') THEN 'ai' ELSE 'extractive' END),'') FROM stories s WHERE s.id=?1",[story_id])?;
    Ok(())
}

impl Db {
    pub fn close_stories(&self, now: i64) -> Result<()> {
        self.write(|conn|{
            conn.execute("UPDATE stories SET closed=1 WHERE closed=0 AND (updated_at<=?1 OR first_seen_at<=?2)",params![now-crate::tuning::STORY_IDLE_SECS,now-crate::tuning::STORY_MAX_AGE_SECS])?;
            Ok(())
        })
    }
}

#[cfg(test)]
impl Db {
    /// Shared SQLite in-memory storage for the offline full-pipeline fixture.
    pub(crate) fn memory() -> Result<Self> {
        static NEXT_MEMORY_DB: AtomicUsize = AtomicUsize::new(0);
        let uri = format!(
            "file:wagwan-memory-{}-{}?mode=memory&cache=shared",
            std::process::id(),
            NEXT_MEMORY_DB.fetch_add(1, Ordering::Relaxed)
        );
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let writer = Connection::open_with_flags(&uri, flags)?;
        writer.pragma_update(None, "foreign_keys", "ON")?;
        Self::migrate(&writer)?;
        let mut readers = Vec::new();
        for _ in 0..READ_CONNECTIONS {
            let reader = Connection::open_with_flags(&uri, flags)?;
            reader.pragma_update(None, "foreign_keys", "ON")?;
            readers.push(Mutex::new(reader));
        }
        Ok(Self(Arc::new(DbInner {
            writer: Mutex::new(writer),
            readers,
            next_reader: AtomicUsize::new(0),
        })))
    }
}
