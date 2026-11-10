//! Standard sender input and date-range picker for local mail search.

use chrono::{NaiveDate, TimeZone as _};
use gpui::component::{
    ActiveTheme as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    date_picker::{DatePicker, DatePickerState, DateRangePreset, DateTime},
    input::{Input, InputState},
};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use mailstronaut_core::store::browse::SearchFilter;

pub enum Event {
    Applied(SearchFilter),
}

pub struct Filters {
    sender: Entity<InputState>,
    dates: Entity<DatePickerState>,
    zone: String,
    error: Option<&'static str>,
    applied_dates: (Option<NaiveDate>, Option<NaiveDate>),
    applied_sender: String,
}

fn bounds(
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
    zone: &str,
) -> Result<(Option<i64>, Option<i64>), &'static str> {
    match (start, end) {
        (None, None) => Ok((None, None)),
        (Some(start), Some(end)) if start <= end => {
            let zone = zone
                .parse::<chrono_tz::Tz>()
                .map_err(|_| "The search time zone is unavailable.")?;
            let midnight = |date: NaiveDate| {
                date.and_hms_opt(0, 0, 0)
                    .and_then(|local| zone.from_local_datetime(&local).earliest())
                    .map(|instant| instant.timestamp())
                    .ok_or("This calendar boundary does not exist in the selected time zone.")
            };
            Ok((
                Some(midnight(start)?),
                Some(midnight(end.succ_opt().ok_or("Date range is too large.")?)?),
            ))
        }
        _ => Err("Choose a complete date range, or clear it."),
    }
}

impl Filters {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            sender: cx.new(|cx| InputState::new(window, cx).placeholder("Sender name or address")),
            dates: cx.new(|cx| DatePickerState::range(window, cx)),
            zone: crate::mailbox::local_zone(None),
            error: None,
            applied_dates: (None, None),
            applied_sender: String::new(),
        }
    }

    pub fn set_zone(&mut self, zone: String, _: &mut Window, cx: &mut Context<Self>) {
        if self.zone != zone {
            self.zone = zone;
            match bounds(self.applied_dates.0, self.applied_dates.1, &self.zone) {
                Ok((received_from, received_before)) => cx.emit(Event::Applied(SearchFilter {
                    received_from,
                    received_before,
                    sender: self.applied_sender.clone(),
                })),
                Err(error) => self.error = Some(error),
            }
            cx.notify();
        }
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let (start, end) = match self.dates.read(cx).date_time() {
            DateTime::Range(start, end) => {
                (start.map(|date| date.date()), end.map(|date| date.date()))
            }
            DateTime::Single(_) => (None, None),
        };
        match bounds(start, end, &self.zone) {
            Ok((received_from, received_before)) => {
                let sender = self.sender.read(cx).value().trim().to_owned();
                if sender.len() > 256 || sender.chars().any(char::is_control) {
                    self.error = Some("Use at most 256 bytes of plain sender text.");
                } else {
                    self.error = None;
                    self.applied_dates = (start, end);
                    self.applied_sender.clone_from(&sender);
                    cx.emit(Event::Applied(SearchFilter {
                        received_from,
                        received_before,
                        sender,
                    }));
                }
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sender
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.dates.update(cx, |state, cx| {
            state.set_date_time(DateTime::Range(None, None), window, cx);
        });
        self.error = None;
        self.applied_dates = (None, None);
        self.applied_sender.clear();
        cx.emit(Event::Applied(SearchFilter::default()));
        cx.notify();
    }
}
impl EventEmitter<Event> for Filters {}

impl Render for Filters {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let zone = self.zone.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC);
        let today = chrono::Utc::now().with_timezone(&zone).date_naive();
        div()
            .w(px(360.))
            .flex()
            .flex_col()
            .gap_3()
            .child("Search filters")
            .child(div().text_sm().child("Sender contains"))
            .child(Input::new(&self.sender))
            .child(div().text_sm().child("Received between (inclusive)"))
            .child(
                DatePicker::new(&self.dates)
                    .cleanable(true)
                    .placeholder("Any date")
                    .presets(vec![
                        DateRangePreset::range("Today", today, today),
                        DateRangePreset::range(
                            "Last 30 days",
                            today - chrono::Days::new(29),
                            today,
                        ),
                    ]),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "Calendar dates use {}. Account and folder follow the sidebar.",
                        self.zone
                    )),
            )
            .when_some(self.error, |view, error| {
                view.child(Alert::warning("search-filter-error", error))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("apply-search-filters")
                            .primary()
                            .label("Apply filters")
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    )
                    .child(
                        Button::new("clear-search-filters")
                            .label("Clear filters")
                            .on_click(cx.listener(|this, _, window, cx| this.clear(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inclusive_calendar_ranges_respect_dst_and_reject_incomplete_values() {
        let day = NaiveDate::from_ymd_opt(2026, 3, 29).expect("date");
        let (from, before) = bounds(Some(day), Some(day), "Europe/Paris").expect("range");
        assert_eq!(
            before.zip(from).map(|(before, from)| before - from),
            Some(23 * 3600)
        );
        assert!(bounds(Some(day), None, "UTC").is_err());
        assert_eq!(bounds(None, None, "UTC").expect("clear"), (None, None));
        assert!(bounds(Some(day.succ_opt().expect("next")), Some(day), "UTC").is_err());
    }
}
