use crate::config::Config;
use arc_swap::{ArcSwap, Guard};
use eyre::Result;
use openssl::{stack::Stack, x509::X509, pkey::{PKey, Private}};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tera::Tera;
use tokio::{
    signal::unix::{signal, SignalKind},
    task::spawn_blocking,
};
use tracing::{error, info, instrument, warn};

/// A simple wrapper for a global state that allows for reloading of the config via a unix signal
pub struct GlobalState(ArcSwap<GlobalStateData>);

impl GlobalState {
    pub async fn new(config_path: PathBuf) -> Result<Arc<Self>> {
        let initial_state = GlobalStateData::new(&config_path).await?;
        let this = Arc::new(Self(ArcSwap::from_pointee(initial_state)));
        this.clone().install_reload_handler(config_path);
        Ok(this)
    }

    #[instrument(skip(self))]
    async fn reload_state(&self, config_path: &Path) {
        info!("Reloading global state...");
        match GlobalStateData::new(config_path).await {
            Ok(new_state_data) => {
                let new_state = Arc::new(new_state_data);
                let old_state = self.0.swap(new_state.clone());

                info!(
                    state_changed = new_state.as_ref().config != old_state.as_ref().config,
                    message = "Global state updated; succefully reloaded!"
                );
            }
            Err(error) => {
                error!(
                    state_changed = false,
                    %error,
                    message = "Global state not updated; error while reloading",
                );
            }
        }
    }

    pub fn load(&self) -> Guard<Arc<GlobalStateData>> {
        self.0.load()
    }

    fn install_reload_handler(self: Arc<Self>, config_path: PathBuf) {
        tokio::spawn(async move {
            if cfg!(unix) {
                match signal(SignalKind::user_defined1()) {
                    Ok(mut signal) => {
                        while signal.recv().await.is_some() {
                            self.reload_state(&config_path).await;
                        }
                        warn!("Signal stream has ended. State reloading is disabled from now on!");
                        std::future::pending::<()>().await;
                    }
                    Err(err) => {
                        warn!(
                            "Could not install signal handler: {}, state reloading is disabled!",
                            err
                        );
                        std::future::pending::<()>().await;
                    }
                }
            } else {
                warn!("Stae reloading is not supported on non-unix OSes");
                std::future::pending::<()>().await;
            }
        });
    }
}

pub struct Certs {
    pub cert: X509,
    pub chain: Stack<X509>, 
    pub key: PKey<Private>
}

impl Certs {
    async fn new(cert_path: impl AsRef<Path>, chain_path: impl AsRef<Path>,key_path: impl AsRef<Path>) -> Result<Self> {
        let cert_buf = tokio::fs::read(cert_path).await?;
        let cert = X509::from_pem(&cert_buf)?;
        let chain_buf = tokio::fs::read(chain_path).await?;
        let chain_stack = X509::stack_from_pem(&chain_buf)?;
        let mut chain = Stack::new()?;
        for cc in chain_stack {
            chain.push(cc)?;
        }

        let key_buf = tokio::fs::read(key_path).await?;
        let key = PKey::private_key_from_pem(&key_buf)?;

        Ok(Self {
            cert,chain,key
        })
    }
}

pub struct GlobalStateData {
    pub config: Config,
    /// Mapping of allowed domain to index
    pub host_map: HashMap<String, usize>,
    pub cert_map: HashMap<String, Certs>,

    pub templates: Tera,
}

impl GlobalStateData {
    async fn new(config_path: &Path) -> Result<Self> {
        let config = Config::load(config_path).await?;
        let mut host_map = HashMap::new();
        let mut cert_map = HashMap::new();
        for (i, domain) in config.domains.iter().enumerate() {
            for allowed_host in &domain.allowed_hosts {
                host_map.insert(allowed_host.to_owned(), i);
            }
            cert_map.insert(domain.email_domain.to_owned(), Certs::new(&domain.ssl_cert, &domain.ssl_chain, &domain.ssl_key).await?);
        }
        let template_path = config.template_path.clone();
        let templates = spawn_blocking(move || Tera::new(&template_path)).await??;
        Ok(Self {
            config,
            host_map,
            cert_map,
            templates,
        })
    }
}
