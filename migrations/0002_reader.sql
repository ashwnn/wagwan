ALTER TABLE stories ADD COLUMN topic_match INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stories ADD COLUMN muted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE stories ADD COLUMN coverage INTEGER NOT NULL DEFAULT 1;
CREATE TABLE app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
DROP TABLE stories_fts;
CREATE VIRTUAL TABLE stories_fts USING fts5(title, body);
INSERT INTO stories_fts(rowid,title,body)
SELECT s.id,s.title,COALESCE((
  SELECT group_concat(t.text,' ')
  FROM story_text t
  WHERE t.story_id=s.id AND t.mode=CASE
    WHEN EXISTS(SELECT 1 FROM story_text ai WHERE ai.story_id=s.id AND ai.mode='ai') THEN 'ai'
    ELSE 'extractive'
  END
),'')
FROM stories s;
