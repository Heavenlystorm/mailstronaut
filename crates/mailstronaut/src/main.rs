//! Native Wayland application and display-independent command entry points.

extern crate gpui_kit as gpui;

mod account_actions;
mod accounts;
mod composer;
mod config_cli;
mod config_export;
mod html_reader;
mod html_worker;
mod link_portal;
mod mail_search;
mod mail_sync;
mod mailbox;
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

fn main() -> ExitCode {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.as_slice() == ["--internal-project-threads"] {
        return mailstronaut_core::threads::helper::run();
    }
    let demo = arguments.as_slice() == ["--demo"];
    if !demo && let Some(status) = config_cli::dispatch(&arguments) {
        return status;
    }
    let mut environment = GraphicalEnvironment::detect();
    if environment
        .wayland_display
        .as_ref()
        .is_none_or(|value| value.is_empty())
    {
        eprintln!("Mailstronaut requires a native Wayland session (WAYLAND_DISPLAY).");
        return ExitCode::FAILURE;
    }
    // Never fall back to X11, even when a session exports DISPLAY as well.
    environment.x11_display = None;
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        eprintln!("Unable to start mail runtime.");
        return ExitCode::FAILURE;
    };
    let runtime = Arc::new(runtime);
    let config = config_cli::selected_path(&arguments);
    gpui::platform::linux(WindowingModes::WAYLAND)
        .with_windowing(WindowingRequest::Windowed(environment))
        .with_assets(gpui::assets::Assets)
        .run(move |cx| {
            gpui::init(cx);
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
                cx.new(|cx| workspace::Workspace::new(config, demo, runtime, window, cx))
            })
            .is_err()
            {
                eprintln!("Unable to open the native Wayland window.");
                cx.quit();
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    ExitCode::SUCCESS
}
