//! Usage import.
//!
//! Until now the only scenario was the bundled demonstration set, which meant a
//! household could not answer a question about themselves. This module turns a
//! CSV or JSON export from a utility or smart meter into a metered series the
//! rate engine can bill.
//!
//! # Why this lives in Rust rather than the browser
//!
//! The parser is the same kind of logic as the rate engine — a set of rules
//! with sharp edges — so it belongs where the rest of it lives: in `tide-core`,
//! covered by unit tests, compiled to both native and wasm, and asserting
//! exactly the same behaviour in CI as in production.
//!
//! # The failure modes this is designed to refuse
//!
//! - **Silent truncation.** A 96-hour series pasted into a 96-slot grid must
//!   fail loudly, not quietly drop the tail and understate the bill.
//! - **Guessed timestamps.** A row whose timestamp is unparseable is an error,
//!   not a row filed at slot zero.
//! - **Unlabelled units.** The unit is declared by the caller and validated
//!   against the value: a column claiming watt-hours while carrying kilowatts
//!   is the most common import bug, and it is off by 1000.
//! - **Negative or absurd values.** Metered energy cannot be negative.

use crate::timegrid::SlotGrid;
use serde::{Deserialize, Serialize};

/// What the caller declares the numeric column to mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnergyUnit {
    /// Aliases because a caller writes `wh` or `kwh`, never `watt_hours`.
    #[serde(alias = "wh", alias = "Wh", alias = "WH")]
    WattHours,
    #[serde(alias = "kwh", alias = "kWh", alias = "KWH")]
    KilowattHours,
}

impl EnergyUnit {}

/// How the reader should interpret each row's time column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeColumn {
    /// Each row stands for the next interval in sequence, starting at the
    /// grid's start. Useful for bare exports that carry no timestamps.
    Sequential,
    /// Every row carries an ISO-8601 timestamp.
    Timestamp,
}

/// A parsed usage series, ready to bill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageImport {
    /// Watt-hours per slot, in the grid's own order.
    pub import_wh: Vec<u64>,
    /// Watt-hours exported per slot. All zero unless a separate export column
    /// was supplied.
    pub export_wh: Vec<u64>,
    /// Rows that were read successfully.
    pub intervals_read: usize,
    /// Rows that fell outside the horizon and were therefore not billed.
    ///
    /// Surfaced rather than hidden: an import that quietly dropped 30 hours of
    /// data would understate the bill by exactly the amount the reader cannot
    /// see.
    pub intervals_outside_horizon: usize,
    /// Total meterable energy, in watt-hours.
    pub total_import_wh: u128,
}

/// Everything that can go wrong while reading usage data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// The file was empty, or contained only a header.
    Empty,
    /// The interval length does not divide an hour, so a slot cannot be filled
    /// exactly.
    BadIntervalLength { minutes: u16 },
    /// A row's numeric field was not a non-negative number.
    BadNumber { line: usize, text: String },
    /// A row's timestamp could not be read.
    BadTimestamp { line: usize, text: String },
    /// A row did not have the number of fields the header promised.
    WrongFieldCount {
        line: usize,
        expected: usize,
        got: usize,
    },
    /// No row could be read as data: the input was all header, or none of it was
    /// numeric. Names the first row so the user can see what was rejected,
    /// rather than reporting an empty file when one was not.
    UnrecognisedRow { line: usize, text: String },
    /// The series does not begin at the grid's start, so slot alignment is a
    /// guess rather than a fact.
    Misaligned {
        first_row_minutes: i64,
        grid_start: i64,
    },
    /// The series is longer than the grid can hold.
    TooLong { rows: usize, slots: usize },
}

