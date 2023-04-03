use color_eyre::eyre::Result;
use rustls::{ClientConfig, KeyLogFile};
use std::{env, io::ErrorKind, net::SocketAddr, sync::Arc};
use tracing::log::{debug, error, info, warn};
use uuid::Uuid;

use parking_lot::RwLock;

use tokio::net::TcpStream;

use quinn::{Connection, Endpoint, RecvStream, SendStream};

use crate::{
    dev_stuff,
    eaves_socket::{EavesSocket, LoggedConnection, TrafficLog},
    Cli, Mode,
};

fn setup_quic_on_available_port(host: &str) -> Endpoint {
    for port in 6001..65535 {
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

async fn start_quic_conn(endpoint: &mut Endpoint, dev_mode: bool) -> Result<quinn::Connecting> {
    let quic_server_port: String =
        env::var("SG__SERVER__QUIC_PORT").unwrap_or_else(|_| "5000".into());
    if dev_mode {
        endpoint.set_default_client_config(dev_stuff::configure_insecure_client());
        let socket_addr = format!("127.0.0.1:{}", quic_server_port);
        info!(
            "quic endpoint at {:?} configured for local, insecure connections only",
            socket_addr
        );
        Ok(endpoint.connect(socket_addr.parse::<SocketAddr>()?, "localhost")?)
    } else {
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
        // this is the fun option it sets a keylogfile so that wireshark can decrypt all the traffic
        client_config.key_log = Arc::new(KeyLogFile::new());
        let clc = quinn::ClientConfig::new(Arc::new(client_config));
        endpoint.set_default_client_config(clc);

        let socket_addr = format!("157.90.124.255:{}", quic_server_port);
        info!(
            "quic endpoint at {:?} onfigured for secure connections",
            socket_addr
        );
        Ok(endpoint.connect(socket_addr.parse::<SocketAddr>()?, "stormgrok.nl")?)
    }
}

async fn connect_client(conn: Connection, mode: Mode) -> Vec<u8> {
    let (mut send, recv) = conn.open_bi().await.unwrap();

    let token: String = match env::var("SGROK_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            warn!("You did not supply a JWT in the env var 'SGROK_TOKEN', using empty string to try and establish connection to server");
            "".to_string()
        }
    };
    send.write_all(&<[u8; 1]>::from(mode)).await.unwrap();
    send.write_all(token.as_bytes()).await.unwrap();
    send.finish().await.unwrap();

    recv.read_to_end(16)
        .await
        .expect("The server did not give us a UUID!")
}

pub async fn start_client(forward_port: u16, cli: Cli, traffic_log: Arc<RwLock<TrafficLog>>) {
    let mut endpoint = if cli.dev {
        setup_quic_on_available_port("127.0.0.1")
    } else {
        setup_quic_on_available_port("0.0.0.0")
    };
    let connection = start_quic_conn(&mut endpoint, cli.dev)
        .await
        .unwrap()
        .await
        .unwrap();
    let response_bytes = connect_client(connection.clone(), cli.mode).await;

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
            let http_server_port =
                env::var("SG__SERVER__HTTP_PORT").unwrap_or_else(|_| "3000".into());
            let uuid = Uuid::from_bytes(response_bytes.try_into().unwrap());
            match cli.dev {
                true => info!("curl http://{:?}.localhost:{}", uuid, http_server_port),
                false => info!("curl https://{:?}.stormgrok.nl", uuid),
            }
        }
    }
    // TODO: Signals are not handled properly so this thing just dies leaving the server to timeout
    tokio::select!(
        _ = handle_uni_conns_loop(connection.clone()) => {},
        _ = handle_bi_conns_loop(connection, forward_port, traffic_log) => {},
    );
    endpoint.wait_idle().await;
}

async fn handle_uni_conns_loop(connection: Connection) {
    while let Ok(stream) = connection.accept_uni().await {
        let buffered_data = stream.read_to_end(100).await.unwrap();
        if buffered_data != b"ping".to_vec() {
            info!(
                "received from server: {:?}",
                String::from_utf8_lossy(&buffered_data)
            );
        }
    }
    error!("could net receive ping from server, something is wrong with the connection")
}

async fn handle_bi_conns_loop(
    connection: Connection,
    port: u16,
    traffic_log: Arc<RwLock<TrafficLog>>,
) {
    while let Ok(streams) = connection.accept_bi().await {
        let traffic_log = traffic_log.clone();
        // Should I keep track of these spawned childtasks?
        tokio::spawn(async move { handle_client_conn(streams, port, traffic_log).await });
    }
    error!("error accepting bidirectional stream, something is wrong with the connection");
}

async fn handle_client_conn(
    streams: (SendStream, RecvStream),
    port: u16,
    traffic_log: Arc<RwLock<TrafficLog>>,
) {
    let (mut client_send, client_recv) = streams;
    match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(mut server_stream) => {
            info!("Succesfully connected client");
            let lc = LoggedConnection {
                traffic_in: vec![],
                traffic_out: vec![],
            };
            let conn_index = traffic_log.read().logged_conns.len();
            traffic_log.write().logged_conns.push(lc);

            let mut es = EavesSocket {
                reader: Box::pin(client_recv),
                writer: Box::pin(client_send),
                traffic_log: traffic_log.clone(),
                conn_index,
            };
            match tokio::io::copy_bidirectional(&mut es, &mut server_stream).await {
                Ok(res) => info!("success {:?}", res),
                Err(e) => info!("failure {:?}", e),
            }
            info!("Disconnected client!");
            info!("Full traffic log: {:?}", traffic_log.read());
        }
        Err(e) => {
            error!("Encountered {:?} while connecting to {:?}", e, port);
            client_send.finish().await.unwrap();
        }
    }
}
