//! UTC timestamps without the chrono dependency.
//!
//! [`UtcTime`] is nanoseconds since the Unix epoch with the wire
//! format chrono's serde impl used: RFC 3339, `Z` suffix, fractional
//! digits only when nonzero. Logs written by older kache versions
//! parse unchanged. Arithmetic saturates instead of overflowing;
//! every saturation site previously relied on chrono's checked or
//! saturating ops the same way.

use std::ops::{Add, Sub};
use std::time::Duration;

/// An instant in UTC, nanoseconds since 1970-01-01T00:00:00Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTime {
    nanos: i128,
}

/// A calendar day, days since 1970-01-01, proleptic Gregorian.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    days: i32,
}

/// Broken-down UTC fields for display. Only [`UtcTime`] builds one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    nanos: u32,
}

const NANOS_PER_SEC: i128 = 1_000_000_000;
const SECS_PER_DAY: i64 = 86_400;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl UtcTime {
    /// Earliest representable instant. Saturation target for unbounded
    /// lookbacks, the same role chrono's `MIN_UTC` played.
    pub const MIN: UtcTime = UtcTime { nanos: i128::MIN };

    /// Current wall-clock time.
    pub fn now() -> UtcTime {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        UtcTime {
            nanos: elapsed.as_secs() as i128 * NANOS_PER_SEC + elapsed.subsec_nanos() as i128,
        }
    }

    /// Seconds plus a nanosecond offset, like chrono's `from_timestamp`.
    /// `None` for a nanosecond offset past two seconds; whole extra
    /// seconds carry into the timestamp.
    pub fn from_timestamp(secs: i64, nanos: u32) -> Option<UtcTime> {
        if nanos >= 2_000_000_000 {
            return None;
        }
        let carry = (nanos / 1_000_000_000) as i64;
        let nanos = (nanos % 1_000_000_000) as i128;
        Some(UtcTime {
            nanos: secs as i128 * NANOS_PER_SEC + carry as i128 * NANOS_PER_SEC + nanos,
        })
    }

    /// Milliseconds since the epoch. Always representable.
    pub fn from_timestamp_millis(millis: i64) -> Option<UtcTime> {
        Some(UtcTime {
            nanos: millis as i128 * 1_000_000,
        })
    }

    /// Whole seconds since the epoch.
    pub fn from_unix_secs(secs: i64) -> UtcTime {
        UtcTime {
            nanos: secs as i128 * NANOS_PER_SEC,
        }
    }

    /// Calendar construction for tests and fixed fixtures. `None` for an
    /// out-of-range month, day, hour, minute or second.
    pub fn from_ymd_hms(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Option<UtcTime> {
        if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 60 {
            return None;
        }
        if day == 0 || day > days_in_month(year, month) {
            return None;
        }
        let days = days_from_civil(year as i64, month as i64, day as i64);
        let secs = days
            .checked_mul(SECS_PER_DAY)?
            .checked_add(hour as i64 * 3600 + minute as i64 * 60 + second as i64)?;
        Some(UtcTime {
            nanos: secs as i128 * NANOS_PER_SEC,
        })
    }

    /// Whole seconds since the epoch, saturating at the i64 bounds.
    pub fn timestamp(&self) -> i64 {
        (self.nanos / NANOS_PER_SEC).clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    /// Milliseconds since the epoch, saturating at the i64 bounds.
    pub fn timestamp_millis(&self) -> i64 {
        (self.nanos / 1_000_000).clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    /// Microseconds since the epoch, saturating at the i64 bounds.
    pub fn timestamp_micros(&self) -> i64 {
        (self.nanos / 1_000).clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    /// Nanoseconds since the epoch. `None` past the i64 range.
    pub fn timestamp_nanos_opt(&self) -> Option<i64> {
        i64::try_from(self.nanos).ok()
    }

    /// Subtract a span, saturating at [`UtcTime::MIN`] instead of wrapping.
    pub fn saturating_sub_duration(&self, span: Duration) -> UtcTime {
        UtcTime {
            nanos: self.nanos.saturating_sub(span.as_nanos() as i128),
        }
    }

    /// Add a span, saturating at the i128 bound instead of wrapping.
    pub fn saturating_add_duration(&self, span: Duration) -> UtcTime {
        UtcTime {
            nanos: self.nanos.saturating_add(span.as_nanos() as i128),
        }
    }

    /// Subtract a span. `None` past [`UtcTime::MIN`].
    pub fn checked_sub_duration(&self, span: Duration) -> Option<UtcTime> {
        self.nanos
            .checked_sub(span.as_nanos() as i128)
            .map(|nanos| UtcTime { nanos })
    }

    /// `self - earlier`, floored at zero. Every caller previously clamped
    /// chrono's signed result the same way before comparing or printing.
    pub fn saturating_duration_since(&self, earlier: UtcTime) -> Duration {
        Duration::from_nanos(self.nanos.saturating_sub(earlier.nanos).max(0) as u64)
    }

    /// Broken-down UTC fields. Instants outside the four-digit-year
    /// range clamp to it; formatting stays total.
    pub fn parts(&self) -> Parts {
        let clamped = self.nanos.clamp(civil_nanos_range().0, civil_nanos_range().1);
        let mut secs = clamped.div_euclid(NANOS_PER_SEC);
        let nanos = clamped.rem_euclid(NANOS_PER_SEC) as u32;
        let days = secs.div_euclid(SECS_PER_DAY as i128);
        secs -= days * SECS_PER_DAY as i128;
        let (year, month, day) = civil_from_days(days);
        Parts {
            year,
            month,
            day,
            hour: (secs / 3600) as u8,
            minute: ((secs % 3600) / 60) as u8,
            second: (secs % 60) as u8,
            nanos,
        }
    }

    /// UTC calendar day.
    pub fn date(&self) -> CivilDate {
        let days = self.nanos.div_euclid(NANOS_PER_SEC * SECS_PER_DAY as i128);
        CivilDate {
            days: days.clamp(i32::MIN as i128, i32::MAX as i128) as i32,
        }
    }

    /// RFC 3339 in chrono's display shape: `+00:00` suffix, fractional
    /// digits only when nonzero, millisecond/microsecond width when
    /// exact. What human reports and JSON envelopes print.
    pub fn to_rfc3339(&self) -> String {
        format!("{}+00:00", self.date_time_part())
    }

    /// RFC 3339 in chrono's serde shape: `Z` suffix. What the event
    /// log and other persisted records carry.
    fn to_rfc3339_z(self) -> String {
        format!("{}Z", self.date_time_part())
    }

    fn date_time_part(&self) -> String {
        let parts = self.parts();
        let mut out = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            parts.year, parts.month, parts.day, parts.hour, parts.minute, parts.second
        );
        if parts.nanos != 0 {
            if parts.nanos.is_multiple_of(1_000_000) {
                out.push_str(&format!(".{:03}", parts.nanos / 1_000_000));
            } else if parts.nanos.is_multiple_of(1_000) {
                out.push_str(&format!(".{:06}", parts.nanos / 1_000));
            } else {
                out.push_str(&format!(".{:09}", parts.nanos));
            }
        }
        out
    }

    /// Parse RFC 3339 with a `Z` or numeric offset and optional fraction.
    /// Accepts what older kache versions wrote; `None` otherwise.
    pub fn parse_rfc3339(value: &str) -> Option<UtcTime> {
        let (date, time) = value.split_once(['T', 't', ' '])?;
        let mut fields = date.split('-');
        let year: i32 = fields.next()?.parse().ok()?;
        let month: u8 = fields.next()?.parse().ok()?;
        let day: u8 = fields.next()?.parse().ok()?;
        if fields.next().is_some() || date.len() < 10 {
            return None;
        }
        let (clock, offset) = split_offset(time)?;
        let mut clock = clock.split(':');
        let hour: u8 = clock.next()?.parse().ok()?;
        let minute: u8 = clock.next()?.parse().ok()?;
        let (second, nanos) = split_fraction(clock.next()?)?;
        if clock.next().is_some() {
            return None;
        }
        let mut moment = UtcTime::from_ymd_hms(year, month, day, hour, minute, second)?;
        moment.nanos += nanos as i128;
        Some(UtcTime {
            nanos: moment.nanos - offset as i128 * NANOS_PER_SEC,
        })
    }

    /// Format a UTC instant as a SQLite/naive `YYYY-MM-DD HH:MM:SS` stamp.
    pub fn format_sqlite(&self) -> String {
        let parts = self.parts();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            parts.year, parts.month, parts.day, parts.hour, parts.minute, parts.second
        )
    }

    /// Parse a SQLite/naive `YYYY-MM-DD HH:MM:SS` stamp as UTC.
    pub fn parse_sqlite(value: &str) -> Option<UtcTime> {
        let (date, time) = value.split_once(' ')?;
        let mut fields = date.split('-');
        let year: i32 = fields.next()?.parse().ok()?;
        let month: u8 = fields.next()?.parse().ok()?;
        let day: u8 = fields.next()?.parse().ok()?;
        if fields.next().is_some() {
            return None;
        }
        let mut clock = time.split(':');
        let hour: u8 = clock.next()?.parse().ok()?;
        let minute: u8 = clock.next()?.parse().ok()?;
        let second: u8 = clock.next()?.parse().ok()?;
        if clock.next().is_some() {
            return None;
        }
        UtcTime::from_ymd_hms(year, month, day, hour, minute, second)
    }
}

impl Add<Duration> for UtcTime {
    type Output = UtcTime;

    fn add(self, span: Duration) -> UtcTime {
        self.saturating_add_duration(span)
    }
}

impl Sub<Duration> for UtcTime {
    type Output = UtcTime;

    fn sub(self, span: Duration) -> UtcTime {
        self.saturating_sub_duration(span)
    }
}

impl std::fmt::Display for UtcTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl deser::Serialize for UtcTime {
    fn serialize<'a>(
        value: &'a Self,
        _state: &mut deser::State,
    ) -> Result<deser::ser::Emit<'a>, deser::Error> {
        Ok(deser::ser::Emit::Atom(deser::Atom::Str(
            value.to_rfc3339_z().into(),
        )))
    }
}

