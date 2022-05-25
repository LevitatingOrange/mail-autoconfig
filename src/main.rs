use std::net::SocketAddr;
use std::{convert::Infallible, sync::Arc};

use clap::{Parser, Subcommand};
use eyre::Result;
use hyper::StatusCode;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use tokio::signal;
use tracing::error;

use crate::global_state::GlobalState;

mod config;
mod global_state;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Cli {
    /// This programs config file location
    #[clap(short, long, default_value_t = String::from("config.toml"))]
    config: String,

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

async fn serve(global_state: Arc<GlobalState>, req: Request<Body>) -> Result<Response<Body>> {
    let global_state = global_state.load();
    let host = req.headers()[hyper::header::HOST].to_str()?;
    let host = host.split_once(":").map(|(s, _)| s).unwrap_or(host);
    if let Some(domain_idx) = global_state.host_map.get(host) {
        let domain = &global_state.config.domains[*domain_idx];
        match req.uri().path() {
            "/mail/config-v1.1.xml" => {
                Ok(Response::new(format!("Your email hostname: {}", domain.email_domain).into()))
            }
            _ => {
                Ok(Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty())?)
            }
        }

    } else {
        Ok(Response::builder().status(StatusCode::FORBIDDEN).body(Body::empty())?)
    }
}

async fn service(global_state: Arc<GlobalState>, req: Request<Body>) -> Result<Response<Body>, Infallible> {
    match serve(global_state, req).await {
        Ok(response) => Ok(response),
        Err(err) => {
            error!("Unexpected error while processing request: {}", err);
            Ok(Response::builder().status(StatusCode::INTERNAL_SERVER_ERROR).body(Body::empty()).unwrap())
        }
    } 
}

async fn run(global_state: Arc<GlobalState>) -> Result<()> {
    println!("Server started, you can gracefully stop this server with Ctrl-C. Reload its config by sending the SIGUSR1 signal.");
    // [TODO]: from config
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

    Server::bind(&addr).serve(make_service).with_graceful_shutdown(shutdown_signal()).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    let cli = Cli::parse();
    let config_path = cli.config.into();
    let global_state = GlobalState::new(config_path).await?;

    match cli.command {
        Commands::Run => run(global_state).await?,
    }    
    Ok(())
}
