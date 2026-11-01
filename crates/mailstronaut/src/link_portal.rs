//! Explicit web-link handoff, with no URL-bearing portal diagnostics or fallback.

use std::time::Duration;

use mailstronaut_render::Link;
use tokio_util::sync::CancellationToken;

pub async fn open(link: Link, cancel: &CancellationToken) -> Result<(), &'static str> {
    open_with_timeout(link, cancel, Duration::from_secs(60)).await
}

async fn open_with_timeout(
    link: Link,
    cancel: &CancellationToken,
    timeout: Duration,
) -> Result<(), &'static str> {
    let uri = ashpd::Uri::parse(link.as_str()).map_err(|_| "Invalid web link.")?;
    cancel
        .run_until_cancelled(tokio::time::timeout(timeout, async {
            let request = ashpd::desktop::open_uri::OpenFileRequest::default()
                .send_uri(&uri)
                .await
                .map_err(|_| "Browser portal is unavailable or refused the request.")?;
            request
                .response()
                .map_err(|_| "Opening the link was cancelled or refused.")
        }))
        .await
        .ok_or("Opening the link was cancelled.")?
        .map_err(|_| "Browser portal did not respond. Try again.")?
}

#[cfg(test)]
mod tests;
