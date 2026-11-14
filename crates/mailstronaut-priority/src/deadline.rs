//! Deadlines stated in a message, with the exact text they came from.
//!
//! Precision comes first: a date counts only when a deadline cue is directly attached to
//! it, in the same clause, so meetings, flights and invoice dates do not qualify. Forms that
//! cannot be resolved with certainty are returned as uncertain, for showing but not for
//! acting on. Nothing here reads a clock: every date is relative to the message itself.

use core::ops::Range;
use std::sync::LazyLock;

use chrono::{
    DateTime, Datelike as _, Days, LocalResult, NaiveDate, NaiveTime, TimeZone as _, Weekday,
};
use chrono_tz::Tz;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

/// Beyond this much text, a message is not asking for something the rules can see.
const MAX_TEXT_BYTES: usize = 32 * 1024;
const MAX_DEADLINES: usize = 16;
/// A date without a year is the next one within this many days; otherwise it has passed,
/// and must not silently become next year's.
const MAX_DAYS_AHEAD_WITHOUT_YEAR: i64 = 183;

/// How the date was written.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Form {
    /// A calendar date.
    Explicit,
    /// Relative to the message, such as "tomorrow" or a weekday.
    Relative,
}

/// A deadline found in a message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Deadline {
    /// Byte range of the evidence in the original body.
    pub span: Range<usize>,
    /// The evidence itself.
    pub quote: String,
    /// The local date.
    pub date: NaiveDate,
    /// The local time, when one was stated.
    pub time: Option<NaiveTime>,
    /// The zone the date was resolved in.
    pub zone: String,
    /// True when the message did not state a zone.
    pub zone_assumed: bool,
    /// How the date was written.
    pub form: Form,
    /// First instant after the deadline. `None` means uncertain: show it, ask if needed,
    /// but never let it decide priority or expiry.
    pub ends_at: Option<i64>,
}

const MONTHS: &str = r"january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sep|sept|oct|nov|dec|janvier|f[ée]vrier|mars|avril|mai|juin|juillet|ao[uû]t|septembre|octobre|novembre|d[ée]cembre|janv|f[ée]vr|avr|juil|d[ée]c|gennaio|febbraio|marzo|aprile|maggio|giugno|luglio|agosto|settembre|ottobre|dicembre|gen|mag|giu|lug|ago|set|ott|dic";
const WEEKDAYS: &str = r"monday|tuesday|wednesday|thursday|friday|saturday|sunday|lundi|mardi|mercredi|jeudi|vendredi|samedi|dimanche|luned[iì]|marted[iì]|mercoled[iì]|gioved[iì]|venerd[iì]|sabato|domenica";
const RELATIVE: &str = r"today|tomorrow|aujourd'hui|aujourd’hui|demain|oggi|domani";

/// Cues that make the date right after them an obligation.
const CUES: &str = r"due(?:\s+date)?(?:\s+(?:is|on|by))*|deadline(?:\s+(?:is|for\s+[^,:;]{1,40}\s+is))?\s*:?|(?:submit|send|reply|respond|register|apply|return|complete|confirm|sign|pay|renew|finish|deliver)\b[^,:;]{0,60}?\s+(?:by|before)|by|before|no\s+later\s+than|not\s+later\s+than|expires?(?:\s+on)?|date\s+limite(?:\s+(?:de|d'|du|des|pour)[^,:;]{1,40})?(?:\s+est)?\s*:?(?:\s+(?:le|au))?|avant(?:\s+(?:le|ce|cette))?|au\s+plus\s+tard(?:\s+(?:le|ce|cette))?|d['’]ici(?:\s+(?:le|au|à|ce|cette))?|[ée]ch[ée]ance\s*:?(?:\s+(?:le|au))?|jusqu['’]au|entro(?:\s+e\s+non\s+oltre)?(?:\s+(?:il|la|le|lo|l'|questo|questa))?|scadenza(?:\s+(?:per|di|del|della)[^,:;]{1,40})?\s*:?(?:\s+(?:il|è\s+il|e\s+il))?|scade(?:\s+il)?|non\s+oltre(?:\s+il)?|prima\s+(?:del|di)|termine(?:\s+ultimo)?\s*:?(?:\s+il)?";