impl<'de> deser::Deserialize<'de> for UtcTime {
    fn deserialize_atom(
        slot: &mut deser::de::Slot<Self>,
        atom: deser::Atom,
        state: &mut deser::State,
    ) -> Result<(), deser::Error> {
        match atom {
            deser::Atom::Str(text) => {
                let time = UtcTime::parse_rfc3339(text.as_str()).ok_or_else(|| {
                    deser::Error::new(deser::ErrorKind::InvalidValue, "invalid RFC 3339 timestamp")
                })?;
                slot.set(time);
                Ok(())
            }
            other => deser::de::default_atom(slot, other, state),
        }
    }
}

fn split_offset(time: &str) -> Option<(&str, i32)> {
    if let Some(clock) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
        return Some((clock, 0));
    }
    let at = time.find(['+', '-'])?;
    let (clock, zone) = time.split_at(at);
    let (sign, digits) = match zone.as_bytes()[0] {
        b'+' => (1i32, &zone[1..]),
        _ => (-1i32, &zone[1..]),
    };
    let (hours, minutes) = match digits.len() {
        2 => (digits.parse::<i32>().ok()?, 0),
        4 => (
            digits[..2].parse::<i32>().ok()?,
            digits[2..].parse::<i32>().ok()?,
        ),
        5 if digits.as_bytes()[2] == b':' => (
            digits[..2].parse::<i32>().ok()?,
            digits[3..].parse::<i32>().ok()?,
        ),
        _ => return None,
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some((clock, sign * (hours * 3600 + minutes * 60)))
}

