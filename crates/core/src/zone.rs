//! Local time zones with daylight-saving rules.
//!
//! Electricity tariffs are specified by *local wall-clock time* — "peak is
//! 4pm to 9pm" — but the settlement grid is an absolute UTC timeline. Bridging
//! the two is where nearly every real-world energy calculation goes wrong, so
//! it is modelled explicitly here rather than delegated.
//!
//! # Why conversion is one-directional
//!
//! The price engine only ever needs **UTC -> local wall clock**. Given an
//! instant, we ask "what time does the customer's clock read, and therefore
//! which rate period are they in?" That direction is always well defined.
//!
//! The inverse (local wall clock -> instant) is *ambiguous* on the autumn
//! fall-back day (01:30 happens twice) and *empty* on the spring-forward day
//! (02:30 never happens). We never need it, so the ambiguity never has to be
//! resolved by guessing.
//!
//! # Consequence for DST
//!
//! A "16:00-21:00" peak window is five hours long on a normal day, and still
//! five hours long on the spring-forward day — it simply starts an hour
//! earlier in absolute time. That falls out of the model for free, and
//! `tests/hour_of_dst.rs` pins it down.

use crate::civil::{CivilDate, WeekOfMonth};
use serde::{Deserialize, Serialize};

/// Offsets are minutes east of UTC (US Eastern is `-300`).
pub type OffsetMinutes = i16;

/// When a DST transition instant is defined in local time, *which* local time.
///
/// Jurisdictions genuinely differ here, and getting it wrong shifts the
/// transition by an hour. The United States defines spring-forward in local
/// **standard** time (02:00 EST) and fall-back in local **daylight** time
/// (02:00 EDT). The European Union defines both in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransitionConvention {
    /// Transition instant is `local_minutes` interpreted at the standard offset.
    LocalStandard,
    /// Transition instant is `local_minutes` interpreted at the daylight offset.
    LocalDaylight,
    /// Transition instant is a UTC time of day.
    Utc,
}

/// One annual DST transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Transition {
    pub month: u8,
    pub week: WeekOfMonth,
    /// 0 = Sunday .. 6 = Saturday
    pub weekday: u8,
    /// Time of day in the convention chosen by the owning [`DstRule`].
    pub time_of_day_minutes: u16,
}

impl Transition {
    /// The date this transition falls on in `year`.
    #[must_use]
    pub fn date_in(&self, year: i32) -> CivilDate {
        CivilDate::new(year, self.month, self.week.day_of(year, self.month, self.weekday))
    }
}

/// Daylight-saving rule for a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DstRule {
    pub start: Transition,
    pub end: Transition,
    /// Offset added while DST is in effect, almost always `+60`.
    pub extra_offset_minutes: OffsetMinutes,
    /// Convention for the spring-forward instant. The US writes this in local
    /// **standard** time ("2:00 AM local time", meaning EST).
    pub start_convention: TransitionConvention,
    /// Convention for the fall-back instant. The US writes this one in local
    /// **daylight** time, because at the moment the clocks go back daylight
    /// saving is still in force.
    ///
    /// Getting this wrong shifts the autumn transition by an hour, which is
    /// exactly the class of bug that makes a "cheapest window" schedule
    /// silently wrong for a week every autumn.
    pub end_convention: TransitionConvention,
}

impl DstRule {
    fn transition_epoch(
        &self,
        transition: &Transition,
        year: i32,
        standard_offset: OffsetMinutes,
        convention: TransitionConvention,
    ) -> i64 {
        let date = transition.date_in(year);
        let base = date.to_days() * 1_440;
        let utc_minutes = i64::from(transition.time_of_day_minutes);
        match convention {
            TransitionConvention::Utc => base + utc_minutes,
            TransitionConvention::LocalStandard => base + utc_minutes - i64::from(standard_offset),
            TransitionConvention::LocalDaylight => {
                base + utc_minutes - i64::from(standard_offset + self.extra_offset_minutes)
            }
        }
    }

    /// Epoch minute at which DST begins in `year`.
    fn start_epoch(&self, year: i32, standard_offset: OffsetMinutes) -> i64 {
        self.transition_epoch(&self.start, year, standard_offset, self.start_convention)
    }

    /// Epoch minute at which DST ends in `year`.
    fn end_epoch(&self, year: i32, standard_offset: OffsetMinutes) -> i64 {
        self.transition_epoch(&self.end, year, standard_offset, self.end_convention)
    }
}

/// A time zone, used to express tariffs in local wall-clock time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LocalZone {
    pub name: String,
    /// Standard-time offset, minutes east of UTC.
    pub standard_offset_minutes: OffsetMinutes,
    pub dst: Option<DstRule>,
}

impl LocalZone {
    /// UTC, no daylight saving. Useful for tests and for tariffs that are
    /// already expressed in UTC.
    pub const UTC: Self = Self {
        name: String::new(),
        standard_offset_minutes: 0,
        dst: None,
    };

