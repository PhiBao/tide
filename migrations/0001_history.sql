-- Tide's accumulated usage history.
--
-- One row per imported interval, keyed by the interval's start time so that
-- re-importing the same export is idempotent rather than duplicating a month of
-- readings.
CREATE TABLE IF NOT EXISTS interval_reading (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  -- Epoch minutes at the start of the interval, normalised to UTC.
  start_epoch_minutes INTEGER NOT NULL,
  -- Interval length in minutes; must divide an hour.
  interval_minutes INTEGER NOT NULL,
  -- Energy drawn from the grid, in watt-hours.
  import_wh INTEGER NOT NULL,
  -- Energy pushed to the grid, in watt-hours. Zero unless supplied.
  export_wh INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE (start_epoch_minutes, interval_minutes)
);

CREATE INDEX IF NOT EXISTS idx_reading_start
  ON interval_reading (start_epoch_minutes);

-- One row per stored bill, so an earlier bill stays comparable after the rate
-- engine changes rather than being silently recomputed.
CREATE TABLE IF NOT EXISTS bill (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  -- Inclusive bounds of the billed period, in epoch minutes.
  from_epoch_minutes INTEGER NOT NULL,
  to_epoch_minutes INTEGER NOT NULL,
  tariff_id TEXT NOT NULL,
  total_micro_usd INTEGER NOT NULL,
  total_import_wh INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_bill_range
  ON bill (from_epoch_minutes, to_epoch_minutes);
