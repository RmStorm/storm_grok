use actix::Addr;
use actix_web::{dev::ServerHandle, web, App, HttpRequest, HttpResponse, HttpServer};
use parking_lot::Mutex;

use std::{io::ErrorKind, net::TcpListener};
use tracing::info;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::{fmt, EnvFilter};

mod client;
mod dev_stuff;

use clap::Parser;
// use clap::ValueEnum;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    /// What mode to run the program in
    // #[clap(arg_enum, value_parser)]
    // mode: Mode,
    /// Port to forward to
    #[clap(value_parser = clap::value_parser!(u16).range(1..65536))]
    port: u16,
    #[clap(long, short, action)]
    dev: bool,
}

// #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
// enum Mode {
//     Http,
//     Tcp,
// }

async fn index(
    req: HttpRequest,
    body: web::Bytes,
    srv: web::Data<Addr<client::StormGrokClient>>,
) -> HttpResponse {
    info!("\nREQ: {req:?}");
    info!("body: {body:?}");
    info!("srv: {srv:?}");
    HttpResponse::Ok().body("request replaying is cool!\n")
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

#[actix_web::main]
async fn main() -> Result<(), std::io::Error> {
    let cli = Cli::parse();
    fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .event_format(fmt::format().pretty())
        .init();
    let stop_handle = web::Data::new(StopHandle::default());
    let client_address = client::start_client(stop_handle.clone(), cli).await;

    let server_port = listen_available_port();
    info!(
        "starting storm grok interface at http://{:?}",
        server_port.local_addr()?
    );
    let srv = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(client_address.clone()))
            .service(web::resource("/").to(index))
    })
    .listen(server_port)?
    .run();
    stop_handle.register(srv.handle());
    srv.await
}

// This comes from: https://github.com/actix/examples/tree/master/shutdown-server
#[derive(Debug, Default)]
pub struct StopHandle {
    pub inner: Mutex<Option<ServerHandle>>,
}
impl StopHandle {
    /// Sets the server handle to stop.
    pub(crate) fn register(&self, handle: ServerHandle) {
        *self.inner.lock() = Some(handle);
    }

    /// Sends stop signal through contained server handle.
    pub(crate) fn stop(&self, graceful: bool) {
        let _ = self.inner.lock().as_ref().unwrap().stop(graceful);
    }
}
