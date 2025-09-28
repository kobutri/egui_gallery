use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ImageResponse {
    pub id: i64,
    pub author: String,
    pub width: i32,
    pub height: i32,
    #[serde(serialize_with = "hex::serde::serialize")]
    #[serde(deserialize_with = "hex::serde::deserialize")]
    pub hash: Vec<u8>,
    pub path: String,
    pub url: String,
    pub mime_type: String,
}
