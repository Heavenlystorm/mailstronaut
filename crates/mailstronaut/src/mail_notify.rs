//! Tells the desktop about new mail while no window is in front.
//!
//! A message's sender and subject appear only if `notifications.showPreview` says so, and
//! its body never does. One notification per account is replaced as mail arrives, rather
//! than one per message. A desktop with no notification portal simply gets none.

use ashpd::desktop::notification::{Notification, NotificationProxy};
use futures_util::StreamExt as _;
use mailstronaut_core::store::analysis::NewMail;
use tokio::{runtime::Runtime, sync::mpsc};

const OPEN_ACTION: &str = "open-inbox";
const MAX_PREVIEW_CHARS: usize = 160;

enum Request {
    Show(String, Box<Notification>),
    Withdraw(String),
}

/// Stops with the application. Cloning its `opened` receiver hears the user's clicks.
pub struct Announcer {
    requests: mpsc::Sender<Request>,
    pub opened: async_channel::Receiver<()>,
    _listener: tokio_util::task::AbortOnDropHandle<()>,
}

fn notification_id(account: &str) -> String {
    format!("new-mail-{account}")
}

fn visible(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control())
        .take(MAX_PREVIEW_CHARS)
        .collect()
}

/// What the notification says. Without a preview it reveals only how many messages came.
pub fn body(count: u32, latest: Option<&NewMail>) -> String {
    let counted = if count == 1 {
        "1 new message".to_owned()
    } else {
        format!("{count} new messages")
    };
    let Some(mail) = latest else {
        return counted;
    };
    let (sender, subject) = (visible(&mail.sender), visible(&mail.subject));
    if count == 1 {
        format!("{sender}\n{subject}")
    } else {
        format!("{counted}, latest from {sender}\n{subject}")
    }
}

impl Announcer {
    pub fn start(runtime: &Runtime) -> Self {
        let (requests, receiver) = mpsc::channel(8);
        let (clicked, opened) = async_channel::bounded(1);
        let listener =
            tokio_util::task::AbortOnDropHandle::new(runtime.spawn(serve(receiver, clicked)));
        Self {
            requests,
            opened,
            _listener: listener,
        }
    }

    /// Replaces any earlier notification for this account.
    pub fn show(&self, account: &str, label: &str, count: u32, latest: Option<&NewMail>) {
        let notification = Notification::new(&format!("New mail · {}", visible(label)))
            .body(body(count, latest).as_str())
            .default_action(OPEN_ACTION);
        // A full queue means the desktop is not answering, and the next one replaces this.
        _ = self.requests.try_send(Request::Show(
            notification_id(account),
            Box::new(notification),
        ));
    }

    pub fn withdraw(&self, account: &str) {
        _ = self
            .requests
            .try_send(Request::Withdraw(notification_id(account)));
    }
}

async fn serve(mut requests: mpsc::Receiver<Request>, clicked: async_channel::Sender<()>) {
    let Ok(proxy) = NotificationProxy::new().await else {
        return;
    };
    let Ok(actions) = proxy.receive_action_invoked().await else {
        return;
    };
    futures_util::pin_mut!(actions);
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                None => return,
                Some(Request::Show(id, notification)) => {
                    _ = crate::reminders::desktop::bounded(async {
                        proxy.add_notification(&id, *notification).await
                            .map_err(|_| "The desktop did not accept the notification.")
                    }).await;
                }
                Some(Request::Withdraw(id)) => {
                    _ = crate::reminders::desktop::bounded(async {
                        proxy.remove_notification(&id).await
                            .map_err(|_| "Unable to withdraw the notification.")
                    }).await;
                }
            },
            action = actions.next() => {
                let Some(action) = action else { return; };
                if action.name() == OPEN_ACTION && action.id().starts_with("new-mail-") {
                    _ = clicked.try_send(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mailstronaut_core::store::analysis::NewMail;

    use super::body;

    fn mail() -> NewMail {
        NewMail {
            id: 1,
            sender: "Maria\nRossi".into(),
            subject: "Bonjour · 世界".into(),
        }
    }

    #[test]
    fn without_a_preview_only_the_number_of_messages_is_revealed() {
        assert_eq!(body(1, None), "1 new message");
        assert_eq!(body(3, None), "3 new messages");
    }

    #[test]
    fn a_preview_names_the_latest_message_with_control_characters_removed() {
        assert_eq!(body(1, Some(&mail())), "MariaRossi\nBonjour · 世界");
        assert_eq!(
            body(2, Some(&mail())),
            "2 new messages, latest from MariaRossi\nBonjour · 世界"
        );
        let long = NewMail {
            id: 1,
            sender: "x".repeat(500),
            subject: "y".repeat(500),
        };
        assert_eq!(body(1, Some(&long)).chars().count(), 321);
    }
}
