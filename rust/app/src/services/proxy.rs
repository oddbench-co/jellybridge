use std::convert::Infallible;
use std::error::Error as StdError;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use futures_util::{StreamExt, TryStreamExt};
use http_body_util::{BodyExt, Empty, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, HeaderMap, HeaderName,
    IF_RANGE, LAST_MODIFIED, LOCATION, RANGE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use reqwest::redirect::Policy;
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use url::Url;

const MAX_REDIRECTS: usize = 5;
const MAX_ACTIVE_UPSTREAM_STREAMS: usize = 8;
const FORWARDED_REQUEST_HEADERS: &[HeaderName] = &[RANGE, IF_RANGE];
const FORWARDED_RESPONSE_HEADERS: &[HeaderName] = &[
    ACCEPT_RANGES,
    CONTENT_LENGTH,
    CONTENT_RANGE,
    CONTENT_TYPE,
    ETAG,
    LAST_MODIFIED,
];

type BoxError = Box<dyn StdError + Send + Sync>;
type ProxyBody = UnsyncBoxBody<Bytes, BoxError>;

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("playback proxy only accepts GET and HEAD")]
    MethodNotAllowed,
    #[error("upstream redirect is invalid")]
    InvalidRedirect,
    #[error("upstream redirect changed origin")]
    CrossOriginRedirect,
    #[error("upstream redirect downgraded HTTPS")]
    HttpsDowngrade,
    #[error("too many upstream redirects")]
    TooManyRedirects,
    #[error("playback proxy session ended")]
    Cancelled,
    #[error("upstream request failed")]
    Upstream(#[source] reqwest::Error),
    #[error("proxy listener failed")]
    Listener(#[source] std::io::Error),
    #[error("operating-system randomness failed")]
    Random(#[source] getrandom::Error),
}

#[derive(Clone)]
pub struct PlaybackOrigin {
    pub url: Url,
    /// Origin-only headers such as Jellyfin's authorization header. They are
    /// never copied to redirected origins or reflected to the local caller.
    pub headers: reqwest::header::HeaderMap,
}

pub struct ProxySession {
    local_url: Url,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl ProxySession {
    pub fn local_url(&self) -> &Url {
        &self.local_url
    }

    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

#[derive(Clone)]
struct ProxyState {
    client: reqwest::Client,
    origin: PlaybackOrigin,
    secret_path: Arc<str>,
    cancel: CancellationToken,
    upstream_limit: Arc<Semaphore>,
}

pub async fn start_proxy(
    client: reqwest::Client,
    origin: PlaybackOrigin,
) -> Result<ProxySession, ProxyError> {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .map_err(ProxyError::Listener)?;
    let address = listener.local_addr().map_err(ProxyError::Listener)?;
    let secret_path: Arc<str> = format!("/play/{}", random_hex_256()?).into();
    let cancel = CancellationToken::new();
    let state = ProxyState {
        client,
        origin,
        secret_path: secret_path.clone(),
        cancel: cancel.clone(),
        upstream_limit: Arc::new(Semaphore::new(MAX_ACTIVE_UPSTREAM_STREAMS)),
    };
    let accept_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            let accepted = tokio::select! {
                _ = accept_cancel.cancelled() => break,
                accepted = listener.accept() => accepted,
                Some(_) = connections.join_next(), if !connections.is_empty() => continue,
            };
            let Ok((stream, peer)) = accepted else { break };
            if !peer.ip().is_loopback() {
                continue;
            }
            let connection_state = state.clone();
            connections.spawn(async move {
                let service =
                    service_fn(move |request| handle_request(request, connection_state.clone()));
                let _ = http1::Builder::new()
                    .keep_alive(true)
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
        connections.abort_all();
        while connections.join_next().await.is_some() {}
    });

    let local_url =
        Url::parse(&format!("http://{address}{secret_path}")).expect("loopback proxy URL is valid");
    Ok(ProxySession {
        local_url,
        cancel,
        task,
    })
}

async fn handle_request(
    request: Request<Incoming>,
    state: ProxyState,
) -> Result<Response<ProxyBody>, Infallible> {
    if state.cancel.is_cancelled() {
        return Ok(text_response(StatusCode::GONE, "playback session ended"));
    }
    if request.uri().path() != state.secret_path.as_ref() {
        return Ok(text_response(StatusCode::NOT_FOUND, "not found"));
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return Ok(text_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "method not allowed",
        ));
    }

    let permit = tokio::select! {
        _ = state.cancel.cancelled() => {
            return Ok(text_response(StatusCode::GONE, "playback session ended"));
        }
        permit = state.upstream_limit.clone().acquire_owned() => {
            match permit {
                Ok(permit) => permit,
                Err(_) => return Ok(text_response(StatusCode::GONE, "playback session ended")),
            }
        }
    };

    match fetch_upstream(request.method(), request.headers(), &state, permit).await {
        Ok(response) => Ok(response),
        Err(error) => {
            tracing::warn!(error = %error, "playback proxy request rejected");
            let status = match error {
                ProxyError::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
                ProxyError::InvalidRedirect
                | ProxyError::CrossOriginRedirect
                | ProxyError::HttpsDowngrade
                | ProxyError::TooManyRedirects => StatusCode::BAD_GATEWAY,
                ProxyError::Cancelled => StatusCode::GONE,
                _ => StatusCode::BAD_GATEWAY,
            };
            Ok(text_response(status, "upstream playback request failed"))
        }
    }
}

async fn fetch_upstream(
    method: &Method,
    incoming_headers: &HeaderMap,
    state: &ProxyState,
    permit: OwnedSemaphorePermit,
) -> Result<Response<ProxyBody>, ProxyError> {
    let method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|_| ProxyError::MethodNotAllowed)?;
    let mut url = state.origin.url.clone();

    for redirect_count in 0..=MAX_REDIRECTS {
        let mut builder = state.client.request(method.clone(), url.clone());
        for (name, value) in &state.origin.headers {
            builder = builder.header(name, value);
        }
        for name in FORWARDED_REQUEST_HEADERS {
            if let Some(value) = incoming_headers.get(name) {
                builder = builder.header(name, value);
            }
        }

        let response = tokio::select! {
            _ = state.cancel.cancelled() => return Err(ProxyError::Cancelled),
            response = builder.send() => response.map_err(ProxyError::Upstream)?,
        };
        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(ProxyError::TooManyRedirects);
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(ProxyError::InvalidRedirect)?;
            let next = url
                .join(location)
                .map_err(|_| ProxyError::InvalidRedirect)?;
            validate_redirect(&state.origin.url, &next)?;
            url = next;
            continue;
        }

        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let mut result = Response::builder().status(status);
        for name in FORWARDED_RESPONSE_HEADERS {
            if let Some(value) = response.headers().get(name) {
                result = result.header(name, value);
            }
        }

        if method == reqwest::Method::HEAD {
            return Ok(result.body(empty_body()).expect("valid proxy response"));
        }

        let cancel = state.cancel.clone();
        let stream = response
            .bytes_stream()
            .take_until(cancel.cancelled_owned())
            .map(move |item| {
                let _permit = &permit;
                item
            })
            .map_ok(Frame::data)
            .map_err(|error| -> BoxError { Box::new(error) });
        let body = BodyExt::boxed_unsync(StreamBody::new(stream));
        return Ok(result.body(body).expect("valid proxy response"));
    }

    Err(ProxyError::TooManyRedirects)
}

fn validate_redirect(origin: &Url, next: &Url) -> Result<(), ProxyError> {
    if origin.scheme() == "https" && next.scheme() != "https" {
        return Err(ProxyError::HttpsDowngrade);
    }
    if canonical_origin(origin) != canonical_origin(next) {
        return Err(ProxyError::CrossOriginRedirect);
    }
    Ok(())
}

fn canonical_origin(url: &Url) -> (String, String, Option<u16>) {
    (
        url.scheme().to_ascii_lowercase(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default(),
    )
}

fn random_hex_256() -> Result<String, ProxyError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(ProxyError::Random)?;
    let mut result = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(result, "{byte:02x}");
    }
    Ok(result)
}

