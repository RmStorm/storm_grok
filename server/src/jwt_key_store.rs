use crate::{HttpsClient, KeyMap};
use anyhow::{anyhow, Context, Result};
use hyper::Uri;
use jsonwebtoken::DecodingKey;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use tokio::time::{sleep, Duration};
use tracing::log::{error, info};

#[derive(Debug, Deserialize)]
struct Key {
    e: String,
    n: String,
    // r#use: String,
    // kty: String,
    kid: String,
    // alg: String,
}

#[derive(Debug, Deserialize)]
struct KeyData {
    keys: Vec<Key>,
}

pub async fn refresh_loop(key_store: KeyMap, https_client: HttpsClient) {
    let endpoints = vec!["https://www.googleapis.com/oauth2/v3/certs"];

    let mut tasks = Vec::new();

    for endpoint in endpoints {
        let key_store = key_store.clone();
        let https_client = https_client.clone();
        let endpoint = endpoint.to_string();
        tasks.push(tokio::spawn(async move {
            refresh_loop_for_endpoint(key_store, https_client, endpoint).await;
        }));
    }

    futures::future::join_all(tasks).await;
}
pub async fn refresh_loop_for_endpoint(
    key_store: KeyMap,
    https_client: HttpsClient,
    endpoint: String,
) {
    loop {
        info!("updating store for {}", endpoint);
        match refresh_keys(https_client.clone(), &endpoint).await {
            Ok((keys, max_age)) => {
                {
                    let mut w = key_store.write();
                    for (kid, key) in keys {
                        w.insert(kid, key);
                    }
                }
                info!("Refreshed tokens, refreshing again in {:?}", max_age);
                sleep(max_age).await;
            }
            Err(e) => {
                error!("Encountered error while refreshing keys '{:?}'", e);
                sleep(Duration::from_millis(10000)).await;
            }
        }
    }
}

async fn refresh_keys(
    https_client: HttpsClient,
    endpoint: &str,
) -> Result<(HashMap<String, DecodingKey>, Duration)> {
    let res = https_client
        .get(Uri::try_from(endpoint).unwrap())
        .await
        .context("Could not retrieve google jwt keys")?;
    info!("{:?}", res.headers());
    let cc = res
        .headers()
        .get("cache-control")
        .context("Could not find cache control header")?
        .clone();

    let ser = hyper::body::to_bytes(res).await?;
    let kd: KeyData = serde_json::from_slice(&ser)?;

    let re = Regex::new(r"max-age=(\d*),?")?;
    let cap = re
        .captures(cc.to_str()?)
        .ok_or_else(|| anyhow!("Could not find max age in cache control header"))?;
    let max_age = cap[1].parse::<u64>()?;

    let keys: HashMap<String, DecodingKey> = kd
        .keys
        .into_iter()
        .map(|key| Ok((key.kid, DecodingKey::from_rsa_components(&key.n, &key.e)?)))
        .collect::<Result<HashMap<String, DecodingKey>>>()
        .context("Could not get keys from google response")?;
    Ok((keys, Duration::from_secs(max_age)))
}
