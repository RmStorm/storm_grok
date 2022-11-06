use futures_util::stream::StreamExt;
use quinn::{Connecting, Connection, NewConnection};
use tokio::net::TcpListener;

use anyhow::{anyhow, bail, Result};
use jsonwebtoken::{decode, decode_header, Algorithm, Validation};
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use tokio::time::{self as time, Duration};
use tracing::log::{debug, error, info};
use uuid::Uuid;

use crate::{settings, ClientMap, KeyMap};

#[derive(Debug, Copy, Clone, PartialEq)]
enum Mode {
    Http,
    Tcp,
}

impl From<u8> for Mode {
    fn from(num: u8) -> Self {
        match num {
            116 => Mode::Tcp, // 116 = t in ascii
            _ => Mode::Http,  // default to Http
        }
    }
}

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(4);

async fn send_ping(connection: Connection) -> Result<()> {
    let mut interval = time::interval(HEARTBEAT_INTERVAL);
    loop {
        interval.tick().await;
        let mut send = connection.open_uni().await?;
        send.write_all(b"ping").await?;
        send.finish().await?;
    }
}

#[derive(Debug)]
pub struct RegisteredListener {
    tcp_listener: TcpListener,
    client_map: ClientMap,
    id: Uuid,
}

impl Drop for RegisteredListener {
    fn drop(&mut self) {
        info!("de-registering {:?}", &self.id);
        self.client_map.write().remove(&self.id);
    }
}

async fn connect_tcp_to_bi_quic(listener: RegisteredListener, connection: Connection) {
    while let Ok((mut client, addr)) = listener.tcp_listener.accept().await {
        debug!("Created tcp listen port on {:?}", addr);
        let (mut server_send, mut server_recv) = match connection.clone().open_bi().await {
            Ok(res) => res,
            Err(e) => {
                error!("Could not establish bi quic conn for forwarding: {e:?}");
                return;
            }
        };
        tokio::spawn(async move {
            let (mut client_recv, mut client_send) = client.split();
            tokio::select! {
                _ = tokio::io::copy(&mut server_recv, &mut client_send) => {}
                _ = tokio::io::copy(&mut client_recv, &mut server_send) => {}
            };
        });
    }
}

async fn listen_available_port(local_addr: &str) -> Result<TcpListener> {
    debug!("Finding available port");
    for port in 1025..65535 {
        match TcpListener::bind((local_addr, port)).await {
            Ok(l) => return Ok(l),
            Err(error) => match error.kind() {
                ErrorKind::AddrInUse => {}
                e => bail!("Encountered error while setting up tcp server: {:?}", e),
            },
        }
    }
    bail!("No ports available")
}

async fn start_local_tcp_server(mode: Mode) -> Result<TcpListener> {
    let local_addr = match mode {
        Mode::Tcp => "0.0.0.0",
        Mode::Http => "127.0.0.1",
    };
    match listen_available_port(local_addr).await {
        Ok(l) => Ok(l),
        Err(e) => {
            error!("Error while finding free port for new client: {:?}", e);
            bail!("internal server error, could not find free port for you");
        }
    }
}

pub async fn start_session(
    conn: Connecting,
    client_map: ClientMap,
    key_map: KeyMap,
    auth: settings::AuthRules,
) {
    info!("Establishing incoming connection");
    let mut conn: NewConnection = match conn.await {
        Ok(conn) => conn,
        Err(e) => {
            error!("Encountered error while starting quicc conn {e:?}");
            return;
        }
    };
    let (id, tcp_listener) = match do_handshake(&mut conn, key_map, &auth).await {
        Ok(res) => res,
        Err(e) => {
            error!("Encountered '{:?}' while handshaking client", e);
            conn.connection.close(1u32.into(), e.to_string().as_bytes());
            return;
        }
    };

    let tcp_addr = tcp_listener.local_addr().unwrap();
    debug!(
        "Setting up client session with tcp listener on {:?}",
        tcp_addr
    );
    client_map.write().insert(id, tcp_addr.to_string());
    let listener = RegisteredListener {
        tcp_listener,
        client_map,
        id,
    };
    tokio::select!(
        _ = connect_tcp_to_bi_quic(listener, conn.connection.clone()) => {},
        _ = send_ping(conn.connection) => {},
    );
}

async fn do_handshake(
    new_conn: &mut NewConnection,
    key_map: KeyMap,
    auth: &settings::AuthRules,
) -> Result<(Uuid, TcpListener)> {
    let conn_err = anyhow!("Could not establish quic connection");
    let (mut send, recv) = new_conn.bi_streams.next().await.ok_or(conn_err)??;
    // Since JWT's have to fit in a header 8kb is the practical upper limit on token size
    let received_bytes = recv.read_to_end(8192).await?;
    let requested_mode = Mode::from(received_bytes[0]);

    let tcp_listener = start_local_tcp_server(requested_mode).await?;
    let tcp_addr = tcp_listener.local_addr()?;

    if auth.enabled {
        let token = String::from_utf8_lossy(&received_bytes[1..]);
        let kid = decode_header(&token)?
            .kid
            .ok_or(anyhow!("No kid found in token header"))?;

        let token_message = match key_map.read().get(&kid) {
            Some(dec_key) => decode::<Claims>(&token, dec_key, &Validation::new(Algorithm::RS256))?,
            None => bail!("Google did not supply a DecodingKey for 'kid={kid}'"), // todo: try fetching new keys before bailing
        };

        if let Err(e) = validate_claims(token_message.claims, auth).await {
            send.reset(1u32.into())?;
            return Err(e);
        }
    }

    let id = Uuid::new_v4();
    info!("Succesfully connected new quic client with {id:?}");
    match requested_mode {
        Mode::Tcp => send.write_all(&tcp_addr.port().to_be_bytes()).await?,
        Mode::Http => send.write_all(id.as_bytes()).await?,
    }
    send.finish().await?;
    Ok((id, tcp_listener))
}

// Claims is a struct that implements Deserialize
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    hd: Option<String>,
    email: String,
    email_verified: bool,
}

async fn validate_claims(claims: Claims, auth: &settings::AuthRules) -> Result<()> {
    if claims.email_verified && auth.users.contains(&claims.email) {
        return Ok(());
    }
    if let Some(host_domain) = claims.hd {
        if auth.host_domains.contains(&host_domain) {
            return Ok(());
        }
    }
    bail!("This token is not authorized!");
}