fn split_fraction(second: &str) -> Option<(u8, u32)> {
    match second.split_once('.') {
        Some((whole, fraction)) => {
            let value: u8 = whole.parse().ok()?;
            if fraction.is_empty()
                || fraction.len() > 9
                || !fraction.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            let mut nanos = fraction.parse::<u32>().ok()?;
            for _ in fraction.len()..9 {
                nanos *= 10;
            }
            Some((value, nanos))
        }
        None => Some((second.parse().ok()?, 0)),
    }
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted = if month <= 2 { year - 1 } else { year };
    let era = if adjusted >= 0 { adjusted } else { adjusted - 399 } / 400;
    let year_of_era = adjusted - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
}

fn civil_from_days(days: i128) -> (i32, u8, u8) {
    let shifted = days + 719468;
    let era = if shifted >= 0 { shifted } else { shifted - 146096 } / 146097;
    let day_of_era = (shifted - era * 146097) as i64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let year = year_of_era + era as i64 * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u8;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u8;
    (
        if month <= 2 { year + 1 } else { year } as i32,
        month,
        day,
    )
}

fn civil_nanos_range() -> (i128, i128) {
    (
        days_from_civil(-9999, 1, 1) as i128 * SECS_PER_DAY as i128 * NANOS_PER_SEC,
        days_from_civil(9999, 12, 31) as i128 * SECS_PER_DAY as i128 * NANOS_PER_SEC
            + (NANOS_PER_SEC - 1),
    )
}

impl CivilDate {
    /// Gregorian year.
    pub fn year(&self) -> i32 {
        civil_from_days(self.days as i128).0
    }

    /// `Sep 12`, with the year when it is not `today`'s.
    pub fn label_since(&self, today: CivilDate) -> String {
        let (year, month, day) = civil_from_days(self.days as i128);
        if year == today.year() {
            format!("{} {day:02}", MONTHS[(month - 1) as usize])
        } else {
            format!("{} {day:02} {year}", MONTHS[(month - 1) as usize])
        }
    }
}

