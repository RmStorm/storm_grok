use crate::copy_writer::TrafficLog;
use actix::{prelude::*, Actor, Addr, StreamHandler};
use actix_web::{rt::net::TcpStream, web};
use parking_lot::RwLock;
use quinn::{
    ClientConfig, Connection, ConnectionError, Endpoint, NewConnection, RecvStream, SendStream,
};
use std::sync::Arc;

use tracing::log::{debug, error, info};
use uuid::Uuid;

use anyhow::{Context as AH_Context, Result};
use std::{env, io::ErrorKind, net::SocketAddr};

use crate::{
    copy_writer::{create_logged_writers, print_full_traffic_log},
    dev_stuff, Cli, Mode, StopHandle,
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

pub struct StormGrokClient {
    pub connection: Connection,
    pub port: u16,
    pub stop_handle: web::Data<StopHandle>,
    pub traffic_log: Arc<RwLock<TrafficLog>>,
}

pub async fn start_client(
    stop_handle: web::Data<StopHandle>,
    cli: Cli,
    traffic_log: Arc<RwLock<TrafficLog>>,
) -> Addr<StormGrokClient> {
    let new_connection = if cli.dev {
        let mut endpoint = setup_quic_on_available_port("127.0.0.1");
        endpoint.set_default_client_config(dev_stuff::configure_insecure_client());
        info!("quic endpoint configured for insecure and local connections only");
        endpoint.connect("127.0.0.1:5000".parse::<SocketAddr>().unwrap(), "localhost")
    } else {
        let mut endpoint = setup_quic_on_available_port("0.0.0.0");
        endpoint.set_default_client_config(ClientConfig::with_native_roots());
        info!("quic endpoint configured for secure connections");
        endpoint.connect(
            "157.90.124.255:5000".parse::<SocketAddr>().unwrap(),
            "stormgrok.nl",
        )
    };
    let new_connection = new_connection.unwrap().await.unwrap();

    let (mut send, recv) = new_connection.connection.open_bi().await.unwrap();
    let token: String = env::var("SGROK_TOKEN")
        .context("You need to supply a jwt in env var SGROK_TOKEN")
        .unwrap();
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
                false => info!("curl https://{:?}.stormgrok.nl:3000", uuid),
            }
        }
    }
    StormGrokClient::start(cli.port, new_connection, stop_handle, traffic_log)
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
        port: u16,
        new_conn: NewConnection,
        stop_handle: web::Data<StopHandle>,
        traffic_log: Arc<RwLock<TrafficLog>>,
    ) -> Addr<Self> {
        StormGrokClient::create(|ctx| {
            ctx.add_stream(new_conn.bi_streams);
            ctx.add_stream(new_conn.uni_streams);
            StormGrokClient {
                port: port,
                connection: new_conn.connection,
                stop_handle: stop_handle,
                traffic_log: traffic_log,
            }
        })
    }
}

async fn handle_uni_stream(stream: Result<RecvStream, ConnectionError>) -> Result<()> {
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

async fn handle_client_conn(
    client_connection: (SendStream, RecvStream),
    port: u16,
    traffic_log: Arc<RwLock<TrafficLog>>,
) {
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
                _ = tokio::io::copy(&mut server_recv, &mut client_send) => {}
                _ = tokio::io::copy(&mut client_recv, &mut server_send) => {}
            };
            info!("Disconnected client");
            print_full_traffic_log(traffic_log);
        }
        Err(e) => {
            error!("Encountered {:?} while connecting to {:?}", e, port);
            client_send.finish().await.unwrap();
        }
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
                handle_client_conn(client_connection, self.port, self.traffic_log.clone())
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
