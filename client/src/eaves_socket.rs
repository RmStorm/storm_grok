use chrono::prelude::*;
use chrono::serde::ts_milliseconds;
use serde::Serialize;
use std::fmt;
use std::pin::Pin;
use tokio::io::{AsyncRead, AsyncWrite};

use parking_lot::RwLock;
use std::sync::Arc;

use base64_serde::base64_serde_type;

base64_serde_type!(Base64Standard, base64::engine::general_purpose::STANDARD);

#[derive(Debug, Clone, Serialize)]
pub struct TrafficLog {
    pub requests: Vec<RequestCycle>,
    pub logged_conns: Vec<LoggedConnection>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoggedConnection {
    pub traffic_in: Vec<ObservedBytes>,
    pub traffic_out: Vec<ObservedBytes>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SerializableRequest {
    pub method: String,
    pub uri: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SerializableResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestCycle {
    #[serde(with = "ts_milliseconds")]
    pub timestamp_in: DateTime<Utc>,
    pub head_in: SerializableRequest,
    #[serde(with = "Base64Standard")]
    pub body_in: Vec<u8>,
    #[serde(with = "ts_milliseconds")]
    pub timestamp_out: DateTime<Utc>,
    pub head_out: SerializableResponse,
    #[serde(with = "Base64Standard")]
    pub body_out: Vec<u8>,
}

#[derive(Serialize, Clone)]
pub struct ObservedBytes {
    #[serde(with = "ts_milliseconds")]
    pub timestamp: DateTime<Utc>,
    pub bytes: String,
}

impl fmt::Debug for ObservedBytes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "\nWritten {} bytes at {:?}",
            self.bytes.len(),
            self.timestamp
        )
    }
}

pub struct EavesSocket<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> {
    pub reader: Pin<Box<R>>,
    pub writer: Pin<Box<W>>,
    pub traffic_log: Arc<RwLock<TrafficLog>>,
    pub conn_index: usize,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> AsyncWrite for EavesSocket<R, W> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        {
            let ob = ObservedBytes {
                timestamp: Utc::now(),
                bytes: String::from_utf8_lossy(buf).to_string(),
            };

            let logged_conn = &mut self.traffic_log.write().logged_conns[self.conn_index];
            logged_conn.traffic_out.push(ob);
        }
        self.writer.as_mut().poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        self.writer.as_mut().poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        self.writer.as_mut().poll_shutdown(cx)
    }
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> AsyncRead for EavesSocket<R, W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let poll_result = self.reader.as_mut().poll_read(cx, buf);
        let ob = ObservedBytes {
            timestamp: Utc::now(),
            bytes: String::from_utf8_lossy(buf.filled()).to_string(),
        };
        let logged_conn = &mut self.traffic_log.write().logged_conns[self.conn_index];
        logged_conn.traffic_in.push(ob);
        poll_result
    }
}