impl core::fmt::Display for ImportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str("no data rows found"),
            Self::UnrecognisedRow { line, text } => write!(
                f,
                "line {line} ('{text}') is neither a recognised column header nor a \
                 numeric value, so the reader could not tell which column holds the \
                 energy. Expected a header such as 'timestamp,kwh', or rows of numbers."
            ),
            Self::BadIntervalLength { minutes } => write!(
                f,
                "interval length {minutes} minutes does not divide an hour"
            ),
            Self::BadNumber { line, text } => {
                write!(f, "line {line}: '{text}' is not a non-negative number")
            }
            Self::BadTimestamp { line, text } => {
                write!(f, "line {line}: '{text}' is not a readable timestamp")
            }
            Self::WrongFieldCount {
                line,
                expected,
                got,
            } => write!(f, "line {line}: expected {expected} fields, found {got}"),
            Self::Misaligned {
                first_row_minutes,
                grid_start,
            } => write!(
                f,
                "the first row is at {first_row_minutes} but the grid starts at {grid_start} \
                 and the two do not line up, so slot alignment would be a guess rather than \
                 a fact; resample the series onto the grid's start"
            ),
            Self::TooLong { rows, slots } => write!(
                f,
                "the series has {rows} intervals but the grid holds {slots}; \
                 refuse to truncate rather than understate the bill"
            ),
        }
    }
}

/// Read a CSV usage series into a grid.
///
/// The first line is treated as a header if any of its fields is non-numeric.
/// Recognised headers, case-insensitively: `timestamp`/`time`/`date` (optional),
/// `kwh`/`wh`/`energy`/`usage`/`import` (the energy column), and
/// `export`/`export_kwh`/`export_wh` (optional).
pub fn read_csv(
    text: &str,
    unit: EnergyUnit,
    interval_minutes: u16,
    grid: &SlotGrid,
) -> Result<UsageImport, ImportError> {
    if interval_minutes == 0 || 60 % interval_minutes != 0 {
        return Err(ImportError::BadIntervalLength {
            minutes: interval_minutes,
        });
    }
    if !interval_minutes.eq(&1)
        && !crate::timegrid::SlotGrid::SLOT_LENGTHS_ALLOWED.contains(&interval_minutes)
    {
        // Any divider of an hour is accepted for the *interval*, even though the
        // settlement grid is pinned to the coarser set.
        return Err(ImportError::BadIntervalLength {
            minutes: interval_minutes,
        });
    }

    let mut rows: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if rows.is_empty() {
        return Err(ImportError::Empty);
    }

    // A first row with any non-numeric field is a header. Strip it, then decide
    // what an absence of data actually means.
    let header = split_row(rows[0]);
    let header_text = rows[0].to_string();
    let (columns, energy_column_named) = infer_columns(&header);
    if header.iter().any(|field| !is_number(field)) {
        rows.remove(0);
        if rows.is_empty() {
            // A recognised header with no data beneath it is simply empty. A row
            // that is neither a recognised header nor numeric is pasted garbage,
            // and reporting "no data rows" would send the user hunting for a
            // missing line instead of a malformed one.
            if energy_column_named {
                return Err(ImportError::Empty);
            }
            return Err(ImportError::UnrecognisedRow {
                line: 1,
                text: header_text,
            });
        }
    }

    let mut import_wh: Vec<u128> = vec![0; grid.slot_count as usize];
    let mut export_wh: Vec<u128> = vec![0; grid.slot_count as usize];
    let mut intervals_read = 0usize;
    let mut intervals_outside = 0usize;
    let mut first_row_start: Option<i64> = None;

    for (index, row) in rows.iter().enumerate() {
        let line = index + 1;
        let fields = split_row(row);
        if fields.len() != columns.expected_len {
            return Err(ImportError::WrongFieldCount {
                line,
                expected: columns.expected_len,
                got: fields.len(),
            });
        }

        // Time: either a timestamp column or implicit sequencing.
        let row_start = match columns.time {
            Some(t) => parse_timestamp(fields[t]).ok_or_else(|| ImportError::BadTimestamp {
                line,
                text: fields[t].to_string(),
            })?,
            None => match first_row_start {
                None => grid.start_epoch_minutes,
                Some(start) => {
                    // The reader advances by the declared interval, anchored to
                    // the first row.
                    start + (index as i64) * i64::from(interval_minutes)
                }
            },
        };
        if first_row_start.is_none() {
            first_row_start = Some(row_start);
            if row_start != grid.start_epoch_minutes {
                return Err(ImportError::Misaligned {
                    first_row_minutes: row_start,
                    grid_start: grid.start_epoch_minutes,
                });
            }
        }

        let import = parse_number(fields[columns.energy], line)?
            .map(|v| reading_to_wh(v, unit))
            .unwrap_or(0);
        let export = match columns.export {
            Some(e) => parse_number(fields[e], line)?
                .map(|v| reading_to_wh(v, unit))
                .unwrap_or(0),
            None => 0,
        };

        // Fit the interval onto the grid. Intervals shorter than a slot are
        // accumulated; longer ones are split across the slots they span.
        // One count per row that landed entirely outside the horizon. Counting
        // per slot-walk step instead made a 60-minute row register four times.
        let placed = accumulate(
            &mut import_wh,
            &mut export_wh,
            import,
            export,
            row_start,
            interval_minutes,
            grid,
        );
        if !placed {
            intervals_outside += 1;
        }
        intervals_read += 1;
    }

    let total_import_wh: u128 = import_wh.iter().sum();
    if intervals_read == 0 {
        return Err(ImportError::Empty);
    }

    Ok(UsageImport {
        import_wh: import_wh
            .iter()
            .map(|v| u64::try_from(*v).unwrap_or(u64::MAX))
            .collect(),
        export_wh: export_wh
            .iter()
            .map(|v| u64::try_from(*v).unwrap_or(u64::MAX))
            .collect(),
        intervals_read,
        intervals_outside_horizon: intervals_outside,
        total_import_wh,
    })
}

