CREATE TABLE feeds (
  id              INTEGER PRIMARY KEY,
  url             TEXT NOT NULL UNIQUE,
  title           TEXT,
  site_url        TEXT,
  folder          TEXT,
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

CREATE TABLE stories (
  id              INTEGER PRIMARY KEY,
  title           TEXT NOT NULL,
  label           TEXT,
  centroid        BLOB NOT NULL,
  first_seen_at   INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  closed          INTEGER NOT NULL DEFAULT 0,
  for_you         INTEGER NOT NULL DEFAULT 0,
  discover        INTEGER NOT NULL DEFAULT 0,
  discover_reason TEXT,
  text_version    INTEGER NOT NULL DEFAULT 0,
  text_built_for  TEXT,
  read_at         INTEGER,
  saved_at        INTEGER,
  vote            INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX stories_updated ON stories(updated_at);

CREATE TABLE articles (
  id              INTEGER PRIMARY KEY,
  feed_id         INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
  guid            TEXT NOT NULL,
  url             TEXT NOT NULL,
  title           TEXT NOT NULL,
  published_at    INTEGER,
  seen_at         INTEGER NOT NULL,
  text            TEXT,
  text_source     TEXT NOT NULL,
  embedding       BLOB,
  minhash         BLOB,
  story_id        INTEGER REFERENCES stories(id),
  copy_of         INTEGER REFERENCES articles(id),
  uniqueness      REAL,
  UNIQUE (feed_id, guid)
);
CREATE INDEX articles_story ON articles(story_id);
CREATE INDEX articles_seen ON articles(seen_at);

CREATE TABLE sentences (
  id              INTEGER PRIMARY KEY,
  article_id      INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
  position        INTEGER NOT NULL,
  text            TEXT NOT NULL,
  embedding       BLOB NOT NULL
);
CREATE INDEX sentences_article ON sentences(article_id);

CREATE TABLE story_text (
  story_id        INTEGER NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
  mode            TEXT NOT NULL,
  position        INTEGER NOT NULL,
  text            TEXT NOT NULL,
  sentence_id     INTEGER NOT NULL REFERENCES sentences(id),
  extra_sources   TEXT,
  verified        INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY (story_id, mode, position)
);

CREATE TABLE disagreements (
  story_id        INTEGER NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
  position        INTEGER NOT NULL,
  subject         TEXT NOT NULL,
  detail          TEXT NOT NULL,
  PRIMARY KEY (story_id, position)
);

CREATE TABLE topics (
  id              INTEGER PRIMARY KEY,
  name            TEXT NOT NULL UNIQUE,
  embedding       BLOB NOT NULL,
  muted           INTEGER NOT NULL DEFAULT 0,
  created_at      INTEGER NOT NULL
);

CREATE VIRTUAL TABLE stories_fts USING fts5(title, body, content='');
