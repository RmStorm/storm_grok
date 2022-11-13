use regex::Regex;

use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server};
use std::{
    collections::HashMap,
    convert::Infallible,
    io::{BufRead, ErrorKind, Read},
    net::TcpListener,
    time::Duration,
};

use hyper_timeout::TimeoutConnector;

fn get_test_bin(bin_name: &str) -> std::process::Command {
    let mut path = get_test_bin_dir();
    path.push(bin_name);
    path.set_extension(std::env::consts::EXE_EXTENSION);
    assert!(path.exists());
    std::process::Command::new(path.into_os_string())
}

fn get_test_bin_dir() -> std::path::PathBuf {
    let current_exe =
        std::env::current_exe().expect("Failed to get the path of the integration test binary");
    let current_dir = current_exe
        .parent()
        .expect("Failed to get the directory of the integration test binary");
    let test_bin_dir = current_dir
        .parent()
        .expect("Failed to get the binary folder");
    test_bin_dir.to_owned()
}

type TimeoutClient = Client<TimeoutConnector<hyper::client::HttpConnector>, hyper::Body>;
fn make_http_client() -> TimeoutClient {
    let h = hyper::client::HttpConnector::new();
    let mut connector = TimeoutConnector::new(h);
    connector.set_connect_timeout(Some(Duration::from_secs(1)));
    connector.set_read_timeout(Some(Duration::from_secs(1)));
    connector.set_write_timeout(Some(Duration::from_secs(1)));
    Client::builder().build(connector)
}

struct ChildWrapper {
    name: &'static str,
    inner: std::process::Child,
}
impl Drop for ChildWrapper {
    fn drop(self: &mut ChildWrapper) {
        println!("Killing {:?}", self.name);
        self.inner.kill().expect("command wasn't running");

        if let Some(mut stdout) = self.inner.stdout.take() {
            let mut buffer = Vec::new();
            stdout.read_to_end(&mut buffer).unwrap();
            if buffer.len() != 0 {
                println!("Printing stdout for {:?}:", self.name);
                println!("{}", std::str::from_utf8(&buffer).unwrap());
            }
        }

        if let Some(mut stderr) = self.inner.stderr.take() {
            let mut buffer = Vec::new();
            stderr.read_to_end(&mut buffer).unwrap();
            if buffer.len() != 0 {
                println!("Printing stderr for {:?}:", self.name);
                println!("{}", std::str::from_utf8(&buffer).unwrap());
            }
        }
    }
}
impl ChildWrapper {
    fn new(cmd: &'static str, args: &[&str], envs: HashMap<&str, &str>) -> ChildWrapper {
        ChildWrapper {
            name: cmd,
            inner: get_test_bin(&cmd)
                .envs(envs)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .expect("Failed to start command"),
        }
    }
    fn wait_for_log_pattern(&mut self, pattern: Regex, capture_index: usize) -> String {
        println!("Searching for '{:?}' in logs of {:?}", pattern, self.name);
        let stdout = self.inner.stdout.as_mut().unwrap();
        for line in std::io::BufReader::new(stdout).lines() {
            let l = line.unwrap();
            let ll = l.as_str();
            println!("line: {}", ll);
            if let Some(cap) = pattern.captures(ll) {
                return cap[capture_index].to_string();
            }
        }
        panic!("Did not find '{:?}' in logs", pattern);
    }
}

fn listen_available_port(start_port: u16) -> TcpListener {
    for port in start_port..65535 {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => return l,
            Err(error) => match error.kind() {
                ErrorKind::AddrInUse => {}
                other_error => panic!(
                    "Encountered errr while setting up tcp server: {:?}",
                    other_error
                ),
            },
        }
    }
    panic!("No ports available")
}

async fn handle(_: Request<Body>) -> Result<Response<Body>, Infallible> {
    dbg!("HANDLED!!");
    Ok(Response::new("Hello, World!".into()))
}

async fn start_server_and_client(quic_port: &str, http_port: &str) -> (ChildWrapper, ChildWrapper) {
    let make_svc = make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(handle)) });
    let listener = listen_available_port(2020);
    let port = listener.local_addr().unwrap().port().to_string();
    let server = Server::from_tcp(listener).unwrap().serve(make_svc);
    tokio::spawn(async move { server.await });

    let envs = HashMap::from([
        ("SG__LOG__LEVEL", "info"),
        ("SG__LOG__FORMAT", "compact"),
        ("SG__SERVER__QUIC_PORT", quic_port),
        ("SG__SERVER__HTTP_PORT", http_port),
    ]);
    let mut server = ChildWrapper::new("sg_server", &[], envs.clone());
    let re = Regex::new(r"Starting Quic server on").unwrap();
    server.wait_for_log_pattern(re, 0);

    let client = ChildWrapper::new("storm_grok", &["http", &port, "-d"], envs);
    (server, client)
}

#[tokio::test]
async fn tunnel_single_request_1() {
    let (_server, mut client) = start_server_and_client("5000", "3000").await;
    let re = Regex::new(r"curl (http://[^.]*.localhost:\d\d\d\d)").unwrap();
    let url = client.wait_for_log_pattern(re, 1);
    let http_client = make_http_client();

    let resp = http_client.get(url.parse().unwrap()).await;
    assert_eq!(resp.unwrap().status(), 200);
}

#[tokio::test]
async fn tunnel_single_request_2() {
    let (_server, mut client) = start_server_and_client("5001", "3001").await;
    let re = Regex::new(r"curl (http://[^.]*.localhost:\d\d\d\d)").unwrap();
    let url = client.wait_for_log_pattern(re, 1);

    let http_client = make_http_client();

    let resp = http_client.get(url.parse().unwrap()).await;
    assert_eq!(resp.unwrap().status(), 200);
}

#[tokio::test]
async fn tunnel_concurrent_requests() {
    let (_server, mut client) = start_server_and_client("5002", "3002").await;
    let re = Regex::new(r"curl (http://[^.]*.localhost:\d\d\d\d)").unwrap();
    let url = client.wait_for_log_pattern(re, 1);
    println!("url: {:?}", url);

    let http_client = make_http_client();

    let (wasm_resp, js_resp) = tokio::join!(
        http_client.get(url.parse().unwrap()),
        http_client.get(url.parse().unwrap()),
    );
    assert_eq!(wasm_resp.unwrap().status(), 200);
    assert_eq!(js_resp.unwrap().status(), 200);
}
