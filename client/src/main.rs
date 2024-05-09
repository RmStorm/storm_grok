use std::sync::Arc;

use clap::{Parser, ValueEnum};
use parking_lot::RwLock;

use shared_types::TrafficLog;

pub mod eaves_proxy;
pub mod sgclient;
pub mod ui;

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
    target_port: u16,
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

fn main() {
    simple_logger::init_with_level(log::Level::Info).expect("couldn't initialize logging");

    let cli = Cli::parse();
    let target_port = cli.target_port;
    let mode = cli.mode;

    // let demo_req = shared_types::RequestCycle {
    //     timestamp_in: chrono::Utc::now(),
    //     request_head: shared_types::RequestHead {
    //         method: "GET".into(),
    //         uri: "localhost".into(),
    //         headers: vec![("host".into(), "localhost".into())],
    //     },
    //     request_body: Vec::new(),
    //     timestamp_out: chrono::Utc::now(),
    //     response_head: shared_types::ResponseHead {
    //         status: 200,
    //         headers: vec![],
    //     },
    //     response_body: Vec::new(),
    // };
    let traffic_log: Arc<RwLock<TrafficLog>> =
        Arc::new(RwLock::new(TrafficLog { requests: vec![] }));

    let mut pingora_server = pingora::server::Server::new(None).unwrap();
    pingora_server.bootstrap();
    log::info!("bootstrapping pingora");
    let ui_server = ui::configure_ui_client(traffic_log.clone());

    if mode == Mode::Http {
        let (eaves_proxy, proxy_port) = eaves_proxy::configure_eaves_proxy(
            &pingora_server.configuration,
            target_port,
            traffic_log,
        );
        let sg_client = sgclient::configure_storm_grok_client(proxy_port, cli);

        pingora_server.add_service(eaves_proxy);
        pingora_server.add_service(sg_client);
    } else {
        let sg_client = sgclient::configure_storm_grok_client(target_port, cli);
        pingora_server.add_service(sg_client);
    }
    pingora_server.add_service(ui_server);
    pingora_server.run_forever();
}