/// Format a UTC instant: `2026-10-14 15:18 UTC`.
pub fn format_utc_ymd_hm(moment: UtcTime) -> String {
    let parts = moment.parts();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        parts.year, parts.month, parts.day, parts.hour, parts.minute
    )
}

/// Format a UTC instant: `20261014T151856Z`.
pub fn format_compact_utc(moment: UtcTime) -> String {
    let parts = moment.parts();
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        parts.year, parts.month, parts.day, parts.hour, parts.minute, parts.second
    )
}

/// Broken-down fields `offset_secs` east of UTC: a local zone applied
/// by hand, so tests pin rendering without depending on the host zone.
pub fn shifted_parts(moment: UtcTime, offset_secs: i32) -> Parts {
    let shift = offset_secs as i128 * NANOS_PER_SEC;
    UtcTime {
        nanos: moment.nanos.saturating_add(shift),
    }
    .parts()
}

/// Broken-down fields in this machine's zone.
pub fn local_parts(moment: UtcTime) -> Parts {
    shifted_parts(moment, local_offset_seconds(moment))
}

/// This machine's calendar day.
pub fn today_local() -> CivilDate {
    local_parts(UtcTime::now()).date()
}

/// Calendar day of `moment` in this machine's zone.
pub fn local_date(moment: UtcTime) -> CivilDate {
    local_parts(moment).date()
}

impl Parts {
    fn date(&self) -> CivilDate {
        CivilDate {
            days: days_from_civil(self.year as i64, self.month as i64, self.day as i64)
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        }
    }

