//! Private Secret Service acceptance; never uses the user's session bus.

use mailstronaut_mail::credentials::{Error, Key, Purpose, Store};
use secrecy::{ExposeSecret as _, SecretString};

#[test]
fn rejects_invalid_identities_before_backend_access() {
    for value in ["", "a/b", "a\nb", "a b", &"a".repeat(257)] {
        assert!(matches!(
            Key::new(value, "ref", Purpose::Password),
            Err(Error::Invalid)
        ));
        assert!(matches!(
            Key::new("account", value, Purpose::Password),
            Err(Error::Invalid)
        ));
    }
    assert!(Key::new("account.one", "ref-2", Purpose::Password).is_ok());
    let secret = SecretString::from("SYNTHETIC-NOT-A-REAL-CREDENTIAL");
    assert!(!format!("{secret:?}").contains(secret.expose_secret()));
}

#[tokio::test]
#[ignore = "requires experiments/p02-accounts/credentials.sh private bus and disposable keyring"]
async fn private_secret_service_round_trip_isolation_rotation_and_lock() {
    check_private_environment();
    round_trip().await;
}

#[tokio::test]
#[ignore = "requires the private fixture after its Secret Service has stopped"]
async fn private_missing_service_fails_closed() {
    check_private_environment();
    assert!(matches!(
        Store::connect_foreground().await,
        Err(Error::Unavailable)
    ));
    let quiet = Store::connect_native_background().await;
    assert_eq!(quiet.err(), Some(Error::Unavailable));
    println!("Unavailable Secret Service fails closed without a plaintext backend.");
}

fn check_private_environment() {
    let root = std::env::var("MAILSTRONAUT_SECRET_TEST_ROOT").expect("private test root");
    assert!(root.starts_with('/'));
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("private bus");
    assert!(address.starts_with(&format!("unix:path={root}/runtime/bus")));
    for name in [
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        assert!(
            std::env::var(name)
                .expect("private XDG directory")
                .starts_with(&format!("{root}/"))
        );
    }
}

async fn round_trip() {
    native_collection_lookup_does_not_create_storage().await;
    let store = Store::connect_foreground()
        .await
        .expect("connect private backend");
    let a = Key::new("synthetic-a", "same-ref", Purpose::Password).expect("key");
    let b = Key::new("synthetic-b", "same-ref", Purpose::Password).expect("key");
    let refresh = Key::new("synthetic-a", "same-ref", Purpose::RefreshToken).expect("key");
    assert!(store.get(&a).await.expect("empty").is_none());
    store
        .put(&a, &SecretString::from("synthetic-first"))
        .await
        .expect("save");
    store
        .put(&b, &SecretString::from("synthetic-other-account"))
        .await
        .expect("save second account");
    store
        .put(&refresh, &SecretString::from("synthetic-refresh"))
        .await
        .expect("save refresh token");
    store
        .put(&a, &SecretString::from("synthetic-rotated"))
        .await
        .expect("rotate");
    let reopened = Store::connect_native_background()
        .await
        .expect("quietly reopen the existing native backend");
    assert_eq!(
        reopened
            .get(&a)
            .await
            .expect("read")
            .expect("present")
            .expose_secret(),
        "synthetic-rotated"
    );
    assert_eq!(
        reopened
            .get(&b)
            .await
            .expect("read")
            .expect("present")
            .expose_secret(),
        "synthetic-other-account"
    );
    assert_eq!(
        reopened
            .get(&refresh)
            .await
            .expect("read")
            .expect("present")
            .expose_secret(),
        "synthetic-refresh"
    );
    store.delete(&a).await.expect("delete one exact key");
    assert!(reopened.get(&a).await.expect("read missing").is_none());
    assert!(
        reopened
            .get(&b)
            .await
            .expect("other account unaffected")
            .is_some()
    );
    assert!(
        reopened
            .get(&refresh)
            .await
            .expect("other purpose unaffected")
            .is_some()
    );
    ambiguity_and_lock(&store, &b, &refresh).await;
}