static DEADLINE: LazyLock<Regex> = LazyLock::new(|| {
    let day = r"(?P<day>\d{1,2})(?:st|nd|rd|th|er|°|º)?\s+(?:de\s+|di\s+)?(?P<month>MONTHS)\.?(?:,?\s+(?P<year>\d{4}))?";
    let month =
        r"(?P<month2>MONTHS)\.?\s+(?P<day2>\d{1,2})(?:st|nd|rd|th)?(?:,?\s+(?P<year2>\d{4}))?";
    let date = format!(
        r"(?:(?:the|le|il|on)\s+)?(?:(?P<iso>\d{{4}}-\d{{2}}-\d{{2}})|(?P<numeric>\d{{1,2}}[/.]\d{{1,2}}(?:[/.]\d{{2,4}})?)|{day}|{month}|(?:(?:this|next|ce|cette|questo|questa)\s+)?(?P<weekday>WEEKDAYS)(?:\s+prochain)?|(?P<relative>RELATIVE))",
    );
    let time = r"(?:(?:\s*,)?\s*(?:at|à|a|alle|ore|h)?\s*(?P<hour>\d{1,2})(?:(?::|h|\.)(?P<minute>\d{2}))?\s*(?P<meridiem>am|pm|h)?(?:\s*(?P<zone>[A-Z]{2,5}|[+-]\d{2}:?\d{2}))?)?";
    let pattern = format!(r"(?i)(?:^|[\s(\[«])(?P<cue>{CUES})\s*{date}\b{time}")
        .replace("MONTHS", MONTHS)
        .replace("WEEKDAYS", WEEKDAYS)
        .replace("RELATIVE", RELATIVE);
    Regex::new(&pattern).expect("static pattern")
});
/// A cue undone by what comes just before it.
static NEGATED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:not|no\s+longer|isn't|aren't|wasn't|was|were|n'est\s+pas|ne\s+\S+\s+pas|plus|était|non|era|nessun[ao]?)\s+\S*\s*$")
        .expect("static pattern")
});
/// A clause about something other than an obligation of the reader's.
static NOT_AN_OBLIGATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:sale|offer|discount|coupon|promo\w*|deals?|shipping|shipped|arriv\w+|deliver(?:ed|y)|flight|boarding|birthday|invoice\s+date|issued|soldes|offres?|r[ée]duction|livraison|vol|anniversaire|sconto|offert\w+|saldi|consegna|consegne|volo|compleanno|spedizion\w+)\b|\d+\s*%")
        .expect("static pattern")
});
/// Where the author's own text ends.
static QUOTE_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:on\s.+\bwrote:|le\s.+\ba\s+écrit\s*:|il\s.+\bha\s+scritto:|-{2,}\s*(?:original\s+message|message\s+d'origine|messaggio\s+originale|forwarded|message\s+transféré|messaggio\s+inoltrato).*)\s*$")
        .expect("static pattern")
});

/// The ranges of a body the author wrote: no quoted lines, reply tails or signature.
pub(crate) fn authored(body: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if start >= MAX_TEXT_BYTES {
            break;
        }
        let text = line.trim_end_matches(['\r', '\n']);
        if text == "-- " || QUOTE_HEADER.is_match(text.trim()) {
            break;
        }
        if !text.trim_start().starts_with('>') {
            ranges.push(start..start + text.len());
        }
    }
    ranges
}

/// Clauses within a line, so that "not due tomorrow; due Friday" reads as two statements.
fn clauses(line: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut start = 0;
    let mut pieces = Vec::new();
    let bytes = line.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        let ends = matches!(byte, b';' | b'!' | b'?')
            || (*byte == b'.' && bytes.get(index + 1).is_none_or(u8::is_ascii_whitespace));
        if ends && line.is_char_boundary(index + 1) {
            pieces.push((start, &line[start..=index]));
            start = index + 1;
        }
    }
    pieces.push((start, &line[start..]));
    pieces.into_iter()
}

fn month_number(name: &str) -> Option<u32> {
    let folded: String = name
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'é' | 'è' => 'e',
            'û' => 'u',
            other => other,
        })
        .collect();
    Some(match folded.as_str() {
        "january" | "jan" | "janvier" | "janv" | "gennaio" | "gen" => 1,
        "february" | "feb" | "fevrier" | "fevr" | "febbraio" => 2,
        "march" | "mar" | "mars" | "marzo" => 3,
        "april" | "apr" | "avril" | "avr" | "aprile" => 4,
        "may" | "mai" | "maggio" | "mag" => 5,
        "june" | "jun" | "juin" | "giugno" | "giu" => 6,
        "july" | "jul" | "juillet" | "juil" | "luglio" | "lug" => 7,
        "august" | "aug" | "aout" | "agosto" | "ago" => 8,
        "september" | "sep" | "sept" | "septembre" | "settembre" | "set" => 9,
        "october" | "oct" | "octobre" | "ottobre" | "ott" => 10,
        "november" | "nov" | "novembre" => 11,
        "december" | "dec" | "decembre" | "dicembre" | "dic" => 12,
        _ => return None,
    })
}

