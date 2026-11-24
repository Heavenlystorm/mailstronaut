//! Saved drafts and outgoing messages. Every resend is an explicit user decision.

use gpui::base::StyledExt as _;
use gpui::component::{
    ActiveTheme as _, Disableable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    label::Label,
};
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use mailstronaut_core::store::outbox::{DraftSummary, Phase, Summary};

use super::{Composer, Target};

/// Entries the user has to act on. Accepted and cancelled mail needs nothing more.
pub const fn needs_decision(entry: &Summary) -> bool {
    matches!(
        entry.phase,
        Phase::Pending | Phase::Failed | Phase::DeliveryUnknown
    )
}

const fn phase_text(entry: &Summary) -> &'static str {
    match entry.phase {
        Phase::Pending => "Waiting to send",
        Phase::Submitting => "Sending…",
        Phase::Accepted if entry.sent_copy_pending => "Sent · copy to Sent folder pending",
        Phase::Accepted => "Sent",
        Phase::DeliveryUnknown => "Delivery unknown",
        Phase::Failed => "Not sent",
        Phase::Cancelled => "Cancelled",
    }
}

fn subject(value: &str) -> SharedString {
    if value.is_empty() {
        "(no subject)".into()
    } else {
        value.to_owned().into()
    }
}

impl Composer {
    pub(super) fn lists(&self, cx: &Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let visible: Vec<_> = self
            .outgoing
            .iter()
            .filter(|entry| entry.phase != Phase::Cancelled)
            .collect();
        div()
            .id("composer-lists")
            .size_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(border)
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                Button::new("new-message")
                    .primary()
                    .w_full()
                    .label("New message")
                    .tooltip("Ctrl+N")
                    .disabled(self.busy() || self.store.is_none() || self.identities.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.compose_new(window, cx))),
            )
            .child(div().pt_2().font_semibold().text_sm().child("Drafts"))
            .when(self.drafts.is_empty(), |view| {
                view.child(Label::new("No saved drafts.").text_sm().text_color(muted))
            })
            .children(
                self.drafts
                    .iter()
                    .enumerate()
                    .map(|(index, draft)| self.draft_row(index, draft, cx)),
            )
            .child(div().pt_2().font_semibold().text_sm().child("Outbox"))
            .when(visible.is_empty(), |view| {
                view.child(
                    Label::new("Nothing waiting to send.")
                        .text_sm()
                        .text_color(muted),
                )
            })
            .children(
                visible
                    .into_iter()
                    .enumerate()
                    .map(|(index, entry)| self.outbox_row(index, entry, cx)),
            )
    }

    fn draft_row(
        &self,
        index: usize,
        draft: &DraftSummary,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let open = self
            .draft
            .as_ref()
            .is_some_and(|current| current.id == draft.id);
        let (account, id) = (draft.account.clone(), draft.id.clone());
        Button::new(("open-draft", index))
            .ghost()
            .w_full()
            .justify_start()
            .label(subject(&draft.subject))
            .tooltip(SharedString::from(draft.account.clone()))
            .disabled(self.busy() || open)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.switch(
                    Target::Saved {
                        account: account.clone(),
                        id: id.clone(),
                    },
                    window,
                    cx,
                );
            }))
    }

    fn outbox_row(&self, index: usize, entry: &Summary, cx: &Context<Self>) -> impl IntoElement {
        let busy = self.busy();
        let muted = cx.theme().muted_foreground;
        let row = entry.clone();
        let may_duplicate = entry.phase == Phase::DeliveryUnknown || entry.uncertain;
        let can_send = needs_decision(entry);
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .child(Label::new(subject(&entry.subject)).truncate())
            .child(
                Label::new(format!("{} · {}", phase_text(entry), entry.account))
                    .text_xs()
                    .text_color(muted),
            )
            .when_some(entry.error.clone(), |view, error| {
                view.child(Label::new(error).text_xs().text_color(muted))
            })
            .when(may_duplicate, |view| {
                view.child(
                    Label::new("An earlier attempt may have delivered this. Check Sent first.")
                        .text_xs()
                        .text_color(muted),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .when(can_send, |actions| {
                        let target = row.clone();
                        actions.child(
                            Button::new(("send-queued", index))
                                .ghost()
                                .label(if may_duplicate {
                                    "Send again…"
                                } else if entry.phase == Phase::Failed {
                                    "Retry"
                                } else {
                                    "Send"
                                })
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.request_resend(&target, window, cx);
                                })),
                        )
                    })
                    .when(entry.phase == Phase::Pending, |actions| {
                        let target = row.clone();
                        actions.child(
                            Button::new(("cancel-queued", index))
                                .ghost()
                                .label("Cancel")
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.cancel_queued(&target, window, cx);
                                })),
                        )
                    })
                    .when(
                        entry.phase == Phase::Failed && !entry.uncertain,
                        |actions| {
                            let (account, id) = (row.account.clone(), row.id.clone());
                            actions.child(
                                Button::new(("recover-draft", index))
                                    .ghost()
                                    .label("Edit as new draft")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.switch(
                                            Target::Recover {
                                                account: account.clone(),
                                                id: id.clone(),
                                            },
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                        },
                    ),
            )
    }

    /// A possibly delivered message needs conscious consent; other resends do not.
    fn request_resend(&mut self, entry: &Summary, window: &mut Window, cx: &mut Context<Self>) {
        let retry = match entry.phase {
            Phase::Pending => None,
            _ => Some(false),
        };
        if entry.phase != Phase::DeliveryUnknown && !entry.uncertain {
            self.submit(entry, retry, window, cx);
            return;
        }
        let this = cx.entity().downgrade();
        let entry = entry.clone();
        let declaration = self.declaration.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (this, entry) = (this.clone(), entry.clone());
            let declaration = declaration.clone();
            alert
                .confirm()
                .title("Send this message again?")
                .description(
                    "An earlier attempt may already have been delivered, so recipients could get it twice. Check your Sent folder first.",
                )
                .ok_text("Send again")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, window, cx| {
                    if !declaration.is_cancelled() {
                        _ = this.update(cx, |this, cx| {
                            this.submit(&entry, Some(true), window, cx);
                        });
                    }
                    true
                })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{Phase, Summary, needs_decision, phase_text};

    fn entry(phase: Phase) -> Summary {
        Summary {
            subject: String::new(),
            id: String::new(),
            account: String::new(),
            phase,
            error: None,
            sent_copy_pending: false,
            uncertain: false,
        }
    }

    #[test]
    fn only_undecided_mail_asks_for_attention_and_accepted_mail_cannot_resend() {
        for phase in [Phase::Pending, Phase::Failed, Phase::DeliveryUnknown] {
            assert!(needs_decision(&entry(phase)));
        }
        for phase in [Phase::Submitting, Phase::Accepted, Phase::Cancelled] {
            assert!(!needs_decision(&entry(phase)));
        }
    }

    #[test]
    fn accepted_mail_distinguishes_a_pending_sent_copy() {
        let mut accepted = entry(Phase::Accepted);
        assert_eq!(phase_text(&accepted), "Sent");
        accepted.sent_copy_pending = true;
        assert!(phase_text(&accepted).contains("pending"));
    }
}
