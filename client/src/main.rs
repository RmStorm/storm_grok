use include_dir::{include_dir, Dir};

use std::{collections::HashMap, io::ErrorKind, net::TcpListener, sync::Arc};

use axum::{
    body::StreamBody, http::header, response::IntoResponse, routing::get, Extension, Json, Router,
};
use parking_lot::RwLock;
use tokio_util::io::ReaderStream;
use tracing::info;
use tracing_subscriber::{filter::LevelFilter, fmt, EnvFilter};

static DS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../dist");

mod client;
mod copy_writer;
mod dev_stuff;

use clap::Parser;
use clap::ValueEnum;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    /// What mode to run the program in
    #[clap(arg_enum, value_parser)]
    mode: Mode,
    /// Port to forward to
    #[clap(value_parser = clap::value_parser!(u16).range(1..65536))]
    port: u16,
    #[clap(long, short, action)]
    dev: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Mode {
    Http,
    Tcp,
}

impl From<Mode> for [u8; 1] {
    fn from(mode: Mode) -> [u8; 1] {
        match mode {
            Mode::Tcp => [116],  // 116 = t in ascii
            Mode::Http => [104], // 104 = h in ascii
        }
    }
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

async fn get_traffic_log(
    Extension(traffic_log): Extension<Arc<RwLock<copy_writer::TrafficLog>>>,
) -> Json<copy_writer::TrafficLog> {
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
    fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .event_format(fmt::format().pretty())
        .init();

    let traffic_log: Arc<RwLock<copy_writer::TrafficLog>> =
        Arc::new(RwLock::new(copy_writer::TrafficLog {
            logged_conns: Vec::new(),
        }));
    let sg_client = client::start_client(cli, traffic_log.clone());

    let listener = listen_available_port();
    info!(
        "starting storm grok UI at http://{:?}",
        listener.local_addr().unwrap()
    );

    let app = Router::new()
        .route("/api/hello", get(hello))
        .route("/api/traffic_log", get(get_traffic_log))
        .route("/", get(html))
        .route(&wasm_path_name, get(wasm))
        .route(&js_path_name, get(js))
        .layer(Extension(frontend_bytes))
        .layer(Extension(traffic_log));

    let http_serve = axum::Server::from_tcp(listener)
        .expect("Could not start server from TcpListener")
        .serve(app.into_make_service());

    tokio::select!(
        _ = http_serve => {},
        _ = sg_client => {},
    );
}

#[cfg(test)]
mod tests {
    #[test]
    #[should_panic]
    fn another() {
        panic!("Make this test fail");
    }
}
