//! A time axis for [`LineChart`](super::LineChart): ticks on whole local
//! hours, days, Mondays or months, the finest that fits, and the long
//! form of a moment for the tooltip (Forge's `timeTicks` and `timeLong`).
//!
//! Times are milliseconds since the Unix epoch. "Local" is UTC moved by a
//! fixed offset the caller gives, as an app reads it once at start; there
//! are no time zone rules here, so ticks don't move across a DST change.

const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 86_400_000.0;

/// What a tick steps by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Hours(u32),
    Days(u32),
    Weeks(u32),
    Months(u32),
}

/// The units, finest first, with roughly how long each is.
const UNITS: [(f64, Unit); 11] = [
    (HOUR, Unit::Hours(1)),
    (3.0 * HOUR, Unit::Hours(3)),
    (6.0 * HOUR, Unit::Hours(6)),
    (12.0 * HOUR, Unit::Hours(12)),
    (DAY, Unit::Days(1)),
    (2.0 * DAY, Unit::Days(2)),
    (7.0 * DAY, Unit::Weeks(1)),
    (14.0 * DAY, Unit::Weeks(2)),
    (30.4 * DAY, Unit::Months(1)),
    (91.0 * DAY, Unit::Months(3)),
    (365.0 * DAY, Unit::Months(12)),
];

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// A tick on a time axis.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TimeTick {
    /// When, in ms since the epoch.
    pub at: f64,
    /// `14:00`, `Oct 8` (a day, or an hour tick at midnight), `Oct` (a
    /// month), or `2027` (January).
    pub label: String,
}

/// A local date and time of day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Civil {
    year: i64,
    /// 1 to 12.
    month: u32,
    /// 1 to 31.
    day: u32,
    hour: u32,
    minute: u32,
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `days` after 1970-01-01 (Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `at` (ms since the epoch) in local time, `offset` seconds east of UTC.
fn civil(at: f64, offset: i32) -> Civil {
    let local = (at / 1_000.0).floor() as i64 + i64::from(offset);
    let days = local.div_euclid(86_400);
    let seconds = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Civil {
        year,
        month,
        day,
        hour: (seconds / 3_600) as u32,
        minute: (seconds % 3_600 / 60) as u32,
    }
}

/// The moment local `civil` is, in ms since the epoch. Days and hours past
/// their range carry over (day 32 of a month is the 1st of the next).
fn moment(civil: Civil, offset: i32) -> f64 {
    let days = days_from_civil(civil.year, civil.month, 1) + i64::from(civil.day) - 1;
    let seconds = days * 86_400 + i64::from(civil.hour) * 3_600 + i64::from(civil.minute) * 60
        - i64::from(offset);
    seconds as f64 * 1_000.0
}

/// 0 for Sunday.
fn weekday(civil: Civil) -> usize {
    // 1970-01-01 was a Thursday.
    (days_from_civil(civil.year, civil.month, civil.day) + 4).rem_euclid(7) as usize
}

fn month_name(month: u32) -> &'static str {
    MONTHS[(month as usize).clamp(1, 12) - 1]
}

/// `civil` moved on by one `unit`; days and hours carry into the calendar.
fn step(civil: Civil, unit: Unit) -> Civil {
    let normal = |days: i64, hour: u32| {
        let (year, month, day) = civil_from_days(days + i64::from(hour / 24));
        Civil {
            year,
            month,
            day,
            hour: hour % 24,
            minute: 0,
        }
    };
    let days = days_from_civil(civil.year, civil.month, civil.day);
    match unit {
        Unit::Hours(k) => normal(days, civil.hour + k),
        Unit::Days(k) => normal(days + i64::from(k), civil.hour),
        Unit::Weeks(k) => normal(days + 7 * i64::from(k), civil.hour),
        Unit::Months(k) => {
            let months = civil.year * 12 + i64::from(civil.month) - 1 + i64::from(k);
            Civil {
                year: months.div_euclid(12),
                month: months.rem_euclid(12) as u32 + 1,
                ..civil
            }
        }
    }
}

/// The first tick of `unit` at or after local `start`'s calendar boundary:
/// a whole hour divisible by the step, a midnight, a Monday, or the first of
/// a month divisible by the step (January for a year).
fn align(start: Civil, unit: Unit) -> Civil {
    match unit {
        Unit::Hours(k) => {
            let mut at = Civil { minute: 0, ..start };
            while !at.hour.is_multiple_of(k) {
                at = step(at, Unit::Hours(1));
            }
            at
        }
        Unit::Days(_) => Civil {
            hour: 0,
            minute: 0,
            ..start
        },
        Unit::Weeks(_) => {
            let midnight = Civil {
                hour: 0,
                minute: 0,
                ..start
            };
            let back = (weekday(midnight) + 6) % 7;
            let days = days_from_civil(midnight.year, midnight.month, midnight.day) - back as i64;
            let (year, month, day) = civil_from_days(days);
            Civil {
                year,
                month,
                day,
                ..midnight
            }
        }
        Unit::Months(k) => {
            let mut at = Civil {
                day: 1,
                hour: 0,
                minute: 0,
                ..start
            };
            while !(at.month - 1).is_multiple_of(k) {
                at = step(at, Unit::Months(1));
            }
            at
        }
    }
}

fn label(at: Civil, unit: Unit) -> String {
    let date = || format!("{} {}", month_name(at.month), at.day);
    match unit {
        Unit::Hours(_) if at.hour != 0 => format!("{:02}:00", at.hour),
        Unit::Months(_) if at.month != 1 => month_name(at.month).to_owned(),
        Unit::Months(_) => at.year.to_string(),
        _ => date(),
    }
}

