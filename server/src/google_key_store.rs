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
    loop {
        info!("updating store");
        match refresh_token(https_client.clone()).await {
            Ok((keys, max_age)) => {
                {
                    let mut w = key_store.write();
                    *w = keys;
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

async fn refresh_token(
    https_client: HttpsClient,
) -> Result<(HashMap<String, DecodingKey>, Duration)> {
    let res = https_client
        .get(Uri::from_static(
            "https://www.googleapis.com/oauth2/v3/certs",
        ))
        .await
        .context("Could not retrieve google jwt keys")?;

    let cc = res
        .headers()
        .get("cache-control")
        .context("Could not find cache control header")?
        .clone();

    let ser = hyper::body::to_bytes(res).await?;
    let kd: KeyData = serde_json::from_slice(&ser)?;

    let re = Regex::new(r"max-age=(\d*),")?;
    let cap = re
        .captures(cc.to_str()?)
        .ok_or(anyhow!("Could not find max age in cache control header"))?;
    let max_age = cap[1].parse::<u64>()?;

    let keys: HashMap<String, DecodingKey> = kd
        .keys
        .into_iter()
        .map(|key| Ok((key.kid, DecodingKey::from_rsa_components(&key.n, &key.e)?)))
        .collect::<Result<HashMap<String, DecodingKey>>>()
        .context("Could not get keys from google response")?;
    Ok((keys, Duration::from_secs(max_age)))
}
