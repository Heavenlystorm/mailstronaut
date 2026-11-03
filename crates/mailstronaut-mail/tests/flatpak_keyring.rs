//! Private Flatpak walkthrough helper. It writes one synthetic password into the file keyring that a
//! sandboxed application opens with the Secret portal's key, so the sandboxed account can authenticate
//! without any host keyring. It runs only when a private walkthrough asks for it.

use std::{collections::HashMap, path::PathBuf};

use oo7::file::UnlockedKeyring;

const ACCOUNT: &str = "flatpak-attach";
const REFERENCE: &str = "flatpak-attach-password";

#[tokio::test]
#[ignore = "private Flatpak walkthrough helper only"]
async fn seed_portal_keyring() {
    let keyring =
        PathBuf::from(std::env::var_os("MAILSTRONAUT_SEED_KEYRING").expect("keyring path"));
    let secret = std::fs::read(std::env::var_os("MAILSTRONAUT_SEED_SECRET").expect("secret file"))
        .expect("portal secret");
    let password = std::env::var("MAILSTRONAUT_SEED_PASSWORD").expect("synthetic password");
    std::fs::create_dir_all(keyring.parent().expect("keyring directory"))
        .expect("keyring directory");
    // The same attributes the application uses for a password credential.
    let application = include_str!("../../../assets/app-id");
    let attributes = HashMap::from([
        ("application", application),
        ("account", ACCOUNT),
        ("reference", REFERENCE),
        ("purpose", "password"),
    ]);
    let unlocked = UnlockedKeyring::load(&keyring, oo7::Secret::from(secret))
        .await
        .expect("portal keyring");
    unlocked
        .create_item(
            "Synthetic Flatpak password",
            &attributes,
            oo7::Secret::text(password),
            true,
        )
        .await
        .expect("seeded password");
}
