use futures::StreamExt;
use rustls::{ClientConfig, KeyLogFile};
use std::{env, io::ErrorKind, net::SocketAddr, sync::Arc};
use tracing::log::{debug, error, info, warn};
use uuid::Uuid;

use parking_lot::RwLock;

use tokio::net::TcpStream;

use quinn::{Endpoint, IncomingBiStreams, IncomingUniStreams, RecvStream, SendStream};

use crate::{
    copy_writer::{create_logged_writers, TrafficLog},
    dev_stuff, Cli, Mode,
};

fn setup_quic_on_available_port(host: &str) -> Endpoint {
    for port in 5001..65535 {
        let socket: SocketAddr = format!("{host}:{port:?}")
            .as_str()
            .parse::<SocketAddr>()
            .unwrap();
        debug!("Found a free socket for quic client '{:?}'", &socket);
        match Endpoint::client(socket) {
            Ok(endpoint) => return endpoint,
            Err(error) => match error.kind() {
                ErrorKind::AddrInUse => {}
                other_error => panic!(
                    "Encountered errr while setting up quic client: {:?}",
                    other_error
                ),
            },
        }
    }
    panic!("No ports available")
}

pub async fn start_client(cli: Cli, traffic_log: Arc<RwLock<TrafficLog>>) {
    let mut endpoint;
    let new_connection = if cli.dev {
        endpoint = setup_quic_on_available_port("127.0.0.1");
        endpoint.set_default_client_config(dev_stuff::configure_insecure_client());
        info!("quic endpoint configured for insecure and local connections only");
        endpoint.connect("127.0.0.1:5000".parse::<SocketAddr>().unwrap(), "localhost")
    } else {
        endpoint = setup_quic_on_available_port("0.0.0.0");
        let mut root_store = rustls::RootCertStore::empty();
        root_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
            rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(
                ta.subject,
                ta.spki,
                ta.name_constraints,
            )
        }));
        let mut client_config = ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        // this is the fun option
        client_config.key_log = Arc::new(KeyLogFile::new());
        let clc = quinn::ClientConfig::new(Arc::new(client_config));
        endpoint.set_default_client_config(clc);
        info!("quic endpoint configured for secure connections");
        endpoint.connect(
            "157.90.124.255:5000".parse::<SocketAddr>().unwrap(),
            "stormgrok.nl",
        )
    };
    let new_connection = new_connection.unwrap().await.unwrap();

    let (mut send, recv) = new_connection.connection.open_bi().await.unwrap();

    let token: String = match env::var("SGROK_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            warn!("You did not supply a JWT in the env var 'SGROK_TOKEN', using empty string to try and establish connection to server");
            "".to_string()
        }
    };
    send.write_all(&<[u8; 1]>::from(cli.mode)).await.unwrap();
    send.write_all(token.as_bytes()).await.unwrap();
    send.finish().await.unwrap();

    let response_bytes = recv
        .read_to_end(16)
        .await
        .expect("The server did not give us a UUID!");

    info!("Exposing localhost:{:?} on the internet!", cli.port);
    match cli.mode {
        Mode::Tcp => {
            let port = u16::from_be_bytes(response_bytes.try_into().unwrap());
            match cli.dev {
                true => info!("nc localhost {:?}", port),
                false => info!("nc stormgrok.nl {:?}", port),
            }
        }
        Mode::Http => {
            let uuid = Uuid::from_bytes(response_bytes.try_into().unwrap());
            match cli.dev {
                true => info!("curl http://{:?}.localhost:3000", uuid),
                false => info!("curl https://{:?}.stormgrok.nl", uuid),
            }
        }
    }
    tokio::select!(
        _ = handle_uni_conns_loop(new_connection.uni_streams) => {},
        _ = handle_bi_conns_loop(new_connection.bi_streams, cli.port, traffic_log) => {},
    );
    endpoint.wait_idle().await;
}

async fn handle_uni_conns_loop(mut uni_streams: IncomingUniStreams) {
    while let Some(stream) = uni_streams.next().await {
        let recv = stream.unwrap();
        let buffed_data = recv.read_to_end(100).await.unwrap();
        if buffed_data != b"ping".to_vec() {
            info!(
                "received from server: {:?}",
                String::from_utf8_lossy(&buffed_data)
            );
        }
    }
}

async fn handle_bi_conns_loop(
    mut bi_streams: IncomingBiStreams,
    port: u16,
    traffic_log: Arc<RwLock<TrafficLog>>,
) {
    while let Some(stream) = bi_streams.next().await {
        info!("Incoming bi stream");
        handle_client_conn(stream.unwrap(), port, traffic_log.clone()).await;
    }
}

async fn handle_client_conn(
    client_connection: (SendStream, RecvStream),
    port: u16,
    traffic_log: Arc<RwLock<TrafficLog>>,
) {
    info!("were here?");
    let (mut client_send, mut client_recv) = client_connection;
    match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(mut server_stream) => {
            info!("Succesfully connected client");
            let (mut server_recv, server_send) = server_stream.split();
            /*
            Ah crap, I thought I was really clever with storing all the data that has traveled over
            a connection but.... It doesn't work well for http forwarding mode.. The http client in
            the stormgrokserver that takes care of proxying the traffic over here does connection
            pooling for all incoming connections to it.

            Keeping track of the seperate connections does work well with tcp forwarding though!
            */
            let (mut client_send, mut server_send) =
                create_logged_writers(client_send, server_send, traffic_log.clone());
            tokio::select! {
                _ = tokio::io::copy(&mut server_recv, &mut client_send) => {info!("Server hit EOF")}
                _ = tokio::io::copy(&mut client_recv, &mut server_send) => {}
            };
            info!("Disconnected client");
            info!("Full traffic log: {:?}", traffic_log.read());
        }
        Err(e) => {
            error!("Encountered {:?} while connecting to {:?}", e, port);
            client_send.finish().await.unwrap();
        }
    }
}
