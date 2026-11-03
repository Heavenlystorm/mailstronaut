//! Single-session ownership and bounded, passive window activation.
//!
//! The store worker's existing profile lock remains the independent protection
//! against another bus/process opening the same data. No mail is opened here.

use std::{collections::HashMap, os::unix::ffi::OsStrExt as _, path::Path, time::Duration};

use async_channel::{Receiver, Sender, TrySendError};
use zbus::{
    Connection, fdo,
    zvariant::{OwnedValue, Value},
};

const NAME: &str = include_str!("../../../assets/app-id");
const PATH: &str = "/io/github/eduardz1/Mailstronaut";
const INTERFACE: &str = "org.freedesktop.Application";
const PROFILE_KEY: &str = "io.github.eduardz1.Mailstronaut.profile";
const BACKGROUND_KEY: &str = "io.github.eduardz1.Mailstronaut.background";
const TIMEOUT: Duration = Duration::from_secs(10);

/// Held for the entire foreground application lifetime.
pub struct Primary {
    connection: Connection,
    activations: Receiver<()>,
}

impl Primary {
    /// One coalescing queue; activation never loads mail or changes Seen.
    pub fn activations(&self) -> Receiver<()> {
        self.activations.clone()
    }

    /// Losing the session bus retires this owner instead of leaving it invisible
    /// to future launches. The separate store profile lock is still authoritative.
    pub fn disconnected(&self) -> impl std::future::Future<Output = ()> + 'static {
        let connection = self.connection.clone();
        async move { connection.closed().await }
    }

    /// Refuse new requests and release ownership after the UI has stopped.
    pub async fn close(self) -> zbus::Result<()> {
        self.activations.close();
        self.connection.release_name(NAME).await?;
        self.connection.close().await
    }
}

struct Application {
    profile: Vec<u8>,
    activations: Sender<()>,
}

#[zbus::interface(name = "org.freedesktop.Application")]
impl Application {
    fn activate(&self, mut platform_data: HashMap<String, OwnedValue>) -> fdo::Result<()> {
        // A normal desktop Activate has no profile hint. Our executable adds one
        // so --config/--demo cannot silently activate a different declaration.
        if let Some(profile) = platform_data.remove(PROFILE_KEY) {
            let profile = Vec::<u8>::try_from(profile)
                .map_err(|_| fdo::Error::InvalidArgs("Invalid profile identity.".into()))?;
            if profile != self.profile {
                return Err(fdo::Error::AccessDenied(
                    "Mailstronaut is already running with another configuration.".into(),
                ));
            }
        }
        if let Some(background) = platform_data.remove(BACKGROUND_KEY) {
            let background = bool::try_from(background)
                .map_err(|_| fdo::Error::InvalidArgs("Invalid launch mode.".into()))?;
            if background {
                return Ok(());
            }
        }
        match self.activations.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => Ok(()),
            Err(TrySendError::Closed(())) => Err(fdo::Error::Failed(
                "Mailstronaut is shutting down. Try again.".into(),
            )),
        }
    }
}

/// Claim ownership, or forward this launch without starting another UI/backend.
pub async fn launch(
    config: Option<&Path>,
    demo: bool,
    background: bool,
) -> Result<Option<Primary>, &'static str> {
    let config = config.ok_or("Unable to locate the native configuration.")?;
    let absolute =
        std::path::absolute(config).map_err(|_| "Unable to resolve configuration path.")?;
    // Source identity is its declared location, not the current file target:
    // Home Manager may replace a symlink with a new generation while we run.
    let mut profile = vec![u8::from(demo)];
    profile.extend_from_slice(absolute.as_os_str().as_bytes());
    launch_profile_mode(profile, TIMEOUT, background).await
}

#[cfg(test)]
async fn launch_profile(
    profile: Vec<u8>,
    timeout: Duration,
) -> Result<Option<Primary>, &'static str> {
    launch_profile_mode(profile, timeout, false).await
}

async fn launch_profile_mode(
    profile: Vec<u8>,
    timeout: Duration,
    background: bool,
) -> Result<Option<Primary>, &'static str> {
    tokio::time::timeout(timeout, async {
        let (sender, activations) = async_channel::bounded(1);
        let connection = zbus::connection::Builder::session()
            .map_err(|_| "The session bus is unavailable. Mailstronaut was not started.")?
            .method_timeout(timeout)
            .max_queued(16)
            .serve_at(
                PATH,
                Application {
                    profile: profile.clone(),
                    activations: sender,
                },
            )
            .map_err(|_| "Unable to register application activation.")?
            .build()
            .await
            .map_err(|_| "The session bus is unavailable. Mailstronaut was not started.")?;
        match connection
            .request_name_with_flags(NAME, fdo::RequestNameFlags::DoNotQueue.into())
            .await
        {
            Ok(fdo::RequestNameReply::PrimaryOwner | fdo::RequestNameReply::AlreadyOwner) => {
                Ok(Some(Primary {
                    connection,
                    activations,
                }))
            }
            Err(zbus::Error::NameTaken) => {
                let profile = OwnedValue::try_from(Value::from(profile))
                    .map_err(|_| "Unable to encode activation request.")?;
                let platform_data = HashMap::from([(PROFILE_KEY.to_owned(), profile), (BACKGROUND_KEY.to_owned(), OwnedValue::from(background))]);
                connection
                    .call_method(Some(NAME), PATH, Some(INTERFACE), "Activate", &(platform_data,))
                    .await
                    .and_then(|reply| reply.body().deserialize::<()>())
                    .map_err(|_| {
                        "Mailstronaut is already running. Activation failed or its configuration differs."
                    })?;
                Ok(None)
            }
            _ => Err("Unable to claim application ownership. Mailstronaut was not started."),
        }
    })
    .await
    .map_err(|_| "Application activation timed out. No second instance was started.")?
}

#[cfg(test)]
mod tests;
