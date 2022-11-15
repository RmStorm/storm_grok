use chrono::prelude::*;
use chrono::serde::ts_milliseconds;
use serde::Serialize;
use std::fmt;
use std::pin::Pin;
use tokio::io::{AsyncWrite, AsyncRead};

use std::{sync::Arc};
use tracing::log::{info};
use parking_lot::RwLock;

#[derive(Debug, Serialize, Clone)]
pub struct TrafficLog {
    pub logged_conns: Vec<LoggedConnection>,
}

#[derive(Debug, Serialize, Clone)]
pub struct LoggedConnection {
    pub traffic_in: Vec<ObservedBytes>,
    pub traffic_out: Vec<ObservedBytes>,
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
            info!("we ever here?");
            let ob = ObservedBytes {
                timestamp: Utc::now(),
                bytes: String::from_utf8_lossy(buf).to_string(),
            };

            let logged_conn = &mut self.traffic_log.write().logged_conns[self.conn_index];
            logged_conn.traffic_in.push(ob);
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
        {
            info!("we hereread :{:?}", buf);
            let ob = ObservedBytes {
                timestamp: Utc::now(),
                bytes: String::from_utf8_lossy(buf.filled()).to_string(),
            };
            let logged_conn = &mut self.traffic_log.write().logged_conns[self.conn_index];
            logged_conn.traffic_out.push(ob);
        }
        self.reader.as_mut().poll_read(cx, buf)
    }
}
