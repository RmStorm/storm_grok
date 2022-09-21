use std::net::ToSocketAddrs;

use actix::{Actor, Addr};
use actix_web::{error, middleware, web, App, Error, HttpRequest, HttpResponse, HttpServer};
use awc::Client;
use clap::StructOpt;
use futures_util::{sink::SinkExt as _, stream::StreamExt as _};
use log::info;
use openssl::ssl::SslConnector;
use url::Url;

mod client;

async fn all(
    req: HttpRequest,
    payload: web::Payload,
    sgc: web::Data<Addr<client::StormGrokClient>>,
) -> Result<HttpResponse, Error> {
    let yada = sgc.send(client::GetCycles { last: 0 }).await;
    info!("{yada:?}");
    Ok(HttpResponse::Ok().body("partyparty"))
}

#[derive(clap::Parser, Debug)]
struct CliArguments {
    forward_port: u16,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let args = CliArguments::parse();
    let forward_socket_addr = ("localhost", args.forward_port)
        .to_socket_addrs()?
        .next()
        .expect("given forwarding address was not valid");

    let forward_url = format!("http://{forward_socket_addr}");
    let forward_url = Url::parse(&forward_url).unwrap();

    // All of this magic comes from: https://stackoverflow.com/questions/70118994/build-a-websocket-client-using-actix
    // TODO: use ssl/wss?
    let (_, framed) = Client::default()
        .ws("ws://localhost:3000/ws/")
        .connect()
        .await
        .unwrap();

    // TODO, this server address gets used by all threads which means there is effectively just 
    // one client used for forwarding requests. Probably not at all a problem?
    let (sink, stream): (client::WsFramedSink, client::WsFramedStream) = framed.split();
    let server_address = client::StormGrokClient::start(sink, stream, forward_url.clone());
    info!("Started websocket to ws://localhost:3000/ws/",);
    info!("starting StormGrok UI at http://localhost:4040");
    info!("forwarding to {forward_url}");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(server_address.clone()))
            .wrap(middleware::Logger::default())
            .route("/all/", web::get().to(all))
    })
    .bind(("127.0.0.1", 4040))?
    .workers(2)
    .run()
    .await
}
