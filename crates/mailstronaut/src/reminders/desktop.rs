//! Standard portal notifications, owned and dropped exclusively on Tokio.
use ashpd::desktop::notification::{Button, Notification, NotificationProxy};
use futures_util::StreamExt as _;
use mailstronaut_core::{store::reminders::Reminder, worker};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Open,
    Again,
    Dismiss,
}
#[derive(Clone)]
pub struct Action {
    pub notification: String,
    pub generation: i64,
    pub kind: Kind,
}
impl core::fmt::Debug for Action {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ReminderAction(<private target>)")
    }
}
type Reply = oneshot::Sender<Result<(), &'static str>>;
enum Request {
    Add(String, Box<Notification>, Reply),
    Remove(String, Reply),
}
pub struct Client {
    requests: mpsc::Sender<Request>,
    pub events: async_channel::Receiver<Action>,
    listener: tokio_util::task::AbortOnDropHandle<()>,
}
impl Client {
    pub async fn connect() -> Result<Arc<Self>, &'static str> {
        let (requests, receiver) = mpsc::channel(4);
        let (sender, events) = async_channel::bounded(16);
        let (ready, started) = oneshot::channel();
        let listener =
            tokio_util::task::AbortOnDropHandle::new(tokio::spawn(serve(receiver, sender, ready)));
        started
            .await
            .map_err(|_| "Desktop notification worker stopped.")??;
        Ok(Arc::new(Self {
            requests,
            events,
            listener,
        }))
    }
    pub fn healthy(&self) -> bool {
        !self.listener.is_finished()
    }
    pub async fn withdraw(&self, record: &Reminder) -> Result<(), &'static str> {
        let (reply, result) = oneshot::channel();
        self.requests
            .send(Request::Remove(record.notification.clone(), reply))
            .await
            .map_err(|_| "Desktop notification worker stopped.")?;
        result
            .await
            .map_err(|_| "Desktop notification worker stopped.")?
    }
    pub async fn deliver(
        &self,
        record: &Reminder,
        store: &worker::Handle,
        preview: bool,
    ) -> Result<(), &'static str> {
        let body = if preview {
            let projection = store
                .message(record.account.clone(), record.message)
                .await
                .map_err(|_| "Reminder message unavailable.")?;
            let title = projection
                .headers
                .subject
                .as_deref()
                .unwrap_or("(no subject)")
                .chars()
                .filter(|c| !c.is_control())
                .take(160)
                .collect::<String>();
            let preview = projection
                .preview
                .as_ref()
                .map_or("", |preview| preview.text.as_str())
                .chars()
                .filter(|c| !c.is_control())
                .take(320)
                .collect::<String>();
            format!("{title}\n{preview}")
        } else {
            "You asked to revisit an email. Open Mailstronaut to view it.".into()
        };
        let notification = Notification::new("Mail reminder")
            .body(body.as_str())
            .default_action("open")
            .default_action_target(record.generation)
            .button(Button::new("Remind in one hour", "again").target(record.generation))
            .button(Button::new("Dismiss", "dismiss").target(record.generation));
        let (reply, result) = oneshot::channel();
        self.requests
            .send(Request::Add(
                record.notification.clone(),
                Box::new(notification),
                reply,
            ))
            .await
            .map_err(|_| "Desktop notification worker stopped.")?;
        result
            .await
            .map_err(|_| "Desktop notification worker stopped.")?
    }
}
async fn serve(
    mut receiver: mpsc::Receiver<Request>,
    events: async_channel::Sender<Action>,
    ready: oneshot::Sender<Result<(), &'static str>>,
) {
    let Ok(proxy) = NotificationProxy::new().await else {
        let _sent = ready.send(Err("Desktop notification portal unavailable."));
        return;
    };
    let Ok(actions) = proxy.receive_action_invoked().await else {
        let _sent = ready.send(Err("Desktop notification actions unavailable."));
        return;
    };
    futures_util::pin_mut!(actions);
    let _sent = ready.send(Ok(()));
    loop {
        tokio::select! {
            request = receiver.recv() => match request {
                None => break,
                Some(Request::Add(id, notification, reply)) => {
                    if !reply.is_closed() {
                        let result = bounded(async {
                            proxy.add_notification(&id, *notification).await
                                .map_err(|_| "The desktop did not accept the reminder notification.")
                        }).await;
                        let _sent = reply.send(result);
                    }
                }
                Some(Request::Remove(id, reply)) => {
                    if !reply.is_closed() {
                        let result = bounded(async {
                            proxy.remove_notification(&id).await
                                .map_err(|_| "Unable to withdraw the previous reminder notification.")
                        }).await;
                        let _sent = reply.send(result);
                    }
                }
            },
            action = actions.next() => {
                let Some(action) = action else { break; };
                let kind = match action.name() {
                    "open" => Kind::Open,
                    "again" => Kind::Again,
                    "dismiss" => Kind::Dismiss,
                    _ => continue,
                };
                if action.id().len() > 128 || action.parameter().len() != 1 {
                    continue;
                }
                let Some(generation) = action.parameter().first()
                    .and_then(|value| value.downcast_ref::<i64>().ok())
                else { continue; };
                if generation <= 0 { continue; }
                let _sent = events.try_send(Action {
                    notification: action.id().into(), generation, kind,
                });
            }
        }
    }
}
pub async fn bounded<T>(
    future: impl core::future::Future<Output = Result<T, &'static str>>,
) -> Result<T, &'static str> {
    tokio::time::timeout(core::time::Duration::from_secs(3), future)
        .await
        .map_err(|_| "Desktop notification request timed out.")?
}