    /// US Eastern.
    #[must_use]
    pub fn us_eastern() -> Self {
        Self {
            name: "America/New_York".into(),
            standard_offset_minutes: -300,
            dst: Some(DstRule {
                start: Transition { month: 3, week: WeekOfMonth::Second, weekday: 0, time_of_day_minutes: 2 * 60 },
                end: Transition { month: 11, week: WeekOfMonth::First, weekday: 0, time_of_day_minutes: 2 * 60 },
                extra_offset_minutes: 60,
                start_convention: TransitionConvention::LocalStandard,
                end_convention: TransitionConvention::LocalDaylight,
            }),
        }
    }

    /// US Pacific.
    #[must_use]
    pub fn us_pacific() -> Self {
        Self {
            name: "America/Los_Angeles".into(),
            standard_offset_minutes: -480,
            dst: Some(DstRule {
                start: Transition { month: 3, week: WeekOfMonth::Second, weekday: 0, time_of_day_minutes: 2 * 60 },
                end: Transition { month: 11, week: WeekOfMonth::First, weekday: 0, time_of_day_minutes: 2 * 60 },
                extra_offset_minutes: 60,
                start_convention: TransitionConvention::LocalStandard,
                end_convention: TransitionConvention::LocalDaylight,
            }),
        }
    }

    /// Central European Time (EU rule: transitions defined in UTC).
    #[must_use]
    pub fn eu_central() -> Self {
        Self {
            name: "Europe/Berlin".into(),
            standard_offset_minutes: 60,
            dst: Some(DstRule {
                start: Transition { month: 3, week: WeekOfMonth::Last, weekday: 0, time_of_day_minutes: 1 * 60 },
                end: Transition { month: 10, week: WeekOfMonth::Last, weekday: 0, time_of_day_minutes: 1 * 60 },
                extra_offset_minutes: 60,
                start_convention: TransitionConvention::Utc,
                end_convention: TransitionConvention::Utc,
            }),
        }
    }

    /// Is DST in effect at `epoch_minutes`?
    #[must_use]
    pub fn is_dst(&self, epoch_minutes: i64) -> bool {
        let Some(rule) = &self.dst else {
            return false;
        };
        // Local year, found by converting at the standard offset first. For any
        // zone whose DST window does not straddle New Year (all of them), this
        // is exact.
        let standard_local_days = (epoch_minutes + i64::from(self.standard_offset_minutes)).div_euclid(1_440);
        let year = CivilDate::from_days(standard_local_days).year;

        let start = rule.start_epoch(year, self.standard_offset_minutes);
        let end = rule.end_epoch(year, self.standard_offset_minutes);

        // Handle the (southern-hemisphere) case where the rule wraps the year
        // end: `end` falls before `start`.
        if end < start {
            epoch_minutes >= start || epoch_minutes < end
        } else {
            epoch_minutes >= start && epoch_minutes < end
        }
    }

    /// Total offset in effect at `epoch_minutes`, minutes east of UTC.
    #[must_use]
    pub fn offset_at(&self, epoch_minutes: i64) -> OffsetMinutes {
        match &self.dst {
            Some(rule) if self.is_dst(epoch_minutes) => {
                self.standard_offset_minutes + rule.extra_offset_minutes
            }
            _ => self.standard_offset_minutes,
        }
    }

    /// Convert an absolute instant to local wall-clock time.
    #[must_use]
    pub fn to_local(&self, epoch_minutes: i64) -> LocalWallClock {
        let offset = i64::from(self.offset_at(epoch_minutes));
        let local = epoch_minutes + offset;
        let days = local.div_euclid(1_440);
        let minutes_of_day = local.rem_euclid(1_440);
        LocalWallClock {
            date: CivilDate::from_days(days),
            minutes_of_day: minutes_of_day as u16,
            is_dst: self.is_dst(epoch_minutes),
        }
    }

    /// Convert a local date + time-of-day back to an instant.
    ///
    /// Returns `None` when the wall-clock time does not exist (the
    /// spring-forward gap). Ambiguous times resolve to the **earlier**
    /// instant, i.e. standard time. The price engine never calls this; it
    /// exists for tests and for expressing tariff boundary dates.
    #[must_use]
    pub fn from_local(&self, date: CivilDate, minutes_of_day: u16) -> Option<i64> {
        let base = date.to_days() * 1_440 + i64::from(minutes_of_day);
        let candidate = base - i64::from(self.standard_offset_minutes);
        if self.to_local(candidate).minutes_of_day == minutes_of_day
            && self.to_local(candidate).date == date
        {
            return Some(candidate);
        }
        // Ambiguous (two instants map to this wall clock) -> pick the earlier.
        let earlier = base
            - i64::from(self.standard_offset_minutes)
            - self
                .dst
                .map_or(0, |r| i64::from(r.extra_offset_minutes));
        if self.to_local(earlier).minutes_of_day == minutes_of_day
            && self.to_local(earlier).date == date
        {
            return Some(earlier);
        }
        None
    }
}

/// A local wall-clock reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalWallClock {
    pub date: CivilDate,
    /// Minutes since local midnight, 0..1440.
    pub minutes_of_day: u16,
    pub is_dst: bool,
}

