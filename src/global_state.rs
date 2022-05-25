use crate::config::Config;
use arc_swap::{ArcSwap, Guard};
use eyre::Result;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::signal::unix::{signal, SignalKind};
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
                    state_changed = new_state.as_ref() != old_state.as_ref(),
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

#[derive(Debug, PartialEq)]
pub struct GlobalStateData {
    pub config: Config,
}

impl GlobalStateData {
    async fn new(config_path: &Path) -> Result<Self> {
        let config = Config::load(config_path).await?;
        Ok(Self { config })
    }
}