fn weekday_of(name: &str) -> Option<Weekday> {
    Some(match name.to_lowercase().replace('ì', "i").as_str() {
        "monday" | "lundi" | "lunedi" => Weekday::Mon,
        "tuesday" | "mardi" | "martedi" => Weekday::Tue,
        "wednesday" | "mercredi" | "mercoledi" => Weekday::Wed,
        "thursday" | "jeudi" | "giovedi" => Weekday::Thu,
        "friday" | "vendredi" | "venerdi" => Weekday::Fri,
        "saturday" | "samedi" | "sabato" => Weekday::Sat,
        "sunday" | "dimanche" | "domenica" => Weekday::Sun,
        _ => return None,
    })
}

/// A resolved date, and whether the writing leaves any doubt about it.
struct Resolved {
    date: NaiveDate,
    certain: bool,
    form: Form,
}

/// The next such date on or after the anchor, unless that is too far away to be meant.
fn without_year(month: u32, day: u32, anchor: NaiveDate) -> Option<NaiveDate> {
    let this_year = NaiveDate::from_ymd_opt(anchor.year(), month, day);
    let candidate = match this_year {
        Some(date) if date >= anchor => date,
        _ => NaiveDate::from_ymd_opt(anchor.year() + 1, month, day)?,
    };
    ((candidate - anchor).num_days() <= MAX_DAYS_AHEAD_WITHOUT_YEAR).then_some(candidate)
}

fn numeric(text: &str, anchor: NaiveDate) -> Option<Resolved> {
    let mut parts = text.split(['/', '.']);
    let first: u32 = parts.next()?.parse().ok()?;
    let second: u32 = parts.next()?.parse().ok()?;
    let year = parts.next().map(str::parse::<i32>).transpose().ok()?;
    let year = year.map(|year| if year < 100 { 2000 + year } else { year });
    // Only one reading can be a date, or both readings are the same date; otherwise this
    // is ambiguous between day-first and month-first, which must not be guessed.
    let certain = first > 12 || second > 12 || first == second;
    let (day, month) = if second > 12 {
        (second, first)
    } else {
        (first, second)
    };
    let date = match year {
        Some(year) => NaiveDate::from_ymd_opt(year, month, day)?,
        None => without_year(month, day, anchor)?,
    };
    Some(Resolved {
        date,
        certain,
        form: Form::Explicit,
    })
}

fn relative(text: &str, anchor: NaiveDate) -> Option<Resolved> {
    let days = match text.to_lowercase().replace('’', "'").as_str() {
        "today" | "aujourd'hui" | "oggi" => 0,
        "tomorrow" | "demain" | "domani" => 1,
        _ => return None,
    };
    Some(Resolved {
        date: anchor.checked_add_days(Days::new(days))?,
        certain: true,
        form: Form::Relative,
    })
}

fn weekday(name: &str, anchor: NaiveDate) -> Option<Resolved> {
    let target = weekday_of(name)?;
    let ahead = (7 + i64::from(target.num_days_from_monday())
        - i64::from(anchor.weekday().num_days_from_monday()))
        % 7;
    // "By Friday", sent on a Friday, means today or next week: only the writer knows.
    let certain = ahead != 0;
    Some(Resolved {
        date: anchor.checked_add_days(Days::new(u64::try_from(ahead).ok()?))?,
        certain,
        form: Form::Relative,
    })
}

fn resolve(found: &Captures<'_>, anchor: NaiveDate) -> Option<Resolved> {
    let text = |name: &str| found.name(name).map(|value| value.as_str());
    if let Some(iso) = text("iso") {
        return Some(Resolved {
            date: NaiveDate::parse_from_str(iso, "%Y-%m-%d").ok()?,
            certain: true,
            form: Form::Explicit,
        });
    }
    if let Some(value) = text("numeric") {
        return numeric(value, anchor);
    }
    if let Some(value) = text("relative") {
        return relative(value, anchor);
    }
    if let Some(value) = text("weekday") {
        return weekday(value, anchor);
    }
    let (day, month, year) = text("day")
        .map(|day| (day, text("month"), text("year")))
        .or_else(|| text("day2").map(|day| (day, text("month2"), text("year2"))))?;
    let (day, month) = (day.parse().ok()?, month_number(month?)?);
    let date = match year {
        Some(year) => NaiveDate::from_ymd_opt(year.parse().ok()?, month, day)?,
        None => without_year(month, day, anchor)?,
    };
    Some(Resolved {
        date,
        certain: true,
        form: Form::Explicit,
    })
}

