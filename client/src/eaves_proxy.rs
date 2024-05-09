use axum::http::{HeaderName, HeaderValue};
use bytes::Bytes;
use chrono::Utc;
use pingora::services::listening::Service;
use shared_types::{RequestCycle, RequestHead, ResponseHead, TrafficLog};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::Result;
use pingora_proxy::{HttpProxy, ProxyHttp, Session};

pub struct EavesProxy {
    traffic_log: Arc<RwLock<TrafficLog>>,
    target_port: u16,
}

pub struct MyCtx {
    pub timestamp_in: Option<chrono::DateTime<Utc>>,
    pub request_head: Option<RequestHead>,
    pub request_body: Vec<u8>,
    pub timestamp_out: Option<chrono::DateTime<Utc>>,
    pub response_head: Option<ResponseHead>,
    pub response_body: Vec<u8>,
}

fn header_mapper((name, val): (&HeaderName, &HeaderValue)) -> (String, String) {
    (
        name.as_str().to_owned(),
        String::from_utf8_lossy(val.as_bytes()).to_string(),
    )
}

#[async_trait]
impl ProxyHttp for EavesProxy {
    type CTX = MyCtx;
    fn new_ctx(&self) -> Self::CTX {
        MyCtx {
            timestamp_in: None,
            request_head: None,
            request_body: vec![],
            timestamp_out: None,
            response_head: None,
            response_body: vec![],
        }
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        ctx.timestamp_in = Some(Utc::now());
        if let Ok(Some(b)) = session.read_request_body().await {
            ctx.request_body = b.into()
        };
        let head = session.req_header();
        ctx.request_head = Some(RequestHead {
            method: head.method.as_str().into(),
            uri: head.uri.to_string(),
            headers: head.headers.iter().map(header_mapper).collect::<Vec<_>>(),
        });
        Ok(false)
    }

    async fn upstream_peer(&self, _: &mut Session, _ctx: &mut Self::CTX) -> Result<Box<HttpPeer>> {
        let addr = ("127.0.0.1", self.target_port);
        Ok(Box::new(HttpPeer::new(addr, false, "nada".to_string())))
    }

    async fn response_filter(
        &self,
        _session: &mut Session,
        head: &mut pingora_http::ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        ctx.response_head = Some(ResponseHead {
            status: head.status.into(),
            headers: head.headers.iter().map(header_mapper).collect::<Vec<_>>(),
        });
        Ok(())
    }

    fn response_body_filter(
        &self,
        _session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> Result<Option<std::time::Duration>>
    where
        Self::CTX: Send + Sync,
    {
        if let Some(b) = body {
            ctx.response_body.extend(&b[..]);
        }
        if end_of_stream {
            self.traffic_log.write().requests.push(RequestCycle {
                timestamp_in: ctx.timestamp_in.unwrap(),
                request_head: ctx.request_head.take().unwrap(),
                request_body: std::mem::take(&mut ctx.request_body),
                timestamp_out: Utc::now(),
                response_head: ctx.response_head.take().unwrap(),
                response_body: std::mem::take(&mut ctx.response_body),
            });
        }

        Ok(None)
    }
}

pub fn configure_eaves_proxy(
    conf: &Arc<pingora::server::configuration::ServerConf>,
    target_port: u16,
    traffic_log: Arc<RwLock<TrafficLog>>,
) -> (Service<HttpProxy<EavesProxy>>, u16) {
    let mut my_proxy = pingora_proxy::http_proxy_service(
        conf,
        EavesProxy {
            traffic_log,
            target_port,
        },
    );
    my_proxy.add_tcp("127.0.0.1:6190");
    (my_proxy, 6190)
}
