use eyre::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs::read_to_string;
use tracing::info;

#[derive(Deserialize, Serialize, PartialEq, Debug)]
pub struct Config {
    pub domains: Vec<Domain>
}

impl Config {
    pub async fn load(config_path: impl AsRef<Path>) -> Result<Self> {
        info!("Loading config...");
        let contents = read_to_string(config_path).await?;
        let config = toml::from_str(&contents)?;
        Ok(config)
    }
}


#[derive(Deserialize, Serialize, PartialEq, Debug)]
pub struct Domain {
    pub email_domain: String,
    pub allowed_hosts: Vec<String>,
    pub smtp: ServerConfig,
    pub imap: ServerConfig
}

#[derive(Deserialize, Serialize, PartialEq, Debug)]
pub struct ServerConfig {
    host: String,
    port: u16,
    ssl: bool
}
