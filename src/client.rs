use actix::{prelude::*, Actor, Addr, StreamHandler};
use actix_web::{rt::net::TcpStream, web};
use quinn::{
    ClientConfig, Connection, ConnectionError, Endpoint, NewConnection, RecvStream, SendStream,
};
use tracing::log::{error, info};

use anyhow::Result;
use std::{env, io::ErrorKind, net::SocketAddr, sync::Arc};
use uuid::Uuid;

use crate::StopHandle;

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

fn configure_client() -> ClientConfig {
    let crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();

    ClientConfig::new(Arc::new(crypto))
}

fn setup_quic_available_port() -> Endpoint {
    for port in 5001..65535 {
        let socket: SocketAddr = format!("127.0.0.1:{port:?}")
            .as_str()
            .parse::<SocketAddr>()
            .unwrap();
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

pub struct StormGrokClient {
    pub id: Uuid,
    pub connection: Connection,
    pub port: u16,
    pub stop_handle: web::Data<StopHandle>,
}

pub async fn start_client(stop_handle: web::Data<StopHandle>, port: u16) -> Addr<StormGrokClient> {
    let mut endpoint = setup_quic_available_port();
    endpoint.set_default_client_config(configure_client());

    // Connect to the server passing in the server name which is supposed to be in the server certificate.
    let new_connection = endpoint
        .connect("127.0.0.1:5000".parse::<SocketAddr>().unwrap(), "localhost")
        .unwrap()
        .await
        .unwrap();

    let (mut send, recv) = new_connection.connection.open_bi().await.unwrap();
    let token: String = env::var("TOKEN").unwrap();
    send.write_all(token.as_bytes()).await.unwrap();
    send.finish().await.unwrap();

    let uuid_bytes = recv
        .read_to_end(16)
        .await
        .expect("The server did not give us a UUID!");
    let uuid: &[u8; 16] = &uuid_bytes.try_into().unwrap();
    let uuid = Uuid::from_bytes(*uuid);
    info!("Exposing localhost:{:?} on the internet!", port);
    info!("got uuid {:?} assigned from server.", uuid);
    info!("curl http://{:?}.localhost:3000", uuid);
    StormGrokClient::start(uuid, port, new_connection, stop_handle)
}

impl Actor for StormGrokClient {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Context<Self>) {
        info!("StormGrokClient started");
    }
    fn stopped(&mut self, _ctx: &mut Context<Self>) {
        info!("Party is over!");
        self.stop_handle.stop(true)
    }
}

impl StormGrokClient {
    fn start(
        id: Uuid,
        port: u16,
        new_conn: NewConnection,
        stop_handle: web::Data<StopHandle>,
    ) -> Addr<Self> {
        StormGrokClient::create(|ctx| {
            ctx.add_stream(new_conn.bi_streams);
            ctx.add_stream(new_conn.uni_streams);
            let connection = new_conn.connection;
            StormGrokClient {
                id,
                port,
                connection,
                stop_handle,
            }
        })
    }
}

async fn handle_uni_stream(
    stream: Result<RecvStream, ConnectionError>,
) -> Result<()> {
    let recv = stream?;
    let buffed_data = recv.read_to_end(100).await?;
    if buffed_data != b"ping".to_vec() {
        info!(
            "received from server: {:?}",
            String::from_utf8_lossy(&buffed_data)
        );
    }
    Ok(())
}

impl StreamHandler<Result<RecvStream, ConnectionError>> for StormGrokClient {
    fn handle(&mut self, item: Result<RecvStream, ConnectionError>, ctx: &mut Self::Context) {
        handle_uni_stream(item)
            .into_actor(self)
            .then(|res, _act, ctx| {
                if let Err(err) = res {
                    error!("encountered connection error in uni_stream: {:?}", err);
                    ctx.stop();
                }
                fut::ready(())
            })
            .spawn(ctx);
    }
}

async fn handle_client_conn(client_connection: (SendStream, RecvStream), port: u16) {
    let (mut client_send, mut client_recv) = client_connection;
    match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(mut server_stream) => {
            info!("Succesfully connected client");
            let (mut server_recv, mut server_send) = server_stream.split();
            tokio::select! {
                _ = tokio::io::copy(&mut server_recv, &mut client_send) => {}
                _ = tokio::io::copy(&mut client_recv, &mut server_send) => {}
            };
        },
        Err(e) => {
            error!("Encountered {:?} while connecting to {:?}", e, port);
            client_send.finish().await.unwrap();
        },
    }
}

impl StreamHandler<Result<(SendStream, RecvStream), ConnectionError>> for StormGrokClient {
    fn handle(
        &mut self,
        item: Result<(SendStream, RecvStream), ConnectionError>,
        ctx: &mut Self::Context,
    ) {
        match item {
            Ok(client_connection) => {
                handle_client_conn(client_connection, self.port)
                    .into_actor(self)
                    .spawn(ctx);
            }
            Err(err) => {
                error!("encountered connection error in bistream: {:?}", err);
                ctx.stop();
            }
        }
    }
}