async fn native_collection_lookup_does_not_create_storage() {
    let connection = zbus::Connection::session().await.expect("private bus");
    let proxy: zbus::Proxy<'_> = zbus::proxy::Builder::new(&connection)
        .destination("org.freedesktop.secrets")
        .expect("fixture destination")
        .path("/org/freedesktop/secrets")
        .expect("fixture path")
        .interface("org.freedesktop.Secret.Service")
        .expect("fixture interface")
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .expect("private Secret Service");
    let original: zbus::zvariant::OwnedObjectPath = proxy
        .call("ReadAlias", &("default",))
        .await
        .expect("original default alias");
    let before: Vec<zbus::zvariant::OwnedObjectPath> = proxy
        .get_property("Collections")
        .await
        .expect("collection positive control");
    let empty = zbus::zvariant::ObjectPath::try_from("/").expect("empty alias path");
    proxy
        .call::<_, _, ()>("SetAlias", &("default", &empty))
        .await
        .expect("remove only the private default alias");
    // GNOME can restore its login fallback when another client opens a session.
    // Verify actual collection identities rather than treating removal of the
    // alias as proof that this provider has no usable default collection.
    let resolved: zbus::zvariant::OwnedObjectPath = proxy
        .call("ReadAlias", &("default",))
        .await
        .expect("observe the default alias before quiet lookup");
    let quiet = Store::connect_native_background().await;
    assert!(quiet.is_ok() || matches!(quiet, Err(Error::Unavailable)));
    let after: Vec<zbus::zvariant::OwnedObjectPath> = proxy
        .get_property("Collections")
        .await
        .expect("collections after quiet access");
    let alias: zbus::zvariant::OwnedObjectPath = proxy
        .call("ReadAlias", &("default",))
        .await
        .expect("unchanged missing alias");
    assert_eq!(before, after, "background access must not create storage");
    if quiet.is_ok() {
        assert_ne!(alias.as_str(), "/", "the opened collection must exist");
    }
    println!(
        "Quiet native lookup created no collections; default alias before/after session: {}/{}.",
        resolved.as_str(),
        alias.as_str()
    );
    proxy
        .call::<_, _, ()>("SetAlias", &("default", original))
        .await
        .expect("restore the private alias for foreground acceptance");
}

async fn ambiguity_and_lock(store: &Store, b: &Key, refresh: &Key) {
    let backend = oo7::Keyring::new().await.expect("independent backend");
    let attrs = [
        ("application", include_str!("../../../assets/app-id")),
        ("account", "synthetic-b"),
        ("reference", "same-ref"),
        ("purpose", "password"),
    ];
    backend
        .create_item("Synthetic duplicate", &attrs, "synthetic-duplicate", false)
        .await
        .expect("duplicate fixture");
    assert!(matches!(store.get(b).await, Err(Error::Ambiguous)));
    assert!(matches!(
        store
            .put(b, &SecretString::from("synthetic-replacement"))
            .await,
        Err(Error::Ambiguous)
    ));
    assert!(matches!(store.delete(b).await, Err(Error::Ambiguous)));
    backend.lock().await.expect("lock private backend");
    assert!(matches!(
        Store::connect_native_background().await,
        Err(Error::Locked)
    ));
    assert!(backend.is_locked().await.expect("still locked"));
    assert!(matches!(store.get(refresh).await, Err(Error::Locked)));
    assert!(matches!(
        store
            .put(refresh, &SecretString::from("synthetic-new"))
            .await,
        Err(Error::Locked)
    ));
    assert!(matches!(store.delete(refresh).await, Err(Error::Locked)));
    println!(
        "Private Secret Service: exact lookup, account/purpose isolation, rotation, deletion, ambiguity and locked background access passed."
    );
}