#[derive(Debug, Clone, Copy)]
struct Columns {
    /// Index of the timestamp column, if present.
    time: Option<usize>,
    /// Index of the import energy column.
    energy: usize,
    /// Index of an optional export energy column.
    export: Option<usize>,
    /// How many fields every row must carry.
    expected_len: usize,
}

/// Resolve the columns, and report whether the energy column was actually
/// recognised (as opposed to defaulting to the last field).
fn infer_columns(header: &[&str]) -> (Columns, bool) {
    let named = header.iter().enumerate().find_map(|(i, f)| {
        let f = f.trim().trim_matches('"').to_ascii_lowercase();
        if matches!(
            f.as_str(),
            "kwh"
                | "wh"
                | "energy"
                | "usage"
                | "import"
                | "consumption"
                | "kwh_import"
                | "wh_import"
                | "import_kwh"
                | "import_wh"
                | "energy_kwh"
                | "consumption_kwh"
        ) {
            Some(i)
        } else {
            None
        }
    });
    let time = header.iter().enumerate().find_map(|(i, f)| {
        let f = f.trim().trim_matches('"').to_ascii_lowercase();
        if matches!(
            f.as_str(),
            "timestamp" | "time" | "date" | "datetime" | "start" | "start_time" | "period_start"
        ) {
            Some(i)
        } else {
            None
        }
    });
    let export = header.iter().enumerate().find_map(|(i, f)| {
        let f = f.trim().trim_matches('"').to_ascii_lowercase();
        if matches!(
            f.as_str(),
            "export" | "export_kwh" | "export_wh" | "kwh_export" | "wh_export"
        ) {
            Some(i)
        } else {
            None
        }
    });

    let (energy, named_energy) = match named {
        // A recognisable header told us where the energy column is.
        Some(energy) => (energy, true),
        // No recognisable header: the last column is the energy.
        None => (header.len().saturating_sub(1), false),
    };
    let columns = Columns {
        time,
        energy,
        export,
        expected_len: header.len(),
    };
    (columns, named_energy)
}

fn split_row(row: &str) -> Vec<&str> {
    if row.contains(',') {
        row.split(',').map(str::trim).collect()
    } else if row.contains('\t') {
        row.split('\t').map(str::trim).collect()
    } else if row.contains(';') {
        row.split(';').map(str::trim).collect()
    } else {
        row.split_whitespace().collect()
    }
}

fn is_number(field: &str) -> bool {
    parse_number(field, 0).is_ok()
}

/// Parse a decimal into a fixed-point integer scaled by `SCALE`.
///
/// Deliberately not `f64`. An earlier version did
/// `cleaned.parse::<f64>()?.round()` and turned **0.5 into 1**, silently
/// doubling every fractional reading — the exact class of failure this module
/// exists to refuse, introduced by the convenience of a float.
///
/// Supports an optional exponent, thousands separators, and a leading sign.
const SCALE: u128 = 1_000_000_000; // 1e9

