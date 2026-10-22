ALTER TABLE stories ADD COLUMN voted_at INTEGER;
UPDATE stories SET voted_at=updated_at WHERE vote<>0;
CREATE INDEX stories_voted ON stories(voted_at) WHERE vote<>0;
