use color_eyre::eyre::Result;
use log::{debug, error, info, warn};
use std::{env, io::ErrorKind, net::SocketAddr, sync::Arc};
use uuid::Uuid;

use quinn::ClientConfig;
use rustls::KeyLogFile;
use tokio::net::TcpStream;

use quinn::{Connection, Endpoint, RecvStream, SendStream};

use crate::{Cli, Mode};

pub struct SgClient {
    dev: bool,
    mode: Mode,
    intermediate_target_port: u16,
    final_target_port: u16,
}

pub fn configure_storm_grok_client(intermediate_target_port: u16, cli: Cli) -> SgClient {
    SgClient {
        dev: cli.dev,
        mode: cli.mode,
        intermediate_target_port,
        final_target_port: cli.target_port,
    }
}

use async_trait::async_trait;
#[async_trait]
impl pingora::services::Service for SgClient {
    async fn start_service(
        &mut self,
        _fds: Option<pingora::server::ListenFds>,
        shutdown: pingora::server::ShutdownWatch,
    ) {
        log::info!("starting service {} {:?}", self.name(), shutdown);
        let mut endpoint = if self.dev {
            setup_quic_on_available_port("127.0.0.1")
        } else {
            setup_quic_on_available_port("0.0.0.0")
        };
        let connection = start_quic_conn(&mut endpoint, self.dev)
            .await
            .unwrap()
            .await
            .unwrap();
        let response_bytes = sgrok_handshake(connection.clone(), self.mode).await;

        info!(
            "Exposing localhost:{:?} on the internet!",
            self.final_target_port
        );
        match self.mode {
            Mode::Tcp => {
                let port = u16::from_be_bytes(response_bytes.try_into().unwrap());
                match self.dev {
                    true => info!("nc localhost {:?}", port),
                    false => info!("nc stormgrok.nl {:?}", port),
                }
            }
            Mode::Http => {
                let http_server_port =
                    env::var("SG__SERVER__HTTP_PORT").unwrap_or_else(|_| "3000".into());
                let uuid = Uuid::from_bytes(response_bytes.try_into().unwrap());
                match self.dev {
                    true => info!("curl http://{:?}.localhost:{}", uuid, http_server_port),
                    false => info!("curl https://{:?}.stormgrok.nl", uuid),
                }
            }
        }
        tokio::select!(
            _ = handle_uni_conns_loop(connection.clone()) => {},
            _ = handle_bi_conns_loop(connection, self.intermediate_target_port) => {},
        );

        endpoint.wait_idle().await;
    }

    fn name(&self) -> &str {
        "storm grok client"
    }
}

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
        endpoint.set_default_client_config(configure_insecure_client());
        let socket_addr = format!("127.0.0.1:{}", quic_server_port);
        info!(
            "quic endpoint at {:?} configured for local, insecure connections only",
            socket_addr
        );
        let t = socket_addr.parse::<SocketAddr>()?;
        Ok(endpoint.connect(t, "localhost")?)
    } else {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| {
            rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(
                ta.subject.as_ref().to_owned(),
                ta.subject_public_key_info.as_ref().to_owned(),
                ta.name_constraints
                    .as_ref()
                    .map(|nc| nc.as_ref().to_owned()),
            )
        }));
        let mut client_config = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        // this is the fun option it sets a keylogfile so that wireshark can decrypt all the traffic
        client_config.key_log = Arc::new(rustls::KeyLogFile::new());
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

async fn sgrok_handshake(conn: quinn::Connection, mode: Mode) -> Vec<u8> {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();

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

async fn handle_uni_conns_loop(connection: Connection) {
    while let Ok(mut stream) = connection.accept_uni().await {
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

async fn handle_bi_conns_loop(connection: Connection, target_port: u16) {
    while let Ok(streams) = connection.accept_bi().await {
        // Should I keep track of these spawned childtasks?
        tokio::spawn(async move { handle_client_conn(streams, target_port).await });
    }
    error!("error accepting bidirectional stream, something is wrong with the connection");
}

async fn handle_client_conn(streams: (SendStream, RecvStream), target_port: u16) {
    let (mut client_send, mut client_recv) = streams;
    match TcpStream::connect(("127.0.0.1", target_port)).await {
        Ok(server_stream) => {
            let (mut read_half, mut write_half) = server_stream.into_split();
            let yada = tokio::join!(
                tokio::io::copy(&mut client_recv, &mut write_half),
                tokio::io::copy(&mut read_half, &mut client_send),
            );
            info!("Disconnected client! {:?}", yada);
        }
        Err(e) => {
            error!("Encountered {:?} while connecting to {:?}", e, target_port);
            client_send.finish().await.unwrap();
        }
    }
}

struct SkipServerVerification;

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl rustls::client::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

fn configure_insecure_client() -> ClientConfig {
    let mut crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();
    crypto.key_log = Arc::new(KeyLogFile::new());
    ClientConfig::new(Arc::new(crypto))
}
