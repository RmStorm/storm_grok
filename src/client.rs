use actix::dev::MessageResponse;
use actix::dev::OneshotSender;
use actix::io::SinkWrite;
use actix::prelude::*;
use actix_codec::Framed;
use actix_web::{
    http::{Method, Uri},
    web::{Bytes, Payload},
};
use async_trait::async_trait;
use awc::{error::WsProtocolError, ws, BoxedSocket, Client};
use awc::{http::header::HeaderValue, ClientRequest};
use futures::{
    sink::Buffer,
    stream::{SplitSink, SplitStream},
};
use futures_util::stream::StreamExt;
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::{error::Error, net::SocketAddr};
use url::Url;

pub type WsFramedSink = SplitSink<Framed<BoxedSocket, ws::Codec>, ws::Message>;
pub type WsFramedStream = SplitStream<Framed<BoxedSocket, ws::Codec>>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FullResponseData {
    pub status: u16,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OrderedResponseData {
    pub response_data: FullResponseData,
    pub response_number: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FullRequestData {
    pub method: String,
    pub uri: String,
    pub version: String,
    pub headers: Vec<(String, Vec<u8>)>,
    pub peer_addr: Option<SocketAddr>,
    pub body: Vec<u8>,
}

impl FullRequestData {
    fn make_request(&self, client: &mut Client, forward_url: &mut Url) -> ClientRequest {
        // Do I need the version for something?
        // let v = match cur_req.head.version.as_str() {
        //     "HTTP/0.9" => Some(Version::HTTP_09),
        //     "HTTP/1.0" => Some(Version::HTTP_10),
        //     "HTTP/1.1" => Some(Version::HTTP_11),
        //     "HTTP/2.0" => Some(Version::HTTP_2),
        //     "HTTP/3.0" => Some(Version::HTTP_3),
        //     _ => None,
        // };
        // info!("v={v:?}");

        let mut new_url = forward_url.clone();
        let uri = Uri::from_str(&self.uri).unwrap();
        new_url.set_path(uri.path());
        new_url.set_query(uri.query());

        let mut r = client.request(Method::from_str(&self.method).unwrap(), new_url.as_str());

        for (k, v) in self.headers.iter() {
            r = r.append_header((k.as_str(), HeaderValue::from_bytes(v).unwrap()));
        }
        let r = match &self.peer_addr {
            Some(addr) => r.insert_header(("x-forwarded-for", format!("{}", addr.ip()))),
            None => r,
        };
        r
    }

    async fn do_test() {
        info!("does it work");
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OrderedRequestData {
    pub request_data: FullRequestData,
    pub request_number: usize,
}

// TODO: figure out how to store requests just once!!, responses can be unique so have to be copied over and over
pub struct StormGrokClient {
    pub sink: SinkWrite<ws::Message, WsFramedSink>,
    pub forward_url: Url,
    pub received_requests: Vec<FullRequestData>,
    pub executed_request_cycles: Vec<(FullRequestData, FullResponseData)>,
    pub client: Client,
}

impl StormGrokClient {
    // All of this magic comes from: https://stackoverflow.com/questions/70118994/build-a-websocket-client-using-actix
    pub fn start(sink: WsFramedSink, stream: WsFramedStream, forward_url: Url) -> Addr<Self> {
        StormGrokClient::create(|ctx| {
            ctx.add_stream(stream);
            StormGrokClient {
                sink: SinkWrite::new(sink, ctx),
                forward_url: forward_url,
                received_requests: vec![],
                executed_request_cycles: vec![],
                client: Client::default(),
            }
        })
    }
}

impl Actor for StormGrokClient {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Context<Self>) {
        info!("StormGrokClient started");
    }
}

impl actix::io::WriteHandler<WsProtocolError> for StormGrokClient {}

#[derive(Message, Debug)]
#[rtype(result = "()")]
struct Respond {
    r: OrderedResponseData,
}
impl Handler<Respond> for StormGrokClient {
    type Result = ();

    fn handle(&mut self, msg: Respond, _ctx: &mut Self::Context) {
        let r = msg.r;
        match self.sink.write(ws::Message::Binary(
            bincode::serialize(&r).unwrap().try_into().unwrap(),
        )) {
            Ok(result) => info!("Succesfully wrote back on websocket result='{result:?}'"),
            Err(e) => error!("Error During Write on websocket {:?}", e),
        }
    }
}

#[derive(Message, Debug)]
#[rtype(result = "()")]
struct StoreCycle {
    req: FullRequestData,
    res: FullResponseData,
}
impl Handler<StoreCycle> for StormGrokClient {
    type Result = ();

    fn handle(&mut self, msg: StoreCycle, _ctx: &mut Self::Context) {
        self.executed_request_cycles.push((msg.req, msg.res));
    }
}

#[derive(Message, Debug)]
#[rtype(result = "Vec<(FullRequestData, FullResponseData)>")]
pub struct GetCycles {
    pub last: usize,
}

impl<A, M> MessageResponse<A, M> for FullRequestData
where
    A: Actor,
    M: Message<Result = FullRequestData>,
{
    fn handle(self, ctx: &mut A::Context, tx: Option<OneshotSender<M::Result>>) {
        if let Some(tx) = tx {
            tx.send(self);
        }
    }
}

impl<A, M> MessageResponse<A, M> for FullResponseData
where
    A: Actor,
    M: Message<Result = FullResponseData>,
{
    fn handle(self, ctx: &mut A::Context, tx: Option<OneshotSender<M::Result>>) {
        if let Some(tx) = tx {
            tx.send(self);
        }
    }
}

impl Handler<GetCycles> for StormGrokClient {
    type Result = Vec<(FullRequestData, FullResponseData)>;

    fn handle(&mut self, msg: GetCycles, _ctx: &mut Self::Context) -> Self::Result {
        self.executed_request_cycles.clone()
    }
}

async fn send_request_to_downstream(
    r: ClientRequest,
    req_data: FullRequestData,
    client_address: Addr<StormGrokClient>,
    relay_back: Option<usize>,
) -> Result<&'static str, Box<dyn Error>> {
    let mut response = r.send_body(req_data.body.clone()).await?;
    let frd = FullResponseData {
        status: response.status().as_u16(),
        headers: response
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), v.as_bytes().to_owned()))
            .collect::<Vec<_>>(),
        body: response.body().await?.to_vec(),
    };
    client_address.do_send(StoreCycle {
        req: req_data,
        res: frd.clone(),
    });
    match relay_back {
        Some(req_num) => {
            client_address
                .send(Respond {
                    r: OrderedResponseData {
                        response_data: frd,
                        response_number: req_num,
                    },
                })
                .await?;
        }
        _ => info!("no relay"),
    }
    Ok("eeey")
}