impl LocalWallClock {
    #[must_use]
    pub fn hhmm(&self) -> String {
        format!("{:02}:{:02}", self.minutes_of_day / 60, self.minutes_of_day % 60)
    }
}

/// `String::new()` is not `const`, so provide an explicit constructor.
impl LocalZone {
    pub const fn utc() -> Self {
        Self { name: String::new(), standard_offset_minutes: 0, dst: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn epoch(y: i32, m: u8, d: u8, hh: u8, mm: u8, offset: OffsetMinutes) -> i64 {
        CivilDate::new(y, m, d).to_days() * 1_440 + i64::from(hh) * 60 + i64::from(mm)
            - i64::from(offset)
    }

    #[test]
    fn us_eastern_transitions_on_the_documented_days() {
        let z = LocalZone::us_eastern();
        // 2026: second Sunday in March = March 8, at 02:00 local standard.
        let just_before = epoch(2026, 3, 8, 1, 59, -300);
        let just_after = epoch(2026, 3, 8, 3, 0, -300);
        assert!(!z.is_dst(just_before), "01:59 local standard is still EST");
        assert_eq!(z.offset_at(just_before), -300);
        assert!(z.is_dst(just_after), "03:00 local is EDT");
        assert_eq!(z.offset_at(just_after), -240);

        // 2026: first Sunday in November = November 1, at 02:00 local daylight.
        let before_end = epoch(2026, 11, 1, 1, 59, -240);
        let after_end = epoch(2026, 11, 1, 1, 0, -300);
        assert!(z.is_dst(before_end));
        assert!(!z.is_dst(after_end), "01:00 standard after fall-back is EST");
        assert_eq!(z.offset_at(after_end), -300);
    }

    #[test]
    fn eu_transitions_are_defined_in_utc() {
        let z = LocalZone::eu_central();
        // 2026: last Sunday in March = March 29, 01:00 UTC.
        let before = epoch(2026, 3, 29, 0, 59, 0);
        let after = epoch(2026, 3, 29, 1, 0, 0);
        assert_eq!(z.offset_at(before), 60);
        assert_eq!(z.offset_at(after), 120);

        // 2026: last Sunday in October = October 25, 01:00 UTC.
        let before = epoch(2026, 10, 25, 0, 59, 0);
        let after = epoch(2026, 10, 25, 1, 0, 0);
        assert_eq!(z.offset_at(before), 120);
        assert_eq!(z.offset_at(after), 60);
    }

    #[test]
    fn wall_clock_jumps_forward_in_the_spring_gap() {
        let z = LocalZone::us_eastern();
        // 07:00 UTC on the spring-forward day is 01:00 EST; 08:00 UTC is 04:00 EDT.
        // DST starts at exactly 07:00 UTC on 2026-03-08 (02:00 local standard),
        // so 06:59 UTC reads 01:59 EST and 07:00 UTC reads 03:00 EDT.
        let t1 = epoch(2026, 3, 8, 6, 59, 0);
        let t2 = epoch(2026, 3, 8, 7, 0, 0);
        assert_eq!(z.to_local(t1).hhmm(), "01:59");
        assert_eq!(z.to_local(t2).hhmm(), "03:00");
        // The hour between them never appears on any local clock.
        assert_eq!(z.to_local(t2).hhmm(), "03:00", "02:xx local does not exist");
        for m in 0..60 {
            let reading = z.to_local(t2 - 60 + m).hhmm();
            assert!(!reading.starts_with("02:"), "02:{m:02} should not exist, got {reading}");
        }
    }

    #[test]
    fn wall_clock_repeats_an_hour_in_the_autumn_fold() {
        let z = LocalZone::us_eastern();
        // 05:30 UTC and 06:30 UTC are both 01:30 local on the fall-back day.
        // 05:30 UTC is 01:30 EDT (still saving); 06:30 UTC is 01:30 EST.
        let a = epoch(2026, 11, 1, 5, 30, 0);
        let b = epoch(2026, 11, 1, 6, 30, 0);
        assert_eq!(z.to_local(a).hhmm(), "01:30");
        assert_eq!(z.to_local(b).hhmm(), "01:30");
        assert!(z.to_local(a).is_dst, "05:30 UTC is still EDT");
        assert!(!z.to_local(b).is_dst, "06:30 UTC has fallen back to EST");
    }

    #[test]
    fn from_local_reports_the_gap_as_nonexistent() {
        let z = LocalZone::us_eastern();
        // 02:30 local on 2026-03-08 does not exist.
        assert!(z.from_local(CivilDate::new(2026, 3, 8), 2 * 60 + 30).is_none());
        // A normal time resolves fine.
        assert!(z.from_local(CivilDate::new(2026, 6, 1), 12 * 60).is_some());
    }

    #[test]
    fn pacific_and_utc_behave() {
        let p = LocalZone::us_pacific();
        assert_eq!(p.offset_at(epoch(2026, 1, 15, 0, 0, 0)), -480);
        assert_eq!(p.offset_at(epoch(2026, 7, 15, 0, 0, 0)), -420);
        let u = LocalZone::utc();
        assert_eq!(u.offset_at(0), 0);
        assert!(!u.is_dst(0));
    }
}
