use eyre::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs::read_to_string;
use tracing::info;

#[derive(Deserialize, Serialize, PartialEq, Debug)]
pub struct Config {
    pub message: String,
}

impl Config {
    pub async fn load(config_path: impl AsRef<Path>) -> Result<Self> {
        info!("Loading config...");
        let contents = read_to_string(config_path).await?;
        let config = toml::from_str(&contents)?;
        Ok(config)
    }
}
