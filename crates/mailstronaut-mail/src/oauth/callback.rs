//! Short-lived loopback callback using Hyper's bounded HTTP/1 server.

use core::{convert::Infallible, time::Duration};
use std::sync::{Arc, Mutex};

use http_body_util::Full;
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use oauth2::CsrfToken;
use tokio::net::TcpListener;

use super::Error;

type ResultSlot = Arc<Mutex<Option<Result<String, Error>>>>;

pub(super) async fn receive(
    listener: &TcpListener,
    authority: &str,
    state: &CsrfToken,
) -> Result<String, Error> {
    for _ in 0..16 {
        let (socket, peer) = listener.accept().await.map_err(|_| Error::Callback)?;
        if !peer.ip().is_loopback() {
            return Err(Error::Callback);
        }
        let slot: ResultSlot = Arc::default();
        let result = slot.clone();
        let service = service_fn(|request| {
            let outcome = parse(&request, authority, state);
            let accepted = outcome.is_some();
            if let Some(outcome) = outcome
                && let Ok(mut slot) = result.lock()
            {
                *slot = Some(outcome);
            }
            let mut response = Response::new(Full::new(Bytes::from_static(if accepted {
                b"Authorization response received. Return to Mailstronaut."
            } else {
                b"Callback rejected. Return to the authorization browser tab."
            })));
            *response.status_mut() = if accepted {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            };
            for (name, value) in [
                ("content-type", "text/plain; charset=utf-8"),
                ("cache-control", "no-store"),
                ("referrer-policy", "no-referrer"),
                (
                    "content-security-policy",
                    "default-src 'none'; frame-ancestors 'none'",
                ),
            ] {
                response
                    .headers_mut()
                    .insert(name, hyper::header::HeaderValue::from_static(value));
            }
            async { Ok::<_, Infallible>(response) }
        });
        let mut builder = hyper::server::conn::http1::Builder::new();
        builder
            .keep_alive(false)
            .max_headers(16)
            .max_buf_size(8192)
            .timer(TokioTimer::new())
            .header_read_timeout(Duration::from_secs(2));
        let _connection = tokio::time::timeout(
            Duration::from_secs(3),
            builder.serve_connection(TokioIo::new(socket), service),
        )
        .await;
        let result = slot.lock().map_err(|_| Error::Callback)?.take();
        if let Some(result) = result {
            return result;
        }
    }
    Err(Error::Callback)
}
fn parse(
    request: &Request<Incoming>,
    authority: &str,
    state: &CsrfToken,
) -> Option<Result<String, Error>> {
    if request.method() != hyper::Method::GET
        || request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || request.uri().path() != "/oauth2/callback"
        || request.uri().to_string().len() > 4096
    {
        return None;
    }
    let mut hosts = request.headers().get_all(hyper::header::HOST).iter();
    if hosts.next()?.to_str().ok()? != authority || hosts.next().is_some() {
        return None;
    }
    let query = request.uri().query()?;
    let (mut code, mut received, mut error) = (None, None, None);
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        let field = match key.as_ref() {
            "code" => &mut code,
            "state" => &mut received,
            "error" => &mut error,
            _ => continue,
        };
        if field.replace(value.into_owned()).is_some() {
            return None;
        }
    }
    let received = CsrfToken::new(received?);
    if &received != state {
        return None;
    }
    match (code, error) {
        (None, Some(error)) if error == "access_denied" => Some(Err(Error::Denied)),
        (None, Some(_)) => Some(Err(Error::Callback)),
        (Some(code), None) if !code.is_empty() && !code.chars().any(char::is_control) => {
            Some(Ok(code))
        }
        _ => None,
    }
}
