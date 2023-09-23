use crate::eaves_socket::{RequestCycle, SerializableRequest, SerializableResponse, TrafficLog};
use chrono::Utc;

use std::{net::TcpListener, sync::Arc};

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
type HttpClient = hyper::client::Client<HttpConnector, Body>;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Mode {
    Http,
    Tcp,
}

async fn proxy(
    Extension(client): Extension<HttpClient>,
    Extension(port): Extension<u16>,
    Extension(traffic_log): Extension<Arc<RwLock<TrafficLog>>>,
    mut req: Request<Body>,
) -> Response<Body> {
    let timestamp_in = Utc::now();
    let path = req.uri().path();
    let path_query = req
        .uri()
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(path);

    let uri = format!("http://127.0.0.1:{}{}", port, path_query);

    *req.uri_mut() = Uri::try_from(uri.clone()).unwrap();
    let (in_head, in_body) = req.into_parts();
    let sreq = SerializableRequest {
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
    let in_body_bytes = hyper::body::to_bytes(in_body).await.unwrap();
    let body_in = in_body_bytes.clone().into();
    let request = Request::from_parts(in_head, in_body_bytes.into());
    let response = client.request(request).await.unwrap();

    let (mut out_head, out_body) = response.into_parts();
    let sresp = SerializableResponse {
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
    let out_body_bytes = hyper::body::to_bytes(out_body).await.unwrap();
    let body_out = out_body_bytes.clone().into();
    out_head.headers.remove(hyper::http::header::CONTENT_LENGTH);
    out_head
        .headers
        .remove(hyper::http::header::TRANSFER_ENCODING);
    let response = Response::from_parts(out_head, out_body_bytes.into());
    traffic_log.write().requests.push(RequestCycle {
        timestamp_in,
        head_in: sreq,
        body_in,
        timestamp_out: Utc::now(),
        head_out: sresp,
        body_out,
    });
    response
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
            .fallback(any(proxy))
            .layer(Extension(exposed_port))
            .layer(Extension(http_client))
            .layer(Extension(traffic_log))
            .into_make_service(),
    );
    (http_proxy, proxy_port)
}