/// Ticks from `from` to `to` (ms since the epoch) on the finest unit that
/// gives at most `most` of them, local time `offset` seconds east of UTC.
/// Past a tick a year, years are skipped so there are still at most `most`.
pub(crate) fn time_ticks(from: f64, to: f64, most: usize, offset: i32) -> Vec<TimeTick> {
    if !(from.is_finite() && to.is_finite()) || to < from || most == 0 {
        return Vec::new();
    }
    let span = to - from;
    let unit = UNITS
        .iter()
        .find(|(length, _)| span / length <= most as f64)
        .map_or_else(
            || {
                let years = (span / (365.0 * DAY) / most as f64).ceil() as u32;
                Unit::Months(12 * years.max(1))
            },
            |&(_, unit)| unit,
        );
    let mut at = align(civil(from, offset), unit);
    while moment(at, offset) < from {
        at = step(at, unit);
    }
    let mut ticks = Vec::new();
    while moment(at, offset) <= to {
        ticks.push(TimeTick {
            at: moment(at, offset),
            label: label(at, unit),
        });
        at = step(at, unit);
    }
    ticks
}

/// `at` written out for a tooltip: `Wed, Oct 8, 14:02` on an axis spanning
/// under two weeks, else `Wed, Oct 8, 2026`.
pub(crate) fn time_long(at: f64, span: f64, offset: i32) -> String {
    let civil = civil(at, offset);
    let date = format!(
        "{}, {} {}",
        WEEKDAYS[weekday(civil)],
        month_name(civil.month),
        civil.day
    );
    if span < 14.0 * DAY {
        format!("{date}, {:02}:{:02}", civil.hour, civil.minute)
    } else {
        format!("{date}, {}", civil.year)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ms since the epoch of a UTC date and time.
    fn utc(year: i64, month: u32, day: u32, hour: u32, minute: u32) -> f64 {
        moment(
            Civil {
                year,
                month,
                day,
                hour,
                minute,
            },
            0,
        )
    }

    fn labels(ticks: &[TimeTick]) -> Vec<&str> {
        ticks.iter().map(|t| t.label.as_str()).collect()
    }

    #[test]
    fn dates_round_trip_through_days_since_the_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        for days in [-800_000, -1, 0, 59, 10_957, 20_000, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        // 2026-10-08 is a Thursday.
        assert_eq!(weekday(civil(utc(2026, 10, 8, 12, 0), 0)), 4);
    }

    #[test]
    fn a_day_ticks_every_few_hours_and_names_midnight_by_its_date() {
        let ticks = time_ticks(utc(2026, 10, 7, 13, 20), utc(2026, 10, 8, 13, 0), 6, 0);
        // Under a day at six labels: every six hours, from 18:00.
        assert_eq!(labels(&ticks), ["18:00", "Oct 8", "06:00", "12:00"]);
        assert_eq!(ticks[1].at, utc(2026, 10, 8, 0, 0));
    }

    #[test]
    fn weeks_start_on_mondays() {
        // Thu 1 Oct 2026 to Sat 31 Oct: four Mondays.
        let ticks = time_ticks(utc(2026, 10, 1, 0, 0), utc(2026, 10, 31, 0, 0), 5, 0);
        assert_eq!(labels(&ticks), ["Oct 5", "Oct 12", "Oct 19", "Oct 26"]);
    }

    #[test]
    fn months_name_january_by_its_year() {
        let ticks = time_ticks(utc(2026, 8, 15, 0, 0), utc(2027, 3, 2, 0, 0), 8, 0);
        assert_eq!(
            labels(&ticks),
            ["Sep", "Oct", "Nov", "Dec", "2027", "Feb", "Mar"]
        );
        let quarters = time_ticks(utc(2026, 2, 1, 0, 0), utc(2027, 2, 1, 0, 0), 5, 0);
        assert_eq!(labels(&quarters), ["Apr", "Jul", "Oct", "2027"]);
    }

    #[test]
    fn ticks_fall_on_local_hours_east_or_west_of_utc() {
        // Tokyo, nine hours east: 15:00 UTC on Oct 7 is local midnight.
        let tokyo = 9 * 3_600;
        let ticks = time_ticks(utc(2026, 10, 7, 14, 0), utc(2026, 10, 7, 20, 0), 6, tokyo);
        assert_eq!(
            labels(&ticks),
            [
                "23:00", "Oct 8", "01:00", "02:00", "03:00", "04:00", "05:00"
            ]
        );
        assert_eq!(ticks[1].at, utc(2026, 10, 7, 15, 0));
        // Half an hour west: whole local hours are half past in UTC.
        let ticks = time_ticks(utc(2026, 10, 7, 10, 0), utc(2026, 10, 7, 12, 0), 4, -1_800);
        assert_eq!(ticks[0].at, utc(2026, 10, 7, 10, 30));
    }

    #[test]
    fn a_long_span_skips_years_to_stay_within_the_count() {
        let ticks = time_ticks(utc(1900, 1, 1, 0, 0), utc(2000, 1, 1, 0, 0), 5, 0);
        assert!(ticks.len() <= 5, "{:?}", labels(&ticks));
        assert_eq!(labels(&ticks)[0], "1900");
        assert!(time_ticks(f64::NAN, 1.0, 5, 0).is_empty());
        assert!(time_ticks(2.0, 1.0, 5, 0).is_empty());
    }

    #[test]
    fn the_tooltip_writes_the_time_for_short_spans_and_the_year_for_long_ones() {
        let at = utc(2026, 10, 8, 14, 2);
        assert_eq!(time_long(at, DAY, 0), "Thu, Oct 8, 14:02");
        assert_eq!(time_long(at, 30.0 * DAY, 0), "Thu, Oct 8, 2026");
        assert_eq!(time_long(at, DAY, 9 * 3_600), "Thu, Oct 8, 23:02");
    }
}
