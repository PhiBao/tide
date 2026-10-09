//! The settlement grid.
//!
//! A grid is an absolute UTC timeline of equal-length slots. Everything the
//! product computes — prices, schedules, bills — is expressed in slot indices,
//! never in local wall-clock time.
//!
//! This is the central DST-safety decision: local time only ever appears at
//! the *edges* of the system (tarriff definitions in, display out), and it is
//! resolved to absolute instants by [`crate::zone::LocalZone`]. No arithmetic
//! anywhere in the solver or the rate engine ever touches a local clock, so a
//! daylight-saving transition cannot make a schedule off-by-one.

/// An absolute, uniform settlement grid.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SlotGrid {
    /// Epoch minute (UTC) of the start of slot 0.
    pub start_epoch_minutes: i64,
    /// Length of one slot in minutes. Must divide 60 so that whole slots never
    /// straddle an hour boundary.
    pub slot_minutes: u16,
    /// Number of slots.
    pub slot_count: u32,
}

impl SlotGrid {
    #[must_use]
    pub const fn new(start_epoch_minutes: i64, slot_minutes: u16, slot_count: u32) -> Self {
        Self { start_epoch_minutes, slot_minutes, slot_count }
    }

    /// A grid starting now-ish, covering `hours` at the given slot length.
    #[must_use]
    pub fn covering(start_epoch_minutes: i64, slot_minutes: u16, hours: u16) -> Self {
        let slots = (u32::from(hours) * 60) / u32::from(slot_minutes);
        Self { start_epoch_minutes, slot_minutes, slot_count: slots }
    }

    pub const SLOT_LENGTHS_ALLOWED: [u16; 6] = [5, 10, 15, 20, 30, 60];

    /// Every structural problem with this grid, in a stable order.
    ///
    /// An empty `Vec` means the grid is usable. These are hard validation
    /// errors, surfaced to the caller as HTTP 422 rather than silently
    /// repaired, because a repaired grid would produce a schedule that is
    /// correct for a different question than the one asked.
    #[must_use]
    pub fn validate(&self) -> Vec<GridFault> {
        let mut faults = Vec::new();
        if self.slot_count == 0 {
            faults.push(GridFault::Empty);
        }
        if self.slot_count > MAX_SLOTS {
            faults.push(GridFault::TooManySlots { found: self.slot_count, max: MAX_SLOTS });
        }
        if !Self::SLOT_LENGTHS_ALLOWED.contains(&self.slot_minutes) {
            faults.push(GridFault::SlotLengthDoesNotDivideHour { found: self.slot_minutes });
        }
        faults
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }

    /// Epoch minute at which slot `index` starts. Panics on out-of-range
    /// index in debug builds; callers that receive untrusted indices must use
    /// [`Self::try_slot_start`].
    #[must_use]
    pub fn slot_start(&self, index: u32) -> i64 {
        debug_assert!(index < self.slot_count, "slot index out of range");
        self.start_epoch_minutes + i64::from(index) * i64::from(self.slot_minutes)
    }

    #[must_use]
    pub fn try_slot_start(&self, index: u32) -> Option<i64> {
        if index < self.slot_count {
            Some(self.slot_start(index))
        } else {
            None
        }
    }

    #[must_use]
    pub fn slot_end(&self, index: u32) -> i64 {
        self.slot_start(index) + i64::from(self.slot_minutes)
    }

    /// Index of the slot containing `epoch_minutes`, or `None` if it falls
    /// outside the grid.
    #[must_use]
    pub fn index_at(&self, epoch_minutes: i64) -> Option<u32> {
        let offset = epoch_minutes - self.start_epoch_minutes;
        if offset < 0 {
            return None;
        }
        let index = offset / i64::from(self.slot_minutes);
        if index < i64::from(self.slot_count) && offset % i64::from(self.slot_minutes) == 0 {
            u32::try_from(index).ok()
        } else if index < i64::from(self.slot_count) {
            // A non-aligned instant still belongs to the slot that contains it.
            u32::try_from(index).ok()
        } else {
            None
        }
    }

    #[must_use]
    pub fn total_minutes(&self) -> i64 {
        i64::from(self.slot_count) * i64::from(self.slot_minutes)
    }

    #[must_use]
    pub fn covers_epoch(&self, epoch_minutes: i64) -> bool {
        epoch_minutes >= self.start_epoch_minutes
            && epoch_minutes < self.start_epoch_minutes + self.total_minutes()
    }
}

