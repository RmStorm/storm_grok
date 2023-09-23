use crate::eaves_socket::{SerializableRequest, SerializableResponse};
use chrono::Utc;
use include_dir::{include_dir, Dir};

use std::{collections::HashMap, io::ErrorKind, net::TcpListener, sync::Arc};

use axum::{
    body::{Body, StreamBody},
    http::header,
    http::{uri::Uri, Request, Response},
    response::IntoResponse,
    routing::{any, get},
    Extension, Json, Router,
};
use hyper::client::HttpConnector;
use parking_lot::RwLock;
use tokio_util::io::ReaderStream;
use tracing::info;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{filter::LevelFilter, fmt, layer::SubscriberExt, EnvFilter};
static DS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../dist");

mod client;
mod dev_stuff;
mod eaves_socket;

use clap::Parser;
use clap::ValueEnum;

type HttpClient = hyper::client::Client<HttpConnector, Body>;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Mode {
    Http,
    Tcp,
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    /// What mode to run the program in
    #[clap(value_enum, value_parser)]
    mode: Mode,
    /// Port to forward to
    #[clap(value_parser = clap::value_parser!(u16).range(1..65536))]
    port: u16,
    #[clap(long, short, action)]
    dev: bool,
}

impl From<Mode> for [u8; 1] {
    fn from(mode: Mode) -> [u8; 1] {
        match mode {
            Mode::Tcp => [b't'],
            Mode::Http => [b'h'],
        }
    }
}

async fn get_traffic_log(
    Extension(traffic_log): Extension<Arc<RwLock<eaves_socket::TrafficLog>>>,
) -> Json<eaves_socket::TrafficLog> {
    Json(traffic_log.read().clone())
}

type FrontendBytes = Extension<HashMap<&'static str, &'static [u8]>>;

async fn html(Extension(frontend_bytes): FrontendBytes) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html")],
        StreamBody::new(ReaderStream::new(*frontend_bytes.get("html").unwrap())),
    )
}

async fn js(Extension(frontend_bytes): FrontendBytes) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript")],
        StreamBody::new(ReaderStream::new(*frontend_bytes.get("js").unwrap())),
    )
}

async fn wasm(Extension(frontend_bytes): FrontendBytes) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/wasm")],
        StreamBody::new(ReaderStream::new(*frontend_bytes.get("wasm").unwrap())),
    )
}

fn listen_available_port() -> TcpListener {
    for port in 4040..65535 {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => return l,
            Err(error) => match error.kind() {
                ErrorKind::AddrInUse => {}
                other_error => panic!(
                    "Encountered errr while setting up tcp server: {:?}",
                    other_error
                ),
            },
        }
    }
    panic!("No ports available")
}

async fn hello() -> &'static str {
    "Some response eeeyyy\n"
}

async fn proxy(
    Extension(client): Extension<HttpClient>,
    Extension(port): Extension<u16>,
    Extension(traffic_log): Extension<Arc<RwLock<eaves_socket::TrafficLog>>>,
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
    traffic_log
        .write()
        .requests
        .push(eaves_socket::RequestCycle {
            timestamp_in,
            head_in: sreq,
            body_in,
            timestamp_out: Utc::now(),
            head_out: sresp,
            body_out,
        });
    response
}

#[tokio::main]
async fn main() {
    let js_file = DS.find("*js").unwrap().next().unwrap().as_file().unwrap();
    let js_path_name = format!("/{}", js_file.path().to_str().unwrap());
    let wasm_file = DS.find("*wasm").unwrap().next().unwrap().as_file().unwrap();
    let wasm_path_name = format!("/{}", wasm_file.path().to_str().unwrap());
    let html_file = DS.find("*html").unwrap().next().unwrap().as_file().unwrap();

    let frontend_bytes = HashMap::from([
        ("html", html_file.contents()),
        ("wasm", wasm_file.contents()),
        ("js", js_file.contents()),
    ]);

    let cli = Cli::parse();
    let exposed_port = cli.port;
    let mode = cli.mode;

    let console_layer = console_subscriber::spawn();
    let fmt_layer = fmt::layer().event_format(fmt::format().compact());
    let filter_layer = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(fmt_layer.with_filter(filter_layer))
        .with(console_layer)
        .init();

    let traffic_log: Arc<RwLock<eaves_socket::TrafficLog>> =
        Arc::new(RwLock::new(eaves_socket::TrafficLog {
            requests: Vec::new(),
            logged_conns: Vec::new(),
        }));
    let http_client: HttpClient = hyper::Client::new();
    let app = Router::new()
        .route("/api/hello", get(hello))
        .route("/api/traffic_log", get(get_traffic_log))
        .route("/", get(html))
        .route(&wasm_path_name, get(wasm))
        .route(&js_path_name, get(js))
        .layer(Extension(frontend_bytes))
        .layer(Extension(traffic_log.clone()));

    let listener = listen_available_port();
    info!(
        "starting storm grok UI at http://{:?}",
        listener.local_addr().unwrap()
    );

    let http_serve = axum::Server::from_tcp(listener)
        .expect("Could not start server from TcpListener")
        .serve(app.into_make_service());
    if mode == Mode::Http {
        let proxy_addr = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let sg_client = client::start_client(
            proxy_addr.local_addr().unwrap().port(),
            cli,
            traffic_log.clone(),
        );

        info!("Starting proxy at {:?}", &proxy_addr);
        let proxy_app = Router::new()
            .fallback(any(proxy))
            .layer(Extension(exposed_port))
            .layer(Extension(http_client))
            .layer(Extension(traffic_log));
        let http_proxy = axum::Server::from_tcp(proxy_addr)
            .unwrap()
            .serve(proxy_app.into_make_service());
        tokio::select!(
            _ = http_proxy => {},
            _ = http_serve => {},
            _ = sg_client => {},
        )
    } else {
        let sg_client = client::start_client(exposed_port, cli, traffic_log);
        tokio::select!(
            _ = http_serve => {},
            _ = sg_client => {},
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[should_panic]
    fn another() {
        panic!("Make this test fail");
    }
}
