use config::{Config, Environment, File};
use rustls::{Certificate, PrivateKey};

use serde::Deserialize;
use std::{fs, io::BufReader, path::PathBuf};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Debug, Deserialize, Clone)]
pub struct Log {
    pub level: String,
    pub format: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Tls {
    pub cert_file: String,
    pub key_file: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Server {
    pub http_host: String,
    pub quic_host: String,
    pub http_port: u16,
    pub quic_port: u16,
    pub tls: Option<Tls>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthRules {
    pub jwt_key_endpoints: Vec<String>,
    pub default_allow_issuers: Vec<String>,
    pub enabled: bool,
    pub users: Vec<String>,
    pub host_domains: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub enum ENV {
    Dev,
    Prod,
}

impl From<&str> for ENV {
    fn from(env: &str) -> Self {
        match env {
            "Prod" => ENV::Prod,
            _ => ENV::Dev,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    pub server: Server,
    pub auth: AuthRules,
    pub log: Log,
    pub env: ENV,
}

fn guess_config_file() -> PathBuf {
    // All of this path guessing shit is crap 💩 but it works
    // core problem is that the path differs when running this in the integratoin test or not.
    // maybe an env var would be a better solution
    let mut config_file_dir = std::env::current_dir().unwrap().join("config");
    if !config_file_dir.is_dir() {
        config_file_dir.pop();
        if let Some(path) = config_file_dir.file_name() {
            if path.to_str().unwrap() == "tests" {
                config_file_dir.pop();
            }
        }
        config_file_dir.push("server/config");
    }
    assert!(config_file_dir.is_dir());
    config_file_dir
}

impl Settings {
    pub fn new() -> Self {
        let env = std::env::var("RUN_ENV").unwrap_or_else(|_| "Dev".into());

        let config_file_dir = guess_config_file();
        let config: Settings = Config::builder()
            .set_override("env", env.clone())
            .unwrap()
            .add_source(File::from(config_file_dir.join("Default.toml")))
            .add_source(File::from(config_file_dir.join(env)))
            .add_source(Environment::with_prefix("SG").separator("__"))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        let subscriber = fmt().with_env_filter(EnvFilter::try_new(&config.log.level).unwrap());
        match config.log.format.as_str() {
            "pretty" => subscriber.event_format(fmt::format().pretty()).init(),
            "compact" => subscriber.event_format(fmt::format().compact()).init(),
            _ => subscriber.event_format(fmt::format()).init(), // default formatter = 'full'
        };
        config
    }

    pub fn get_certs_and_key(&self) -> (Vec<Certificate>, PrivateKey) {
        if self.env == ENV::Prod {
            let certs = rustls_pemfile::certs(&mut BufReader::new(
                fs::File::open(&self.server.tls.as_ref().unwrap().cert_file).unwrap(),
            ))
            .expect("cannot parse certificate .pem file")
            .iter()
            .map(|v| Certificate(v.clone()))
            .collect();

            let key = match rustls_pemfile::read_one(&mut BufReader::new(
                fs::File::open(&self.server.tls.as_ref().unwrap().key_file).unwrap(),
            ))
            .expect("cannot parse private key .pem file")
            {
                Some(rustls_pemfile::Item::RSAKey(key)) => PrivateKey(key),
                Some(rustls_pemfile::Item::PKCS8Key(key)) => PrivateKey(key),
                Some(rustls_pemfile::Item::ECKey(key)) => PrivateKey(key),
                Some(_) => panic!("No good key found!"),
                None => panic!("No good key found!"),
            };
            (certs, key)
        } else {
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
            let certs = vec![rustls::Certificate(cert.serialize_der().unwrap())];
            let key = rustls::PrivateKey(cert.serialize_private_key_der());
            (certs, key)
        }
    }
}
