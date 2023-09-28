use chrono::Utc;

use std::{error::Error, net::TcpListener, sync::Arc};

use axum::{
    body::Body,
    http::{uri::Uri, Request, Response},
    routing::{any, IntoMakeService},
    Extension, Router,
};
use clap::ValueEnum;
use hyper::{client::HttpConnector, server::conn::AddrIncoming};
use parking_lot::RwLock;

use tracing::info;

use shared_types::{RequestCycle, RequestHead, ResponseHead, TrafficLog};

type HttpClient = hyper::client::Client<HttpConnector, Body>;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Mode {
    Http,
    Tcp,
}

#[derive(Debug)]
enum ProxyError {
    UriError,
    BodyError,
    ConnectionRefused,
    OtherRequestError,
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyError::UriError => write!(f, "Failed to parse URI"),
            ProxyError::BodyError => write!(f, "Failed to read body"),
            ProxyError::ConnectionRefused => write!(f, "Forwaded service not running"),
            ProxyError::OtherRequestError => write!(f, "Request error"),
        }
    }
}

impl std::error::Error for ProxyError {}

fn error_response(err: ProxyError) -> Response<Body> {
    let status = match err {
        ProxyError::UriError | ProxyError::BodyError => hyper::StatusCode::BAD_REQUEST,
        ProxyError::ConnectionRefused => hyper::StatusCode::NOT_FOUND,
        ProxyError::OtherRequestError => hyper::StatusCode::INTERNAL_SERVER_ERROR,
    };
    Response::builder()
        .status(status)
        .body(Body::from(err.to_string()))
        .unwrap()
}

async fn proxy(
    client: HttpClient,
    port: u16,
    traffic_log: Arc<RwLock<TrafficLog>>,
    req: Request<Body>,
) -> Result<Response<Body>, ProxyError> {
    let timestamp_in = Utc::now();
    let (request_head, request_body, request) = copy_request(req, port).await?;

    let response = client.request(request).await.map_err(map_hyper_error)?;

    let (response_head, response_body, response) = copy_response(response).await?;
    traffic_log.write().requests.push(RequestCycle {
        timestamp_in,
        request_head,
        request_body,
        timestamp_out: Utc::now(),
        response_head,
        response_body,
    });
    Ok(response)
}

fn map_hyper_error(err: hyper::Error) -> ProxyError {
    if let Some(source) = err.source() {
        if source.to_string().contains("Connection refused") {
            return ProxyError::ConnectionRefused;
        }
    }
    ProxyError::OtherRequestError
}

async fn copy_request(
    mut req: Request<Body>,
    port: u16,
) -> Result<(RequestHead, Vec<u8>, Request<Body>), ProxyError> {
    let path = req.uri().path();
    let path_query = req
        .uri()
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(path);
    let uri = format!("http://127.0.0.1:{}{}", port, path_query);
    *req.uri_mut() = Uri::try_from(uri.clone()).map_err(|_| ProxyError::UriError)?;
    let (in_head, in_body) = req.into_parts();
    let sreq = RequestHead {
        method: in_head.method.as_str().to_string(),
        uri,
        headers: in_head
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    String::from_utf8_lossy(v.as_bytes()).to_string(),
                )
            })
            .collect::<Vec<_>>(),
    };
    let in_body_bytes = hyper::body::to_bytes(in_body)
        .await
        .map_err(|_| ProxyError::BodyError)?;
    let body_in = in_body_bytes.clone().into();
    let request = Request::from_parts(in_head, in_body_bytes.into());
    Ok((sreq, body_in, request))
}
async fn copy_response(
    response: Response<Body>,
) -> Result<(ResponseHead, Vec<u8>, Response<Body>), ProxyError> {
    let (mut out_head, out_body) = response.into_parts();
    let sresp = ResponseHead {
        status: out_head.status.as_u16(),
        headers: out_head
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_owned(),
                    String::from_utf8_lossy(v.as_bytes()).to_string(),
                )
            })
            .collect::<Vec<_>>(),
    };
    let out_body_bytes = hyper::body::to_bytes(out_body)
        .await
        .map_err(|_| ProxyError::BodyError)?;
    let body_out = out_body_bytes.clone().into();
    out_head.headers.remove(hyper::http::header::CONTENT_LENGTH);
    out_head
        .headers
        .remove(hyper::http::header::TRANSFER_ENCODING);
    let response = Response::from_parts(out_head, out_body_bytes.into());
    Ok((sresp, body_out, response))
}

async fn proxy_request(
    Extension(client): Extension<HttpClient>,
    Extension(port): Extension<u16>,
    Extension(traffic_log): Extension<Arc<RwLock<TrafficLog>>>,
    req: Request<Body>,
) -> Response<Body> {
    match proxy(client, port, traffic_log, req).await {
        Ok(response) => response,
        Err(err) => error_response(err),
    }
}

pub fn set_up_eaves_proxy(
    exposed_port: u16,
    http_client: hyper::Client<HttpConnector>,
    traffic_log: Arc<RwLock<TrafficLog>>,
) -> (axum::Server<AddrIncoming, IntoMakeService<Router>>, u16) {
    let proxy_addr = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let proxy_port = proxy_addr.local_addr().unwrap().port();

    info!("Starting proxy at {:?}", &proxy_addr);
    let http_proxy = axum::Server::from_tcp(proxy_addr).unwrap().serve(
        Router::new()
            .fallback(any(proxy_request))
            .layer(Extension(exposed_port))
            .layer(Extension(http_client))
            .layer(Extension(traffic_log))
            .into_make_service(),
    );
    (http_proxy, proxy_port)
}
