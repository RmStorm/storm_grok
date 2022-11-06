use anyhow::Result;
use futures::StreamExt;
use quinn::Incoming;
use quinn::{Endpoint, ServerConfig};
use std::net::SocketAddr;
use tokio::task::JoinHandle;
use tracing::info;

use crate::{session, settings, ClientMap, KeyMap};

#[derive(Debug)]
pub struct ChildTask<T> {
    inner: JoinHandle<T>,
}

impl<T> Drop for ChildTask<T> {
    fn drop(&mut self) {
        self.inner.abort()
    }
}

pub async fn start_storm_grok_server(
    config: &settings::Settings,
    client_map: ClientMap,
    key_map: KeyMap,
) -> Result<()> {
    let server_address = format!("{}:{:?}", config.server.quic_host, config.server.quic_port);
    let server_address = server_address.parse::<SocketAddr>().unwrap();

    let (certs, key) = config.get_certs_and_key();
    let server_config = ServerConfig::with_single_cert(certs, key).expect("bad certificate/key");

    info!("Starting Quic server on {:?}", server_address);
    let (endpoint, incoming) = Endpoint::server(server_config, server_address)?;
    handle_conns_loop(incoming, client_map, key_map, config.auth.clone()).await;
    info!("Waiting for clean quic server shutdown");
    endpoint.wait_idle().await;
    Ok(())
}

async fn handle_conns_loop(
    mut incoming: Incoming,
    client_map: ClientMap,
    key_map: KeyMap,
    auth: settings::AuthRules,
) {
    // Todo: I guess this vector will grow very long now.. Need something to prune the done tasks off.
    let mut handles = Vec::new();
    while let Some(conn) = incoming.next().await {
        let ses = session::start_session(conn, client_map.clone(), key_map.clone(), auth.clone());
        handles.push(ChildTask {
            inner: tokio::spawn(ses),
        });
    }
}
