use std::io::BufRead;
use std::time::Duration;

use regex::Regex;

use hyper::Client;
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
    inner: std::process::Child,
}
impl Drop for ChildWrapper {
    fn drop(&mut self) {
        self.inner.kill().expect("command wasn't running");
    }
}
impl ChildWrapper {
    fn new(mut cmd: std::process::Command, args: &[&str]) -> ChildWrapper {
        ChildWrapper {
            inner: cmd
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .expect("Failed to start command"),
        }
    }
    fn wait_for_log_pattern(&mut self, pattern: Regex, capture_index: usize) -> String {
        println!("Searching for {:?}", pattern);
        let stdout = self.inner.stdout.as_mut().unwrap();
        for line in std::io::BufReader::new(stdout).lines() {
            println!("Read: {:?}", line);
            if let Some(cap) = pattern.captures(&line.unwrap()) {
                println!("capture! {:?}", cap);
                return cap[capture_index].to_string();
            }
        }
        panic!("Did not find '{:?}' in logs", pattern);
    }
}

#[tokio::test]
async fn test_sg_binary() {
    color_eyre::install().unwrap();
    let mut server = ChildWrapper::new(get_test_bin("sg_server"), &[]);
    let re = Regex::new(r"Starting Quic server on").unwrap();
    server.wait_for_log_pattern(re, 0);

    let mut client = ChildWrapper::new(get_test_bin("storm_grok"), &["http", "4040", "-d"]);
    let re = Regex::new(r"curl (http://[^.]*.localhost:3000)").unwrap();
    let url = client.wait_for_log_pattern(re, 1);

    println!("{:?}", url);
    let http_client = make_http_client();
    let resp = http_client.get(url.parse().unwrap()).await.unwrap();
    assert_eq!(resp.status(), 200);

    let (wasm_resp, js_resp) = tokio::join!(
        http_client.get(url.parse().unwrap()),
        http_client.get(url.parse().unwrap()),
    );
    assert_eq!(wasm_resp.unwrap().status(), 200);
    assert_eq!(js_resp.unwrap().status(), 200);
}