fn empty_body() -> ProxyBody {
    Empty::<Bytes>::new()
        .map_err(|never| -> BoxError { match never {} })
        .boxed_unsync()
}

fn text_response(status: StatusCode, message: &'static str) -> Response<ProxyBody> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(
            Full::new(Bytes::from_static(message.as_bytes()))
                .map_err(|never| -> BoxError { match never {} })
                .boxed_unsync(),
        )
        .expect("valid text response")
}

/// Builds one client for one playback origin.
pub fn strict_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .redirect(Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(30))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::AUTHORIZATION;

    async fn test_upstream() -> (SocketAddr, JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let service = service_fn(|request: Request<Incoming>| async move {
                        let authorized = request.headers().get(AUTHORIZATION)
                            == Some(&hyper::header::HeaderValue::from_static(
                                "MediaBrowser token",
                            ));
                        if !authorized {
                            return Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::UNAUTHORIZED)
                                    .body(Full::new(Bytes::new()))
                                    .unwrap(),
                            );
                        }
                        if request.uri().path() == "/redirect" {
                            return Ok(Response::builder()
                                .status(StatusCode::FOUND)
                                .header(LOCATION, "/media")
                                .body(Full::new(Bytes::new()))
                                .unwrap());
                        }
                        let range = request.headers().get(RANGE).and_then(|v| v.to_str().ok());
                        let (status, bytes, content_range) = match range {
                            Some("bytes=2-5") => (
                                StatusCode::PARTIAL_CONTENT,
                                Bytes::from_static(b"2345"),
                                Some("bytes 2-5/10"),
                            ),
                            _ => (StatusCode::OK, Bytes::from_static(b"0123456789"), None),
                        };
                        let mut response = Response::builder()
                            .status(status)
                            .header(ACCEPT_RANGES, "bytes")
                            .header(CONTENT_LENGTH, bytes.len().to_string());
                        if let Some(value) = content_range {
                            response = response.header(CONTENT_RANGE, value);
                        }
                        Ok(response.body(Full::new(bytes)).unwrap())
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        (address, task)
    }

    fn origin(address: SocketAddr, path: &str) -> PlaybackOrigin {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_static("MediaBrowser token"),
        );
        PlaybackOrigin {
            url: Url::parse(&format!("http://{address}{path}")).unwrap(),
            headers,
        }
    }

    #[tokio::test]
    async fn proxies_ranges_without_exposing_credentials() {
        let (address, upstream) = test_upstream().await;
        let proxy = start_proxy(strict_client().unwrap(), origin(address, "/media"))
            .await
            .unwrap();
        assert!(!proxy.local_url().as_str().contains("token"));

        let response = reqwest::Client::new()
            .get(proxy.local_url().clone())
            .header(RANGE, "bytes=2-5")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response.headers().get(CONTENT_RANGE).unwrap(),
            "bytes 2-5/10"
        );
        assert_eq!(response.bytes().await.unwrap(), Bytes::from_static(b"2345"));

        proxy.shutdown().await;
        upstream.abort();
    }

    #[tokio::test]
    async fn proxies_head_without_a_response_body() {
        let (address, upstream) = test_upstream().await;
        let proxy = start_proxy(strict_client().unwrap(), origin(address, "/media"))
            .await
            .unwrap();

        let response = reqwest::Client::new()
            .head(proxy.local_url().clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get(CONTENT_LENGTH).unwrap(), "10");
        assert!(response.bytes().await.unwrap().is_empty());

        proxy.shutdown().await;
        upstream.abort();
    }

    #[tokio::test]
    async fn shutdown_aborts_a_connection_waiting_for_upstream_headers() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_sender, accepted_receiver) = tokio::sync::oneshot::channel();
        let upstream = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            let _ = accepted_sender.send(());
            std::future::pending::<()>().await;
        });
        let proxy = start_proxy(strict_client().unwrap(), origin(address, "/media"))
            .await
            .unwrap();
        let local_url = proxy.local_url().clone();
        let request =
            tokio::spawn(async move { reqwest::Client::new().get(local_url).send().await });

        tokio::time::timeout(std::time::Duration::from_secs(1), accepted_receiver)
            .await
            .expect("proxy should connect to the upstream")
            .expect("upstream should report the accepted connection");
        tokio::time::timeout(std::time::Duration::from_secs(1), proxy.shutdown())
            .await
            .expect("proxy shutdown should not wait for upstream headers");
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), request)
            .await
            .expect("local request should finish after proxy shutdown")
            .expect("request task should not panic");
        assert!(result.is_err());
        upstream.abort();
    }

    #[tokio::test]
    async fn follows_only_same_origin_redirects() {
        let (address, upstream) = test_upstream().await;
        let proxy = start_proxy(strict_client().unwrap(), origin(address, "/redirect"))
            .await
            .unwrap();
        let response = reqwest::Client::new()
            .get(proxy.local_url().clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.bytes().await.unwrap(),
            Bytes::from_static(b"0123456789")
        );
        proxy.shutdown().await;
        upstream.abort();
    }

    #[test]
    fn rejects_cross_origin_and_https_downgrade_redirects() {
        let origin = Url::parse("https://jellyfin.example:443/media").unwrap();
        let downgrade = Url::parse("http://jellyfin.example/media").unwrap();
        let cross_origin = Url::parse("https://cdn.example/media").unwrap();
        assert!(matches!(
            validate_redirect(&origin, &downgrade),
            Err(ProxyError::HttpsDowngrade)
        ));
        assert!(matches!(
            validate_redirect(&origin, &cross_origin),
            Err(ProxyError::CrossOriginRedirect)
        ));
    }
}
