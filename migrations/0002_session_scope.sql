-- Partition the usage history by browser session.
--
-- Until now every caller shared one bucket per table, so a test run's synthetic
-- readings and a household's real ones were indistinguishable, and a synthetic
-- import could change what a visitor saw. Each browser session now gets its own
-- partition.
--
-- `session_id` is a partition key, not an authentication token: it keeps data
-- apart, it does not protect it.
--
-- SQLite cannot drop the old unique constraint in place, so both tables are
-- rebuilt. D1's migration ledger applies each migration exactly once; the
-- rebuild is deliberately not guarded for re-runs, because a second run would
-- copy every session's rows back into the 'demo' bucket.
--
-- Rows that predate the change go into 'demo'. That was the single shared
-- bucket, so nothing is lost and nothing leaks into a fresh session.

-- ---------------------------------------------------------------------------
-- interval_reading: keyed by (session, interval start, interval length) rather
-- than (interval start, interval length) alone.
-- ---------------------------------------------------------------------------
CREATE TABLE interval_reading_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  -- The partition this reading belongs to. 'demo' is the fallback bucket, so a
  -- row inserted without an explicit session lands somewhere known.
  session_id TEXT NOT NULL DEFAULT 'demo',
  -- Epoch minutes at the start of the interval, normalised to UTC.
  start_epoch_minutes INTEGER NOT NULL,
  -- Interval length in minutes; must divide an hour.
  interval_minutes INTEGER NOT NULL,
  -- Energy drawn from the grid, in watt-hours.
  import_wh INTEGER NOT NULL,
  -- Energy pushed to the grid, in watt-hours. Zero unless supplied.
  export_wh INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE (session_id, start_epoch_minutes, interval_minutes)
);

-- Explicit ids are preserved so D1 keeps referring to the same rows and
-- `sqlite_sequence` advances past them for future autoincrement inserts.
INSERT INTO interval_reading_new
  (id, session_id, start_epoch_minutes, interval_minutes, import_wh, export_wh, created_at)
SELECT id, 'demo', start_epoch_minutes, interval_minutes, import_wh, export_wh, created_at
FROM interval_reading;

DROP TABLE interval_reading;

ALTER TABLE interval_reading_new RENAME TO interval_reading;

-- Range scans are always scoped to one session, so the session leads the key.
CREATE INDEX idx_reading_session_start
  ON interval_reading (session_id, start_epoch_minutes);

-- ---------------------------------------------------------------------------
-- bill: same partition, same rebuild.
-- ---------------------------------------------------------------------------
CREATE TABLE bill_new (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id TEXT NOT NULL DEFAULT 'demo',
  -- Inclusive bounds of the billed period, in epoch minutes.
  from_epoch_minutes INTEGER NOT NULL,
  to_epoch_minutes INTEGER NOT NULL,
  tariff_id TEXT NOT NULL,
  total_micro_usd INTEGER NOT NULL,
  total_import_wh INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO bill_new
  (id, session_id, from_epoch_minutes, to_epoch_minutes, tariff_id, total_micro_usd,
   total_import_wh, created_at)
SELECT id, 'demo', from_epoch_minutes, to_epoch_minutes, tariff_id, total_micro_usd,
       total_import_wh, created_at
FROM bill;

DROP TABLE bill;

ALTER TABLE bill_new RENAME TO bill;

CREATE INDEX idx_bill_session_range
  ON bill (session_id, from_epoch_minutes, to_epoch_minutes);
