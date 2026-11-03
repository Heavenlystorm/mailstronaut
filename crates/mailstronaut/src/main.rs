//! Native Wayland application and display-independent command entry points.

extern crate gpui_kit as gpui;

mod account_actions;
mod accounts;
mod assets;
mod composer;
mod config_cli;
mod config_export;
mod html_reader;
mod html_worker;
mod instance;
#[cfg(test)]
mod lifecycle_tests;
mod link_portal;
mod mail_notify;
mod mail_priority;
mod mail_search;
mod mail_sync;
mod mailbox;
mod onboarding;
#[cfg(test)]
mod portal_fixture;
mod preferences;
mod read_gate;
mod reminders;
mod thread_reader;
mod workspace;

use std::{env, process::ExitCode, sync::Arc};

use gpui::{
    AppContext as _, Bounds, GraphicalEnvironment, WindowBounds, WindowOptions, WindowingModes,
    WindowingRequest, px, size,
};

type SharedStore = Arc<tokio::sync::OnceCell<Arc<mailstronaut_core::worker::Worker>>>;

#[derive(Clone)]
struct Services {
    reminders: gpui::Entity<reminders::Session>,
    sync: gpui::Entity<mail_sync::Session>,
    store: SharedStore,
}

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.as_slice() == ["--internal-project-threads"] {
        return mailstronaut_core::threads::helper::run();
    }
    let demo = arguments.as_slice() == ["--demo"];
    if !demo && let Some(status) = config_cli::dispatch(&arguments) {
        return status;
    }
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        eprintln!("Unable to start mail runtime.");
        return ExitCode::FAILURE;
    };
    let runtime = Arc::new(runtime);
    let config = config_cli::selected_path(&arguments);
    // Claim the session name before creating a toolkit, window or store owner.
    // A second launch only asks the existing process to activate its window.
    let instance = match runtime.block_on(instance::launch(config.as_deref(), demo)) {
        Ok(Some(instance)) => instance,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let mut environment = GraphicalEnvironment::detect();
    if environment
        .wayland_display
        .as_ref()
        .is_none_or(|value| value.is_empty())
    {
        eprintln!("Mailstronaut requires a native Wayland session (WAYLAND_DISPLAY).");
        _ = runtime.block_on(instance.close());
        return ExitCode::FAILURE;
    }
    // Never fall back to X11, even when a session exports DISPLAY as well.
    environment.x11_display = None;
    let ui_runtime = runtime.clone();
    let activations = instance.activations();
    let disconnected = instance.disconnected();
    gpui::platform::linux(WindowingModes::WAYLAND)
        .with_windowing(WindowingRequest::Windowed(environment))
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui::init(cx);
            let services = Services {
                reminders: cx.new(|_| reminders::Session::new(ui_runtime.clone())),
                sync: cx.new(|_| mail_sync::Session::new(ui_runtime.clone())),
                store: Arc::new(tokio::sync::OnceCell::new()),
            };
            let retained_services = services.clone();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(840.)),
                    cx,
                ))),
                app_id: Some(include_str!("../../../assets/app-id").into()),
                ..gpui::component::TitleBar::window_options()
            };
            if gpui::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    workspace::Workspace::new(config, demo, ui_runtime, services, window, cx)
                })
            })
            .is_err()
            {
                eprintln!("Unable to open the native Wayland window.");
                cx.quit();
            }
            cx.on_window_closed(move |cx, _| {
                let _ = &retained_services;
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            listen_for_activation(activations, disconnected, cx);
            cx.activate(true);
        });
    // Keep the session name until the platform and its window-owned work stop.
    _ = runtime.block_on(instance.close());
    ExitCode::SUCCESS
}

/// Forward passive activation on the UI thread and stop if session ownership ends.
fn listen_for_activation(
    activations: async_channel::Receiver<()>,
    disconnected: impl std::future::Future<Output = ()> + 'static,
    cx: &gpui::App,
) {
    let shutdown = activations.clone();
    cx.on_app_quit(move |_| {
        shutdown.close();
        std::future::ready(())
    })
    .detach();
    cx.spawn(async move |cx| {
        tokio::pin!(disconnected);
        loop {
            tokio::select! {
                () = &mut disconnected => {
                    eprintln!("The session bus closed. Mailstronaut is stopping.");
                    cx.update(|cx| cx.quit());
                    break;
                }
                request = activations.recv() => {
                    if request.is_err() { break; }
                    cx.update(|cx| {
                        for handle in cx.windows() {
                            _ = handle.update(cx, |_, window, _| window.activate_window());
                        }
                    });
                }
            }
        }
    })
    .detach();
}

mod cancellable_send {
    // Bounded service updates must not prevent shutdown when a consumer stalls.

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    pub async fn send<T>(sender: &mpsc::Sender<T>, value: T, cancel: &CancellationToken) -> bool {
        tokio::select! {
            biased;
            () = cancel.cancelled() => false,
            result = sender.send(value) => result.is_ok(),
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::Duration;

        use tokio::sync::mpsc;
        use tokio_util::sync::CancellationToken;

        #[tokio::test]
        async fn stalled_updates_do_not_prevent_shutdown() {
            let (sender, mut receiver) = mpsc::channel(1);
            let cancel = CancellationToken::new();
            assert!(super::send(&sender, 1, &cancel).await);
            let pending = tokio::spawn({
                let sender = sender.clone();
                let cancel = cancel.clone();
                async move { super::send(&sender, 2, &cancel).await }
            });
            tokio::task::yield_now().await;
            assert!(
                !pending.is_finished(),
                "the full channel applies backpressure"
            );
            cancel.cancel();
            assert!(
                !tokio::time::timeout(Duration::from_secs(1), pending)
                    .await
                    .expect("shutdown must not hang")
                    .expect("task")
            );
            assert_eq!(receiver.recv().await, Some(1));
            assert!(receiver.try_recv().is_err());
            assert!(!super::send(&sender, 3, &cancel).await);
        }

        #[tokio::test]
        async fn closed_consumer_stops_the_service() {
            let (sender, receiver) = mpsc::channel(1);
            drop(receiver);
            assert!(!super::send(&sender, 1, &CancellationToken::new()).await);
        }
    }
}
