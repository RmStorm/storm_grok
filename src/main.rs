use std::{io::ErrorKind, net::TcpListener, sync::Arc};
use tracing::info;
use tracing_subscriber::{filter::LevelFilter, fmt, EnvFilter};

use parking_lot::RwLock;

use axum::{routing::get, Router};

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

async fn index() -> &'static str {
    "request replaying is cool!\n"
}

#[tokio::main]
async fn main() {
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
        "starting storm grok interface at http://{:?}",
        listener.local_addr().unwrap()
    );

    let app = Router::new().route("/", get(index));
    let http_serve = axum::Server::from_tcp(listener)
        .expect("Could not create server from TcpListener")
        .serve(app.into_make_service());

    tokio::select!(
        _ = http_serve => {},
        _ = sg_client => {},
    );
}
