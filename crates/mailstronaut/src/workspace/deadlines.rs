//! Native deadline correction for the currently opened original.
use super::Workspace;
use gpui::component::{
    ActiveTheme as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    date_picker::{DatePicker, DatePickerState, DateTime},
    label::Label,
    notification::{Notification, NotificationDelivery},
    time_field::TimePrecision,
};
use gpui::{
    AppContext as _, Context, IntoElement, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use mailstronaut_core::store::analysis::CorrectDeadline;

impl Workspace {
    pub(super) fn deadline_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let visible = self.corrected_deadline.is_some()
            || self
                .opened_analysis
                .as_ref()
                .is_some_and(|analysis| !analysis.deadlines.is_empty());
        div().when(visible, |view| {
            view.flex()
                .items_center()
                .gap_2()
                .mb_3()
                .when_some(self.corrected_deadline.as_ref(), |view, corrected| {
                    let date = corrected.date.format("%-d %B %Y");
                    let when = corrected.time.map_or_else(
                        || date.to_string(),
                        |time| format!("{date}, {}", time.format("%H:%M")),
                    );
                    view.child(
                        Label::new(format!("Your deadline: {when} ({})", corrected.zone))
                            .text_sm()
                            .text_color(cx.theme().muted_foreground),
                    )
                })
                .child(
                    Button::new("correct-deadline")
                        .ghost()
                        .label("Correct deadline")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.correct_deadline(window, cx)),
                        ),
                )
        })
    }

    fn correct_deadline(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(row), Some(store)) = (&self.selected, self.handle()) else {
            return;
        };
        let value = self
            .corrected_deadline
            .as_ref()
            .map(|saved| (saved.date, saved.time))
            .or_else(|| {
                self.opened_analysis
                    .as_ref()?
                    .deadlines
                    .first()
                    .map(|date| (date.date, date.time))
            });
        let Some((date, time)) = value else {
            return;
        };
        let zone = self.corrected_deadline.as_ref().map_or_else(
            || self.list.read(cx).delegate().time_zone.clone(),
            |saved| saved.zone.clone(),
        );
        let Ok(zone) = zone.parse::<chrono_tz::Tz>() else {
            return;
        };
        let picker = cx.new(|cx| {
            let picker = DatePickerState::new(window, cx);
            if time.is_some() {
                picker
                    .time_precision(TimePrecision::Minute)
                    .date_format("%Y-%m-%d %H:%M")
            } else {
                picker.date_format("%Y-%m-%d")
            }
        });
        picker.update(cx, |picker, cx| {
            picker.set_date_time(
                date.and_time(time.unwrap_or(chrono::NaiveTime::MIN)),
                window,
                cx,
            );
        });

        let (generation, revision) = (self.reading_generation, self.declaration.revision());
        let (account, message) = (row.account.clone(), row.id);
        let workspace = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let (picker, workspace, store, account) = (
                picker.clone(),
                workspace.clone(),
                store.clone(),
                account.clone(),
            );
            let description = if time.is_some() {
                format!("Choose the due date and time in {zone}.")
            } else {
                format!("The deadline includes the whole selected day in {zone}.")
            };
            dialog
                .confirm()
                .title("Correct deadline")
                .description(description)
                .child(DatePicker::new(&picker).placeholder("Choose a date"))
                .ok_text("Save deadline")
                .on_ok(move |_, window, cx| {
                    let DateTime::Single(Some(local)) = picker.read(cx).date_time() else {
                        return false;
                    };
                    let edit = CorrectDeadline {
                        account: account.clone(),
                        message,
                        date: local.date(),
                        time: time.map(|_| local.time()),
                        zone,
                        now: chrono::Utc::now().timestamp(),
                    };
                    if edit.resolve().is_none() {
                        window.push_notification(
                            Notification::error(
                                "Choose an unambiguous date and time in this timezone.",
                            )
                            .delivery(NotificationDelivery::InApp),
                            cx,
                        );
                        return false;
                    }
                    _ = workspace.update(cx, |workspace, cx| {
                        if generation == workspace.reading_generation
                            && revision == workspace.declaration.revision()
                        {
                            workspace.save_deadline(store.clone(), edit, window, cx);
                        }
                    });
                    true
                })
        });
    }

    fn save_deadline(
        &self,
        store: mailstronaut_core::worker::Handle,
        edit: CorrectDeadline,
        window: &Window,
        cx: &Context<Self>,
    ) {
        let (generation, revision) = (self.reading_generation, self.declaration.revision());
        let job = self
            .runtime
            .spawn(async move { store.correct_deadline(edit).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Ok(corrected)) => {
                        if generation == this.reading_generation
                            && revision == this.declaration.revision()
                        {
                            this.corrected_deadline = Some(corrected);
                        }
                        this.refresh_rows(window, cx);
                    }
                    _ => window.push_notification(
                        Notification::error("Unable to save the deadline.")
                            .delivery(NotificationDelivery::InApp),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
