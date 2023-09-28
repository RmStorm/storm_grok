use chrono::Utc;

use chrono::prelude::*;
use chrono::serde::ts_milliseconds;
use serde::{Deserialize, Serialize};

use base64_serde::base64_serde_type;

base64_serde_type!(Base64Standard, base64::engine::general_purpose::STANDARD);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficLog {
    pub requests: Vec<RequestCycle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHead {
    pub method: String,
    pub uri: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestCycle {
    #[serde(with = "ts_milliseconds")]
    pub timestamp_in: DateTime<Utc>,
    pub request_head: RequestHead,
    #[serde(with = "Base64Standard")]
    pub request_body: Vec<u8>,
    #[serde(with = "ts_milliseconds")]
    pub timestamp_out: DateTime<Utc>,
    pub response_head: ResponseHead,
    #[serde(with = "Base64Standard")]
    pub response_body: Vec<u8>,
}

impl PartialEq for RequestCycle {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp_in == other.timestamp_in
    }
}