    /// `17:18:56`.
    pub fn clock(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    /// `2026-10-14 17:18:56`.
    pub fn datetime(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// `10-14 17:18`.
    pub fn month_day_clock(&self) -> String {
        format!(
            "{:02}-{:02} {:02}:{:02}",
            self.month, self.day, self.hour, self.minute
        )
    }

    /// `Oct 14 17:18`.
    pub fn month_name_day_clock(&self) -> String {
        format!(
            "{} {:02} {:02}:{:02}",
            MONTHS[(self.month - 1) as usize],
            self.day,
            self.hour,
            self.minute
        )
    }
}

/// Seconds east of UTC at `moment` for this machine's zone, via the OS.
fn local_offset_seconds(moment: UtcTime) -> i32 {
    imp::local_offset_seconds(moment)
}

#[cfg(unix)]
mod imp {
    pub fn local_offset_seconds(moment: super::UtcTime) -> i32 {
        // Safety: both pointers name live stack slots; localtime_r
        // writes only `zone` and reads only `stamp`. The cast target is
        // inferred from `localtime_r` so this never names `libc::time_t`,
        // which is deprecated on some musl targets.
        unsafe {
            let stamp = moment.timestamp() as _;
            let mut zone: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&stamp, &mut zone).is_null() {
                return 0;
            }
            zone.tm_gmtoff.clamp(i32::MIN as libc::c_long, i32::MAX as libc::c_long) as i32
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    #[repr(C)]
    struct SystemTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SystemTimeToTzSpecificLocalTime(
            zone: *const c_void,
            utc: *const SystemTime,
            local: *mut SystemTime,
        ) -> i32;
    }

    pub fn local_offset_seconds(moment: super::UtcTime) -> i32 {
        let parts = moment.parts();
        let utc = SystemTime {
            year: parts.year as u16,
            month: parts.month as u16,
            weekday: 0,
            day: parts.day as u16,
            hour: parts.hour as u16,
            minute: parts.minute as u16,
            second: parts.second as u16,
            milliseconds: 0,
        };
        // Safety: both pointers name live stack slots; a null zone
        // selects the machine's current zone with its DST rules for
        // the given instant. Zero return keeps UTC.
        unsafe {
            let mut local: SystemTime = std::mem::zeroed();
            if SystemTimeToTzSpecificLocalTime(
                std::ptr::null(),
                &utc,
                &mut local,
            ) == 0
            {
                return 0;
            }
            let days = super::days_from_civil(local.year as i64, local.month as i64, local.day as i64);
            let local_secs = days * 86_400
                + local.hour as i64 * 3600
                + local.minute as i64 * 60
                + local.second as i64;
            (local_secs - moment.timestamp())
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    pub fn local_offset_seconds(_moment: super::UtcTime) -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trip_matches_chrono_shapes() {
        for (moment, display, wire) in [
            (
                UtcTime::from_unix_secs(0),
                "1970-01-01T00:00:00+00:00",
                "1970-01-01T00:00:00Z",
            ),
            (
                UtcTime::from_ymd_hms(2026, 9, 30, 12, 0, 0).unwrap(),
                "2026-09-30T12:00:00+00:00",
                "2026-09-30T12:00:00Z",
            ),
            (
                UtcTime::from_timestamp(1_700_000_000, 500_000_000).unwrap(),
                "2023-11-14T22:13:20.500+00:00",
                "2023-11-14T22:13:20.500Z",
            ),
            (
                UtcTime::from_timestamp(1_700_000_000, 500_000).unwrap(),
                "2023-11-14T22:13:20.000500+00:00",
                "2023-11-14T22:13:20.000500Z",
            ),
            (
                UtcTime::from_timestamp(1_700_000_000, 500).unwrap(),
                "2023-11-14T22:13:20.000000500+00:00",
                "2023-11-14T22:13:20.000000500Z",
            ),
        ] {
            assert_eq!(moment.to_rfc3339(), display);
            assert_eq!(moment.to_rfc3339_z(), wire);
            assert_eq!(UtcTime::parse_rfc3339(display), Some(moment));
            assert_eq!(UtcTime::parse_rfc3339(wire), Some(moment));
        }
    }

    #[test]
    fn rfc3339_accepts_offsets() {
        let utc = UtcTime::from_ymd_hms(2026, 9, 12, 12, 11, 5).unwrap();
        assert_eq!(UtcTime::parse_rfc3339("2026-09-12T12:11:05+00:00"), Some(utc));
        assert_eq!(UtcTime::parse_rfc3339("2026-09-12T14:11:05+0200"), Some(utc));
        assert_eq!(UtcTime::parse_rfc3339("2026-09-12T14:11:05+02"), Some(utc));
        assert_eq!(UtcTime::parse_rfc3339("2026-09-12T07:11:05-05:00"), Some(utc));
        for bad in [
            "",
            "2026-09-12",
            "2026-09-12T12:11:05",
            "2026-13-01T00:00:00Z",
            "2026-09-31T00:00:00Z",
            "2026-09-12T24:00:00Z",
            "2026-09-12T12:11:05+25:00",
            "not a time",
        ] {
            assert_eq!(UtcTime::parse_rfc3339(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn civil_dates_round_trip() {
        for (days, year, month, day) in [
            (0i64, 1970i32, 1u8, 1u8),
            (-1, 1969, 12, 31),
            (739_136, 3993, 9, 8),
            (20_454, 2026, 1, 1),
            (20_708, 2026, 9, 12),
        ] {
            let (y, mo, d) = civil_from_days(days as i128);
            assert_eq!((y, mo, d), (year, month, day));
            assert_eq!(days_from_civil(year as i64, month as i64, day as i64), days);
        }
        assert_eq!(UtcTime::from_unix_secs(0).date(), CivilDate { days: 0 });
        assert!(UtcTime::from_ymd_hms(2026, 13, 1, 0, 0, 0).is_none());
        assert!(UtcTime::from_ymd_hms(2026, 2, 29, 0, 0, 0).is_none());
        assert!(UtcTime::from_ymd_hms(2024, 2, 29, 0, 0, 0).is_some());
    }

    #[test]
    fn arithmetic_saturates() {
        let moment = UtcTime::from_unix_secs(1_700_000_000);
        assert_eq!(
            moment.saturating_sub_duration(Duration::from_secs(100)).timestamp(),
            1_699_999_900
        );
        assert_eq!(
            moment.saturating_duration_since(UtcTime::from_unix_secs(1_699_999_900)),
            Duration::from_secs(100)
        );
        assert_eq!(
            UtcTime::from_unix_secs(100).saturating_duration_since(moment),
            Duration::ZERO
        );
        assert_eq!(
            UtcTime::MIN.saturating_sub_duration(Duration::from_secs(1)),
            UtcTime::MIN
        );
        assert_eq!((moment + Duration::from_secs(60)).timestamp(), 1_700_000_060);
        assert_eq!((moment - Duration::from_secs(60)).timestamp(), 1_699_999_940);
    }

    #[test]
    fn since_labels_show_the_year_only_across_years() {
        let today = CivilDate { days: 20_708 };
        assert_eq!(
            CivilDate { days: 20_700 }.label_since(today),
            "Sep 04"
        );
        assert_eq!(
            CivilDate { days: 20_454 }.label_since(today),
            "Jan 01"
        );
        assert_eq!(
            CivilDate { days: 0 }.label_since(today),
            "Jan 01 1970"
        );
    }
}
