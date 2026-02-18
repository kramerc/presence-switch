use std::collections::HashMap;

use lazy_static::lazy_static;
use serde::Deserialize;
use tokio::sync::RwLock;

const BASE_URL: &str = "https://discord.com/api/v9";

type Data = HashMap<String, ApplicationRpcData>;
lazy_static! {
    static ref CACHE: RwLock<Data> = RwLock::new(HashMap::new());
}

#[derive(Clone, Deserialize)]
pub struct ApplicationRpcData {
    pub name: String,
}

/// Fetches and caches metadata of an RPC application
pub async fn application_rpc(client_id: &str) -> Result<ApplicationRpcData, reqwest::Error> {
    // Read cache
    {
        let cache = CACHE.read().await;
        let cached_data = cache.get(client_id);
        if cached_data.is_some() {
            return Ok(cached_data.unwrap().clone());
        }
    }

    let data = reqwest::get(format!(
        "{}/oauth2/applications/{}/rpc",
        BASE_URL, client_id
    ))
    .await?
    .json::<ApplicationRpcData>()
    .await?;

    // Write cache
    let mut cache = CACHE.write().await;
    cache.insert(client_id.to_string(), data.clone());

    Ok(data)
}