#[derive(Message, Debug)]
#[rtype(result = "()")]
struct HitDownstream {
    request_number: usize,
    relay_back: bool,
}
impl Handler<HitDownstream> for StormGrokClient {
    type Result = ();

    fn handle(&mut self, msg: HitDownstream, ctx: &mut Self::Context) {
        let cur_req = &self.received_requests[msg.request_number];

        let r = cur_req.make_request(&mut self.client, &mut self.forward_url);

        let relay_back = if msg.relay_back {
            Some(msg.request_number)
        } else {
            None
        };

        send_request_to_downstream(r, cur_req.clone(), ctx.address().clone(), relay_back)
            .into_actor(self)
            .then(|result, act, ctx| {
                info!("send_request_to_downstream result={result:?}");
                fut::ready(())
            })
            .spawn(ctx);
    }
}

// TODO: Rewrite the whole damn thing to use tcp sockets instead of websockets?
impl StreamHandler<Result<ws::Frame, WsProtocolError>> for StormGrokClient {
    fn handle(&mut self, item: Result<ws::Frame, WsProtocolError>, ctx: &mut Self::Context) {
        use ws::Frame;
        match item.unwrap() {
            Frame::Text(text_bytes) => {
                info!("Receiving Message: {}", text_bytes.len());

                let text = std::str::from_utf8(text_bytes.as_ref()).unwrap();
                info!("Receiving Message: {}", text);
            }
            Frame::Binary(bin) => {
                info!("Receiving bin Message with length: {}", bin.len());
                let data: OrderedRequestData = bincode::deserialize(&bin).unwrap();
                let request_number = data.request_number;
                self.received_requests
                    .insert(request_number, data.request_data);

                info!("all_reqs:\n{:?}", self.received_requests);
                ctx.address().do_send(HitDownstream {
                    request_number,
                    relay_back: true,
                });
            }
            Frame::Continuation(_) => {}
            Frame::Ping(_) => {
                self.sink
                    .write(ws::Message::Pong(format!("pong").into()))
                    .unwrap();
            }
            Frame::Pong(_) => {
                //self.hb = Instant::now();
            }
            Frame::Close(_) => {}
        }
    }
}
