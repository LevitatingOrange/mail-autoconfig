use std::{fmt, net::SocketAddr};

use eyre::Result;
use serde::{Deserialize, Serialize};
use std::{fmt::Display, path::Path};
use tokio::fs::read_to_string;
use tracing::info;

#[derive(Deserialize, Serialize, PartialEq, Debug)]
pub struct Config {
    pub domains: Vec<Domain>,
    // [NOTE]: A change of this value after server start (with a reload) will have no effect!
    pub socket_address: SocketAddr, 
    pub template_path: String,
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
    pub ssl_cert: String,
    pub ssl_chain: String,
    pub ssl_key: String,
    pub display_name: String,
    pub display_short_name: String,
    pub allowed_hosts: Vec<String>,
    pub smtp: ServerConfig,
    pub imap: ServerConfig,
}

#[derive(Deserialize, Serialize, PartialEq, Debug)]
pub struct ServerConfig {
    host: String,
    port: u16,
    socket_type: SocketType,
}

#[derive(Deserialize, Serialize, PartialEq, Debug)]
enum SocketType {
    Plain,
    SSL,
    StartTLS,
}

impl Display for SocketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain => write!(f, "plain"),
            Self::SSL => write!(f, "SSL"),
            Self::StartTLS => write!(f, "STARTTLS"),
        }
    }
}