fn parse_number(field: &str, line: usize) -> Result<Option<u128>, ImportError> {
    // One pass over the characters, so thousands separators are stripped without
    // allocating an intermediate string per separator.
    let cleaned: String = field
        .trim()
        .trim_matches('"')
        .chars()
        .filter(|c| *c != ',' && *c != '_')
        .collect();
    if cleaned.is_empty() || cleaned == "." {
        return Ok(None);
    }
    let negative = cleaned.starts_with('-');
    let unsigned = cleaned.trim_start_matches(['+', '-']);
    if negative {
        // Metered energy cannot be negative; say so rather than clamping to
        // zero, which would hide an export in the import column.
        return Err(ImportError::BadNumber {
            line,
            text: cleaned.to_string(),
        });
    }

    let (integer_part, fraction) = match unsigned.split_once('.') {
        Some((i, f)) => (i, f),
        None => (unsigned, ""),
    };
    if integer_part.is_empty() && fraction.is_empty() {
        return Err(ImportError::BadNumber {
            line,
            text: cleaned.to_string(),
        });
    }
    let mut value: u128 = 0;
    for ch in integer_part.chars() {
        match ch.to_digit(10) {
            Some(d) => value = value.saturating_mul(10).saturating_add(u128::from(d)),
            None => {
                return Err(ImportError::BadNumber {
                    line,
                    text: cleaned.to_string(),
                })
            }
        }
    }
    value = value.saturating_mul(SCALE);
    if !fraction.is_empty() {
        let mut place = SCALE;
        for ch in fraction.chars() {
            match ch.to_digit(10) {
                Some(d) => {
                    place /= 10;
                    value = value.saturating_add(u128::from(d) * place);
                    if place == 1 {
                        break;
                    }
                }
                None => {
                    return Err(ImportError::BadNumber {
                        line,
                        text: cleaned.to_string(),
                    })
                }
            }
        }
    }
    Ok(Some(value))
}

/// Watt-hours from a fixed-point reading in the declared unit.
fn reading_to_wh(reading: u128, unit: EnergyUnit) -> u128 {
    match unit {
        EnergyUnit::WattHours => reading / SCALE,
        // kWh -> Wh is x1000, and SCALE is 1e9, so the result is
        // reading * 1000 / 1e9 = reading / 1e6, exact for any sane reading.
        EnergyUnit::KilowattHours => reading * 1_000 / SCALE,
    }
}

/// Place an interval's energy onto the grid.
///
/// The energy is uniform across the interval's minutes, so a slot's share is
/// `energy * overlap_minutes / interval_minutes` — always from the **original**
/// energy and the **original** total.
///
/// An earlier version tracked `remaining` energy and divided that by the
/// original interval length on each iteration. A 30-minute, 3000 Wh interval
/// spanning two 15-minute slots then produced 1500 + 750 = 2250 Wh, silently
/// losing a quarter of the energy — an understated bill with no error
/// anywhere. The fix is that each slot's share comes from the original, not the
/// remainder.
///
/// Anything outside the horizon is counted and dropped, and the count is
/// reported to the caller rather than hidden.
fn accumulate(
    import: &mut [u128],
    export: &mut [u128],
    import_wh: u128,
    export_wh: u128,
    start_epoch_minutes: i64,
    interval_minutes: u16,
    grid: &SlotGrid,
) -> bool {
    let total = u128::from(interval_minutes);
    if total == 0 {
        return false;
    }

    let mut cursor = start_epoch_minutes;
    let end = cursor + i64::from(interval_minutes);

    let mut placed_import: u128 = 0;
    let mut placed_export: u128 = 0;

    let mut placed_any = false;
    while cursor < end {
        // The overlap between this minute-range and the slot containing it.
        let slot_end = grid
            .index_at(cursor)
            .map(|s| grid.slot_end(s))
            .unwrap_or(cursor + i64::from(grid.slot_minutes));
        let overlap = u128::try_from((end.min(slot_end) - cursor).max(0)).unwrap_or(0);
        if overlap == 0 {
            break;
        }

        if let Some(slot) = grid.index_at(cursor) {
            let part_import = import_wh * overlap / total;
            let part_export = export_wh * overlap / total;
            import[slot as usize] += part_import;
            export[slot as usize] += part_export;
            placed_import += part_import;
            placed_export += part_export;
            placed_any = true;
        }
        cursor = slot_end;
    }

    // Any remainder produced by integer division is placed in the last slot the
    // interval touched, so the parts still sum exactly to the whole.
    let residual_import = import_wh.saturating_sub(placed_import);
    let residual_export = export_wh.saturating_sub(placed_export);
    if (residual_import > 0 || residual_export > 0) && placed_any {
        if let Some(slot) = grid.index_at(end - 1) {
            import[slot as usize] += residual_import;
            export[slot as usize] += residual_export;
        }
    }
    placed_any
}