/// Upper bound on grid size. A 15-minute grid over 8 days is 768 slots; 16k
/// slots is a year of 15-minute settlement, far beyond any household decision
/// and a guard against a hostile request allocating unbounded memory.
pub const MAX_SLOTS: u32 = 16_384;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridFault {
    Empty,
    TooManySlots { found: u32, max: u32 },
    SlotLengthDoesNotDivideHour { found: u16 },
}

impl core::fmt::Display for GridFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str("grid has no slots"),
            Self::TooManySlots { found, max } => {
                write!(f, "grid has {found} slots, maximum is {max}")
            }
            Self::SlotLengthDoesNotDivideHour { found } => write!(
                f,
                "slot length {found} minutes does not divide an hour; allowed lengths are 5, 10, 15, 20, 30, 60"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(hours: u16) -> SlotGrid {
        // 2026-10-09T00:00:00Z is epoch day 20471.
        SlotGrid::covering(civil_day_epoch::DAY_2026_10_09 * 1_440, 15, hours)
    }

    // Kept local so the tests do not depend on each other. Derived from the
    // date rather than hard-coded, so a wrong constant cannot silently shift a
    // test's meaning the way a stale 20471 (which is 2026-01-18, not 2026-10-09)
    // did.
    mod civil_day_epoch {
        pub const DAY_2026_10_09: i64 = crate::civil::days_from_civil(2026, 10, 9);
    }

    #[test]
    fn slot_lengths_that_divide_the_hour_are_accepted() {
        for len in [5u16, 10, 15, 20, 30, 60] {
            let g = SlotGrid::new(0, len, 4);
            assert!(g.is_valid(), "{len} minutes should be valid");
        }
    }

    #[test]
    fn odd_slot_lengths_are_rejected_because_they_truncate_energy() {
        let g = SlotGrid::new(0, 7, 4);
        assert_eq!(
            g.validate(),
            vec![GridFault::SlotLengthDoesNotDivideHour { found: 7 }]
        );
    }

    #[test]
    fn oversized_and_empty_grids_are_rejected() {
        assert_eq!(SlotGrid::new(0, 15, 0).validate(), vec![GridFault::Empty]);
        assert_eq!(
            SlotGrid::new(0, 15, MAX_SLOTS + 1).validate(),
            vec![GridFault::TooManySlots { found: MAX_SLOTS + 1, max: MAX_SLOTS }]
        );
    }

    #[test]
    fn slot_boundaries_are_exact_and_contiguous() {
        let g = grid(24);
        assert_eq!(g.slot_count, 96);
        assert_eq!(g.slot_start(0), civil_day_epoch::DAY_2026_10_09 * 1_440);
        assert_eq!(g.slot_end(0), g.slot_start(1));
        assert_eq!(g.slot_end(95), g.slot_start(0) + 1_440);
        assert_eq!(g.total_minutes(), 1_440);
    }

    #[test]
    fn index_at_resolves_and_rejects_out_of_range() {
        let g = grid(24);
        assert_eq!(g.index_at(g.slot_start(0)), Some(0));
        assert_eq!(g.index_at(g.slot_start(42)), Some(42));
        assert_eq!(g.index_at(g.slot_start(42) + 1), Some(42), "unaligned instant is contained");
        // Past the end of the grid. Computed arithmetically rather than with
        // `slot_start(96)`, which is itself out of range.
        assert_eq!(g.index_at(g.slot_start(0) + g.total_minutes()), None);
        assert_eq!(g.index_at(g.slot_start(0) - 1), None);
    }

    #[test]
    fn try_slot_start_is_safe_for_untrusted_indices() {
        let g = grid(24);
        assert_eq!(g.try_slot_start(95), Some(g.slot_start(95)));
        assert_eq!(g.try_slot_start(96), None);
        assert_eq!(g.try_slot_start(u32::MAX), None);
    }

    #[test]
    fn covers_epoch_is_half_open() {
        let g = grid(24);
        assert!(g.covers_epoch(g.slot_start(0)));
        assert!(!g.covers_epoch(g.slot_start(0) - 1));
        assert!(g.covers_epoch(g.slot_end(95) - 1));
        assert!(!g.covers_epoch(g.slot_start(0) + g.total_minutes()));
    }
}
