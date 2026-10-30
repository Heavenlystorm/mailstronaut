//! Civil-time reminder policy using the existing timezone database.

use chrono::{
    DateTime, Days, LocalResult, NaiveDateTime, NaiveTime, TimeDelta, TimeZone as _, Utc,
};
use chrono_tz::Tz;

/// A native user's explicit time selection, never derived from message instructions.
#[derive(Clone, Debug)]
pub enum When {
    /// An elapsed hour from now, independent of DST changes.
    Hour,
    /// Today's configured evening in the chosen timezone.
    Evening,
    /// Next calendar day's configured morning in the chosen timezone.
    Tomorrow,
    /// An explicitly chosen civil date/time.
    Custom(NaiveDateTime),
}
/// Static time-selection errors, without user/message text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeError {
    /// Invalid IANA zone, date or configured time.
    Invalid,
    /// The selected wall time does not exist due to a timezone transition.
    Gap,
    /// The selected instant has already passed.
    Past,
}
impl core::fmt::Display for TimeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self{Self::Invalid=>"Invalid reminder date, time or timezone.",Self::Gap=>"That local time does not exist because the clock moves forward. Choose another time.",Self::Past=>"Choose a reminder time in the future."})
    }
}
impl core::error::Error for TimeError {}
/// Resolve a preset with existing civil-time rules. On a fold select the first
/// still-future occurrence; never silently shift a nonexistent wall time.
/// # Errors
/// Rejects unsupported dates/zones, DST gaps and past selections.
pub fn resolve(
    now: i64,
    zone: &str,
    when: &When,
    morning: &str,
    evening: &str,
) -> Result<i64, TimeError> {
    let now = DateTime::from_timestamp(now, 0).ok_or(TimeError::Invalid)?;
    let zone = zone.parse::<Tz>().map_err(|_| TimeError::Invalid)?;
    if matches!(when, When::Hour) {
        return now
            .checked_add_signed(TimeDelta::hours(1))
            .map(|at| at.timestamp())
            .ok_or(TimeError::Invalid);
    }
    let today = now.with_timezone(&zone).date_naive();
    let local = match when {
        When::Hour => return Err(TimeError::Invalid),
        When::Evening => today.and_time(time(evening)?),
        When::Tomorrow => today
            .checked_add_days(Days::new(1))
            .ok_or(TimeError::Invalid)?
            .and_time(time(morning)?),
        When::Custom(local) => *local,
    };
    let at = match zone.from_local_datetime(&local) {
        LocalResult::None => return Err(TimeError::Gap),
        LocalResult::Single(at) => at.with_timezone(&Utc),
        LocalResult::Ambiguous(a, b) => [a, b]
            .into_iter()
            .map(|at| at.with_timezone(&Utc))
            .filter(|at| *at > now)
            .min()
            .ok_or(TimeError::Past)?,
    };
    if at <= now {
        return Err(TimeError::Past);
    }
    Ok(at.timestamp())
}
fn time(value: &str) -> Result<NaiveTime, TimeError> {
    if value.len() != 5 {
        return Err(TimeError::Invalid);
    }
    NaiveTime::parse_from_str(value, "%H:%M").map_err(|_| TimeError::Invalid)
}

/// Display a stored UTC instant in its original IANA zone, including the offset.
/// # Errors
/// Rejects invalid timestamps and unknown zones.
pub fn display(at: i64, zone: &str) -> Result<String, TimeError> {
    let at = DateTime::from_timestamp(at, 0).ok_or(TimeError::Invalid)?;
    let zone = zone.parse::<Tz>().map_err(|_| TimeError::Invalid)?;
    Ok(at
        .with_timezone(&zone)
        .format("%Y-%m-%d %H:%M %Z (%:z)")
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn utc(value: &str) -> i64 {
        DateTime::parse_from_rfc3339(value)
            .expect("UTC fixture")
            .timestamp()
    }
    fn civil(value: &str) -> When {
        When::Custom(NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").expect("civil fixture"))
    }
    #[test]
    fn civil_presets_use_calendar_days_and_elapsed_hours_across_dst() {
        let now = utc("2026-03-28T08:00:00Z");
        assert_eq!(
            resolve(now, "Europe/Paris", &When::Tomorrow, "09:00", "18:00").expect("morning"),
            utc("2026-03-29T07:00:00Z")
        );
        assert_eq!(
            resolve(
                utc("2026-03-29T00:30:00Z"),
                "Europe/Paris",
                &When::Hour,
                "09:00",
                "18:00"
            )
            .expect("elapsed hour"),
            utc("2026-03-29T01:30:00Z")
        );
        assert_eq!(
            resolve(now, "Europe/Paris", &When::Evening, "09:00", "18:00").expect("evening"),
            utc("2026-03-28T17:00:00Z")
        );
        assert_eq!(
            resolve(
                utc("2026-03-28T20:00:00Z"),
                "Europe/Paris",
                &When::Evening,
                "09:00",
                "18:00"
            ),
            Err(TimeError::Past)
        );
    }
    #[test]
    fn gaps_are_rejected_and_folds_choose_the_first_still_future_occurrence() {
        assert_eq!(
            resolve(
                utc("2026-03-28T00:00:00Z"),
                "Europe/Paris",
                &civil("2026-03-29 02:30"),
                "09:00",
                "18:00"
            ),
            Err(TimeError::Gap)
        );
        assert_eq!(
            resolve(
                utc("2026-10-25T00:00:00Z"),
                "Europe/Paris",
                &civil("2026-10-25 02:30"),
                "09:00",
                "18:00"
            )
            .expect("first fold"),
            utc("2026-10-25T00:30:00Z")
        );
        assert_eq!(
            resolve(
                utc("2026-10-25T00:45:00Z"),
                "Europe/Paris",
                &civil("2026-10-25 02:30"),
                "09:00",
                "18:00"
            )
            .expect("later fold"),
            utc("2026-10-25T01:30:00Z")
        );
        assert_eq!(
            resolve(100, "invalid/zone", &When::Hour, "09:00", "18:00"),
            Err(TimeError::Invalid)
        );
    }
}