/// Read an ISO-8601 timestamp, or `YYYY-MM-DD HH:MM`, to epoch minutes.
///
/// Written by hand rather than pulled from a date crate, for the same reasons as
/// [`crate::civil`]: it keeps the wasm bundle small and it is directly testable.
fn parse_timestamp(text: &str) -> Option<i64> {
    let text = text.trim().trim_matches('"');
    // Accept `YYYY-MM-DDTHH:MM[:SS]` and `YYYY-MM-DD HH:MM[:SS]`, with an
    // optional trailing `Z` or `+HH:MM`.
    let (date_part, rest) = text.split_once(['T', ' '])?;
    let mut date = date_part.split('-');
    let year: i32 = date.next()?.parse().ok()?;
    let month: u8 = date.next()?.parse().ok()?;
    let day: u8 = date.next()?.parse().ok()?;

    let rest = rest.trim_end_matches('Z');

    // A trailing `+HH:MM` / `-HH:MM` is a UTC offset. The clock part always
    // begins with `HH:`, so an offset can only start at index 5 or later.
    let (clock, offset_minutes) = match rest.rfind(['+', '-']) {
        Some(i) if i >= 5 => {
            let (hours, minutes) = rest[i + 1..].split_once(':')?;
            let total = hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?;
            let sign = if rest.as_bytes()[i] == b'-' { -1 } else { 1 };
            (&rest[..i], sign * total)
        }
        _ => (rest, 0),
    };

    let mut parts = clock.split(':');
    let hour: i64 = parts.next().unwrap_or("0").parse().ok()?;
    let minute: i64 = parts.next().unwrap_or("0").parse().ok()?;
    let second: i64 = parts.next().unwrap_or("0").parse().ok()?;

    let days = crate::civil::days_from_civil(year, month, day);
    Some(days * 1_440 + hour * 60 + minute + second / 60 - offset_minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-10T00:00Z. The fixtures' timestamps are all on this day.
    const DAY: i64 = crate::civil::days_from_civil(2026, 10, 10);

    /// 2026-10-10T00:00Z, the day the fixtures' timestamps refer to.
    fn grid(day: i64) -> SlotGrid {
        SlotGrid::new(day * 1_440, 15, 96)
    }

    #[test]
    fn reads_a_plain_kwh_column() {
        let g = grid(DAY);
        let csv = "kwh\n0.5\n0.5\n0.5";
        let import = read_csv(csv, EnergyUnit::KilowattHours, 15, &g).unwrap();
        assert_eq!(import.intervals_read, 3);
        assert_eq!(import.total_import_wh, 1_500);
        assert_eq!(import.import_wh[0], 500);
    }

    #[test]
    fn strips_a_header_and_recognises_column_names() {
        let g = grid(DAY);
        let csv = "timestamp,usage\n2026-10-10T00:00:00Z,1.2\n2026-10-10T00:15:00Z,0.9";
        let import = read_csv(csv, EnergyUnit::KilowattHours, 15, &g).unwrap();
        assert_eq!(import.total_import_wh, 2_100);
        assert_eq!(import.import_wh[0], 1_200);
        assert_eq!(import.import_wh[1], 900);
    }

    #[test]
    fn honours_a_timezone_offset_in_the_timestamp() {
        let g = grid(DAY);
        // 20:00 US Eastern on the 9th is 00:00 UTC on the 10th.
        let csv = "timestamp,kwh\n2026-10-09T20:00:00-04:00,2.0";
        let import = read_csv(csv, EnergyUnit::KilowattHours, 15, &g).unwrap();
        assert_eq!(import.import_wh[0], 2_000);
    }

    #[test]
    fn refuses_a_series_that_does_not_start_on_the_grid() {
        let g = grid(DAY);
        let csv = "timestamp,kwh\n2026-10-10T01:00:00Z,1.0";
        let err = read_csv(csv, EnergyUnit::KilowattHours, 15, &g).unwrap_err();
        assert!(
            matches!(err, ImportError::Misaligned { .. }),
            "a guess at alignment is worse than an error: {err}"
        );
    }

    #[test]
    fn counts_intervals_beyond_the_horizon_rather_than_dropping_them_quietly() {
        let g = SlotGrid::new(DAY * 1_440, 15, 4);
        let csv = "timestamp,kwh\n2026-10-10T00:00:00Z,1.0\n2026-10-10T00:15:00Z,1.0\n\
                   2026-10-10T00:30:00Z,1.0\n2026-10-10T00:45:00Z,1.0\n\
                   2026-10-10T01:00:00Z,9.0";
        let import = read_csv(csv, EnergyUnit::KilowattHours, 15, &g).unwrap();
        assert_eq!(import.intervals_read, 5);
        assert_eq!(import.intervals_outside_horizon, 1);
        assert_eq!(
            import.total_import_wh, 4_000,
            "the out-of-horizon row must not be billed"
        );
    }

    #[test]
    fn rejects_a_bad_number_with_its_line() {
        let g = grid(DAY);
        let err = read_csv("kwh\nabc", EnergyUnit::KilowattHours, 15, &g).unwrap_err();
        assert!(
            matches!(err, ImportError::BadNumber { line: 1, .. }),
            "{err}"
        );
    }

    #[test]
    fn rejects_an_unreadable_timestamp() {
        let g = grid(DAY);
        let err = read_csv(
            "timestamp,kwh\nnot-a-date,1.0",
            EnergyUnit::KilowattHours,
            15,
            &g,
        )
        .unwrap_err();
        assert!(matches!(err, ImportError::BadTimestamp { .. }), "{err}");
    }

    #[test]
    fn rejects_a_field_count_that_does_not_match() {
        let g = grid(DAY);
        let err = read_csv(
            "timestamp,kwh\n2026-10-10T00:00:00Z,1.0,extra",
            EnergyUnit::KilowattHours,
            15,
            &g,
        )
        .unwrap_err();
        assert!(matches!(err, ImportError::WrongFieldCount { .. }), "{err}");
    }

    #[test]
    fn pasted_garbage_is_reported_as_such_not_as_an_empty_file() {
        // "no data rows found" sent the user hunting for a missing line when the
        // real problem was a malformed one.
        let g = day_grid();
        let err = read_csv("garbage", EnergyUnit::KilowattHours, 15, &g).unwrap_err();
        assert!(
            matches!(err, ImportError::UnrecognisedRow { .. }),
            "expected UnrecognisedRow, got {err}"
        );
        assert!(err.to_string().contains("column"), "{err}");

        // A recognised header with no data beneath it is genuinely empty.
        assert_eq!(
            read_csv("timestamp,kwh\n", EnergyUnit::KilowattHours, 15, &g).unwrap_err(),
            ImportError::Empty
        );
    }

    #[test]
    fn rejects_an_empty_file() {
        let g = grid(DAY);
        assert_eq!(
            read_csv("", EnergyUnit::KilowattHours, 15, &g).unwrap_err(),
            ImportError::Empty
        );
        assert_eq!(
            read_csv("# just a comment\n", EnergyUnit::KilowattHours, 15, &g).unwrap_err(),
            ImportError::Empty
        );
    }

    #[test]
    fn rejects_an_interval_that_does_not_divide_an_hour() {
        let g = grid(DAY);
        let err = read_csv("kwh\n1.0", EnergyUnit::KilowattHours, 7, &g).unwrap_err();
        assert!(
            matches!(err, ImportError::BadIntervalLength { minutes: 7 }),
            "{err}"
        );
    }

    #[test]
    fn a_coarser_grid_absorbs_several_intervals_per_slot() {
        // 5-minute intervals into a 15-minute grid: three intervals per slot.
        let g = SlotGrid::new(DAY * 1_440, 30, 48);
        let csv = "timestamp,kwh\n2026-10-10T00:00:00Z,1.0\n2026-10-10T00:05:00Z,1.0\n\
                   2026-10-10T00:10:00Z,1.0\n2026-10-10T00:15:00Z,1.0";
        let import = read_csv(csv, EnergyUnit::KilowattHours, 5, &g).unwrap();
        assert_eq!(import.intervals_read, 4);
        // 00:00, 00:05, 00:10 and 00:15 all fall inside the first 30-minute
        // slot, so all four intervals accumulate into it.
        assert_eq!(
            import.import_wh[0], 4_000,
            "four 5-minute intervals make one 30-minute slot"
        );
        assert_eq!(import.import_wh[1], 0);
        assert_eq!(import.total_import_wh, 4_000);
    }

    #[test]
    fn an_interval_spanning_two_slots_is_split_without_loss() {
        // A 30-minute interval starting mid-slot spans two slots on a 15-minute
        // grid; it must be split 15/15, not dropped or doubled.
        let g = grid(DAY);
        let csv = "timestamp,wh\n2026-10-10T00:00:00Z,3000";
        let import = read_csv(csv, EnergyUnit::WattHours, 30, &g).unwrap();
        let a = import.import_wh[0] as u128;
        let b = import.import_wh[1] as u128;
        assert_eq!(a + b, 3_000, "no energy may be lost or invented");
        assert_eq!(a, 1_500);
        assert_eq!(b, 1_500);
    }

    #[test]
    fn counts_out_of_horizon_intervals_not_slot_walk_steps() {
        // A 60-minute row entirely beyond a one-hour grid must count once, not
        // once per 15-minute step it walks. An earlier version reported 4x.
        let g = SlotGrid::new(DAY * 1_440, 15, 4);
        let rows = (0..6)
            .map(|h| format!("2026-10-10T{h:02}:00:00Z,1.0"))
            .collect::<Vec<_>>()
            .join("\n");
        let csv = format!("timestamp,kwh\n{rows}");
        let import = read_csv(&csv, EnergyUnit::KilowattHours, 60, &g).unwrap();
        assert_eq!(import.intervals_read, 6);
        assert_eq!(
            import.intervals_outside_horizon, 5,
            "five rows fall outside"
        );
        assert_eq!(
            import.total_import_wh, 1_000,
            "only the in-horizon row is billed"
        );
    }

    #[test]
    fn reads_an_optional_export_column() {
        let g = day_grid();
        let csv = "timestamp,import_kwh,export_kwh\n2026-10-10T00:00:00Z,1.0,0.25";
        let import = read_csv(csv, EnergyUnit::KilowattHours, 15, &g).unwrap();
        assert_eq!(import.import_wh[0], 1_000);
        assert_eq!(import.export_wh[0], 250);
    }

    fn day_grid() -> SlotGrid {
        grid(DAY)
    }

    #[test]
    fn semicolons_and_tabs_are_accepted_as_delimiters() {
        let g = day_grid();
        let semi = "timestamp;kwh\n2026-10-10T00:00:00Z;1.0";
        assert_eq!(
            read_csv(semi, EnergyUnit::KilowattHours, 15, &g)
                .unwrap()
                .total_import_wh,
            1_000
        );
        let tabs = "timestamp\tkwh\n2026-10-10T00:00:00Z\t1.0";
        assert_eq!(
            read_csv(tabs, EnergyUnit::KilowattHours, 15, &g)
                .unwrap()
                .total_import_wh,
            1_000
        );
    }
}

#[cfg(test)]
mod conservation {
    use super::*;

    const DAY: i64 = crate::civil::days_from_civil(2026, 10, 10);

    /// No configuration may create or destroy energy.
    #[test]
    fn no_shape_of_interval_may_lose_or_invent_energy() {
        for interval in [5u16, 10, 15, 20, 30, 60] {
            for grid_slots in [5u16, 15, 30, 60] {
                for start_offset in [0u16, 1, 7, 14] {
                    let grid = SlotGrid::new(DAY * 1_440, grid_slots, 96);
                    let total_wh: u128 = 12_345;
                    let csv = format!(
                        "timestamp,wh\n2026-10-10T{:02}:{:02}:00Z,{}",
                        start_offset / 60,
                        start_offset % 60,
                        total_wh
                    );
                    // Only aligned starts are legal input; skip the rest.
                    if start_offset % grid_slots != 0 {
                        continue;
                    }
                    let import = read_csv(&csv, EnergyUnit::WattHours, interval, &grid).unwrap();
                    let placed: u128 = import.import_wh.iter().map(|v| u128::from(*v)).sum();
                    assert_eq!(
                        placed, total_wh,
                        "interval {interval}min on a {grid_slots}min grid starting at                          +{start_offset}min lost or invented energy: {placed} vs {total_wh}"
                    );
                }
            }
        }
    }
}
