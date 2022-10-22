use chrono::prelude::*;
use parking_lot::RwLock;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncWrite;

pub struct CopyWriter<T: AsyncWrite + Unpin> {
    pub writer: Pin<Box<T>>,
    pub traffic_log: Arc<RwLock<TrafficLog>>,
    pub conn_index: usize,
    pub incoming: bool,
}

impl<T: AsyncWrite + Unpin> AsyncWrite for CopyWriter<T> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        {
            let ob = ObservedBytes {
                timestamp: Utc::now(),
                bytes: buf.clone().into(),
            };
            let logged_conn = &mut self.traffic_log.write().logged_conns[self.conn_index];
            match self.incoming {
                true => logged_conn.traffic_in.push(ob),
                false => logged_conn.traffic_out.push(ob),
            }
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

#[derive(Debug)]
pub struct TrafficLog {
    pub logged_conns: Vec<LoggedConnection>,
}
#[derive(Debug)]
pub struct LoggedConnection {
    pub traffic_in: Vec<ObservedBytes>,
    pub traffic_out: Vec<ObservedBytes>,
}
#[derive(Debug)]
pub struct ObservedBytes {
    pub timestamp: DateTime<Utc>,
    pub bytes: Vec<u8>,
}

pub fn create_logged_writers<T1: AsyncWrite + Unpin, T2: AsyncWrite + Unpin>(
    client_send: T1,
    server_send: T2,
    traffic_log: Arc<RwLock<TrafficLog>>,
) -> (CopyWriter<T1>, CopyWriter<T2>) {
    let lc = LoggedConnection {
        traffic_in: vec![],
        traffic_out: vec![],
    };
    let conn_index = traffic_log.read().logged_conns.len();
    traffic_log.write().logged_conns.push(lc);
    let client_send = CopyWriter {
        writer: Box::pin(client_send),
        traffic_log: traffic_log.clone(),
        conn_index: conn_index,
        incoming: false,
    };
    let server_send = CopyWriter {
        writer: Box::pin(server_send),
        traffic_log: traffic_log.clone(),
        conn_index: conn_index,
        incoming: true,
    };
    (client_send, server_send)
}

pub fn print_full_traffic_log(traffic_log: Arc<RwLock<TrafficLog>>) {
    for c in &traffic_log.read().logged_conns {
        println!("\n\nPrinting traffic for connn");
        for ti in &c.traffic_in {
            println!(
                "observed traffic in at: {}\n{:?}",
                ti.timestamp,
                String::from_utf8_lossy(&ti.bytes)
            );
        }
        for to in &c.traffic_out {
            println!(
                "observed traffic out at: {:?}\n{:?}",
                to.timestamp,
                String::from_utf8_lossy(&to.bytes)
            );
        }
    }
}