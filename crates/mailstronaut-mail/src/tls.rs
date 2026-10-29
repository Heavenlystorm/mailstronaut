//! Validated native roots and explicit additional CA files; no insecure mode.

use std::{fs, io::Read as _, sync::Arc};

use rustls::{ClientConfig, RootCertStore};

use crate::imap::Error;

pub async fn config(files: Vec<String>) -> Result<Arc<ClientConfig>, Error> {
    tokio::task::spawn_blocking(move || {
        if files.len() > 16 {
            return Err(Error::Configuration);
        }
        let mut roots = RootCertStore::empty();
        let native = rustls_native_certs::load_native_certs();
        let _native_counts = roots.add_parsable_certificates(native.certs);
        for certificate in additional(&files)? {
            roots.add(certificate).map_err(|_| Error::TrustStore)?;
        }
        if roots.is_empty() {
            return Err(Error::TrustStore);
        }
        let config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|_| Error::Tls)?
                .with_root_certificates(roots)
                .with_no_client_auth();
        Ok(Arc::new(config))
    })
    .await
    .map_err(|_| Error::TrustStore)?
}

/// Shared bounded extra-CA reader; both transports retain their native-root loaders.
pub fn additional(
    files: &[String],
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, Error> {
    if files.len() > 16 {
        return Err(Error::Configuration);
    }
    let mut certificates = Vec::new();
    for path in files {
        if !fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
            return Err(Error::TrustStore);
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(|_| Error::TrustStore)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::TrustStore)?;
        if bytes.len() > 1024 * 1024 {
            return Err(Error::TrustStore);
        }
        let certs = rustls_pemfile::certs(&mut bytes.as_slice())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::TrustStore)?;
        if certs.is_empty() {
            return Err(Error::TrustStore);
        }
        certificates.extend(certs);
    }
    Ok(certificates)
}
