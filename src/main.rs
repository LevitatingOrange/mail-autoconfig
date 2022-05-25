use std::net::SocketAddr;
use std::{convert::Infallible, sync::Arc};

use clap::{Parser, Subcommand};
use eyre::Result;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use tokio::signal;

use crate::global_state::GlobalState;

mod config;
mod global_state;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    /// This programs config file location
    #[clap(short, long, default_value_t = String::from("config.toml"))]
    config_file: String,

    #[clap(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// Run the server
    Run,
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    println!("signal received, starting graceful shutdown");
}

async fn service(global_state: Arc<GlobalState>, req: Request<Body>) -> Result<Response<Body>> {
    let global_state = global_state.load();
    Ok(Response::new(global_state.config.message.to_owned().into()))
}

async fn run(global_state: Arc<GlobalState>) -> Result<()> {
    println!("Hello, you can gracefully stop this program with Ctrl-C. Reload its config by sending the SIGUSR1 signal.");
    // TODO
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

    let make_service = make_service_fn(move |_conn| {
        let global_state = global_state.clone();
        async move {
            Ok::<_, eyre::Report>(service_fn(move |req| {
                let global_state = global_state.clone();
                async move { service(global_state, req).await }
            }))
        }
    });

    let server = Server::bind(&addr).serve(make_service).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    let cli = Cli::parse();
    let config_path = cli.config_file.into();
    let global_state = GlobalState::new(config_path).await?;

    tokio::select! {
    _ = shutdown_signal() => {
        println!("Bye!");
    }
    result = match cli.command {
        Commands::Run => run(global_state),
    } => {
            result?;
    }
    }
    Ok(())
}