/// A stated time, and whether its zone can be trusted. An abbreviation such as PST is
/// not a zone: several of them mean different things.
fn stated_time(found: &Captures<'_>) -> (Option<NaiveTime>, Option<i32>, bool) {
    let Some(hour) = found
        .name("hour")
        .and_then(|value| value.as_str().parse::<u32>().ok())
    else {
        return (None, None, true);
    };
    let minute = found
        .name("minute")
        .and_then(|value| value.as_str().parse::<u32>().ok());
    let meridiem = found
        .name("meridiem")
        .map(|value| value.as_str().to_lowercase());
    // A bare number after a date is usually not a time at all ("by 15 November 30 people").
    if minute.is_none() && meridiem.is_none() {
        return (None, None, true);
    }
    let hour = match meridiem.as_deref() {
        Some("pm") if hour < 12 => hour + 12,
        Some("am") if hour == 12 => 0,
        _ => hour,
    };
    let time = NaiveTime::from_hms_opt(hour, minute.unwrap_or(0), 0);
    let zone = found.name("zone").map(|value| value.as_str());
    let (offset, certain) = match zone {
        None => (None, true),
        Some(zone) if zone.eq_ignore_ascii_case("utc") || zone.eq_ignore_ascii_case("gmt") => {
            (Some(0), true)
        }
        Some(zone) => zone
            .strip_prefix(['+', '-'])
            .map(|digits| digits.replace(':', ""))
            .and_then(|digits| {
                let hours: i32 = digits.get(..2)?.parse().ok()?;
                let minutes: i32 = digits.get(2..4)?.parse().ok()?;
                let total = hours * 3600 + minutes * 60;
                Some(if zone.starts_with('-') { -total } else { total })
            })
            .map_or((None, false), |offset| (Some(offset), true)),
    };
    (time, offset, certain)
}

/// The first instant in this zone at or after local midnight of a date. Midnight itself
/// does not exist on some clock changes, and happens twice on others.
fn start_of_day(date: NaiveDate, zone: Tz) -> Option<DateTime<Tz>> {
    let midnight = date.and_hms_opt(0, 0, 0)?;
    (0..=3).find_map(|hours| {
        match zone.from_local_datetime(&(midnight + chrono::Duration::hours(hours))) {
            LocalResult::Single(instant) => Some(instant),
            LocalResult::Ambiguous(earliest, _) => Some(earliest),
            LocalResult::None => None,
        }
    })
}

/// A date-only deadline lasts through that whole local day. "Due tomorrow" is still due
/// tomorrow evening, and over at the next local midnight, however long that day is.
fn ends_at(date: NaiveDate, time: Option<NaiveTime>, offset: Option<i32>, zone: Tz) -> Option<i64> {
    let Some(time) = time else {
        return Some(start_of_day(date.checked_add_days(Days::new(1))?, zone)?.timestamp());
    };
    let local = date.and_time(time);
    if let Some(offset) = offset {
        return Some(local.and_utc().timestamp() - i64::from(offset));
    }
    // A time in a clock change's gap or fold does not identify one instant.
    match zone.from_local_datetime(&local) {
        LocalResult::Single(instant) => Some(instant.timestamp()),
        _ => None,
    }
}

/// Deadlines in the author's own text, relative to when the message was written.
#[must_use]
pub fn find(body: &str, anchor: DateTime<Tz>) -> Vec<Deadline> {
    constraints(body, anchor)
        .into_iter()
        .filter(|deadline| deadline.date >= anchor.date_naive())
        .collect()
}

/// Past obligations still constrain priority even though they are not upcoming.
pub(crate) fn constraints(body: &str, anchor: DateTime<Tz>) -> Vec<Deadline> {
    let zone = anchor.timezone();
    let today = anchor.date_naive();
    let mut found = Vec::new();
    for range in authored(body) {
        let line = &body[range.clone()];
        for (offset, clause) in clauses(line) {
            if NOT_AN_OBLIGATION.is_match(clause) {
                continue;
            }
            let mut previous = 0;
            for captures in DEADLINE.captures_iter(clause) {
                let Some(whole) = captures.get(0) else {
                    continue;
                };
                let cue = captures
                    .name("cue")
                    .map_or_else(|| whole.start(), |cue| cue.start());
                let before = clause.get(previous..cue).unwrap_or_default();
                previous = whole.end();
                if NEGATED.is_match(before) {
                    continue;
                }
                let Some(resolved) = resolve(&captures, today) else {
                    continue;
                };
                let (time, stated_offset, zone_certain) = stated_time(&captures);
                let ending = if resolved.certain && zone_certain {
                    ends_at(resolved.date, time, stated_offset, zone)
                } else {
                    None
                };
                let start = range.start + offset + cue;
                let span = start..range.start + offset + whole.end();
                let deadline = Deadline {
                    quote: body[span.clone()].to_owned(),
                    span,
                    date: resolved.date,
                    time,
                    zone: zone.name().to_owned(),
                    zone_assumed: stated_offset.is_none(),
                    form: resolved.form,
                    ends_at: ending,
                };
                let repeated = found.iter().any(|other: &Deadline| {
                    other.date == deadline.date && other.time == deadline.time
                });
                if !repeated && found.len() < MAX_DEADLINES {
                    found.push(deadline);
                }
            }
        }
    }
    found.sort_by_key(|deadline| (deadline.date, deadline.time));
    found
}
