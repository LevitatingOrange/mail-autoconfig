use std::collections::HashMap;
use std::fmt::Debug;
use std::iter::{repeat, repeat_with};
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use std::{convert::Infallible, sync::Arc};

use clap::{Parser, Subcommand};
use color_eyre::Report;
use email_address::EmailAddress;
use eyre::ensure;
use eyre::eyre;
use eyre::Result;
use futures::TryStreamExt;
use global_state::Notify;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use hyper::{Method, StatusCode, Uri};
use notify::{RecommendedWatcher, Watcher};
use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
use serde::{Deserialize, Serialize};
use tera::Context;
use tokio::io::BufReader;
use tokio::runtime::{Builder, Runtime};
use tokio::signal;
use tokio::sync::mpsc::{channel, Sender};
use tokio::task::spawn_blocking;
use tokio_util::io::StreamReader;
use tracing::{debug, error, info, warn};
use util::get_email_from_request;
use uuid::Uuid;

use crate::config::Domain;
use crate::global_state::GlobalState;

mod config;
mod global_state;
mod util;

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

#[derive(Debug, Serialize)]
struct Payload {
    uuid: Uuid,
    identifier: String,
    description: String,
    display_name: String,
    ptype: String,
    organization: String,
}

impl Payload {
    fn new_plist(domain: &Domain) -> Self {
        let uuid = Uuid::new_v4();
        let parts: Vec<&str> = domain
            .email_domain
            .split('.')
            .rev()
            .chain(std::iter::once("autoconfig"))
            .collect();
        let identifier = parts.join(".");
        let description = format!(
            "Install this profile to autoconfigure your email on {}",
            domain.email_domain
        );
        let display_name = format!("Email Autoconfiguration");
        let ptype = "Configuration".to_owned();
        let organization = format!("{} mail provider", domain.email_domain);
        Self {
            uuid,
            identifier,
            description,
            display_name,
            ptype,
            organization,
        }
    }
    fn new_domain(domain: &Domain) -> Self {
        let mut this = Self::new_plist(domain);
        this.ptype = "com.apple.mail.managed".to_owned();
        this.description = domain.display_name.to_owned();
        this.display_name = domain.display_short_name.to_owned();
        this
    }
}

fn get_mails(uri: &Uri, email_domain: &str) -> Result<Vec<String>> {
    let mut emails = Vec::new();
    for (key, value) in
        form_urlencoded::parse(uri.query().ok_or(eyre!("query missing"))?.as_bytes())
    {
        ensure!(key == "email", "only email query keys are allowed!");
        let parsed = EmailAddress::from_str(&value)?;
        ensure!(
            parsed.domain() == email_domain,
            "email {} does not belong to this server",
            value
        );
        emails.push(parsed.to_string());
    }
    Ok(emails)
}

async fn serve(global_state: Arc<GlobalState>, req: Request<Body>) -> Result<Response<Body>> {
    let global_state = global_state.load();
    let host = req.headers()[hyper::header::HOST].to_str()?;
    let host = host.split_once(":").map(|(s, _)| s).unwrap_or(host);
    if let Some(domain_idx) = global_state.host_map.get(host).map(|s| *s) {
        let domain = &global_state.config.domains[domain_idx];
        let mut context = Context::new();
        context.insert("domain", &domain);
        match &req.uri().path().to_lowercase()[..] {
            "/generate_profile" => {
                if req.method() == Method::GET {
                    let rendered = global_state
                        .templates
                        .render("apple_email.html", &context)?;

                    let response = Response::builder().header("Content-Type", "text/html");
                    Ok(response.body(rendered.into())?)
                } else {
                    Ok(Response::builder()
                        .status(StatusCode::METHOD_NOT_ALLOWED)
                        .body(Body::empty())?)
                }
            }
            "/email.mobileconfig" => {
                // Apple Mail
                match *req.method() {
                    Method::GET => {
                        let emails = match get_mails(req.uri(), &domain.email_domain) {
                            Ok(v) => v,
                            Err(err) => {
                                return Ok(Response::builder()
                                    .status(StatusCode::BAD_REQUEST)
                                    .body(format!("Error: {:#}", err).into())?);
                            }
                        };
                        debug!("Got emails: {:?}", emails);
                        context.insert("plist_payload", &Payload::new_plist(domain));
                        let payloads: HashMap<String, Payload> = emails
                            .into_iter()
                            .zip(repeat_with(|| Payload::new_domain(domain)))
                            .collect();
                        context.insert("payloads", &payloads);

                        //context.insert("domain_payload", &Payload::new_domain(domain));
                        //context.insert("email_address", &query.email);
                        let rendered_config = global_state
                            .templates
                            .render("apple_config.plist", &context)?;
                        let global_state = global_state.clone();
                        let signed = spawn_blocking(move || -> Result<Vec<u8>> {
                            let domain = &global_state.config.domains[domain_idx];
                            let certs = global_state
                                .cert_map
                                .get(&domain.email_domain)
                                .ok_or(eyre!("No cert for domain {}", domain.email_domain))?;
                            let singed = Pkcs7::sign(
                                &certs.cert,
                                &certs.key,
                                &certs.chain,
                                rendered_config.as_bytes(),
                                Pkcs7Flags::empty(),
                            )?
                            .to_der()?;
                            Ok(singed)
                        })
                        .await??;

                        let response = Response::builder().header("Content-Type", "application/pkcs7-mime; smime-type=signed-data; name=email.mobileconfig").header("Content-Disposition", "attachment; filename=email.mobileconfig");
                        Ok(response.body(signed.into())?)
                    }
                    _ => Ok(Response::builder()
                        .status(StatusCode::METHOD_NOT_ALLOWED)
                        .body(Body::empty())?),
                }
            }

            "/mail/config-v1.1.xml" => {
                // Thunderbird
                if req.method() == Method::GET {
                    let rendered_config = global_state
                        .templates
                        .render("thunderbolt_config.xml", &context)?;

                    let response = Response::builder().header("Content-Type", "text/xml");
                    Ok(response.body(rendered_config.into())?)
                } else {
                    Ok(Response::builder()
                        .status(StatusCode::METHOD_NOT_ALLOWED)
                        .body(Body::empty())?)
                }
            }
            "/autodiscover/autodiscover.xml" => {
                // Microsoft mail
                if req.method() == Method::POST {
                    let buf_read =
                        BufReader::new(StreamReader::new(req.into_body().map_err(|err| {
                            error!("Request stream err: {}", err);
                            tokio::io::Error::new(tokio::io::ErrorKind::UnexpectedEof, "eof")
                        })));
                    let email = get_email_from_request(buf_read).await?;
                    context.insert("email", &email);
                    let rendered_config = global_state
                        .templates
                        .render("microsoft_config.xml", &context)?;

                    let response = Response::builder().header("Content-Type", "text/xml");
                    Ok(response.body(rendered_config.into())?)
                } else {
                    Ok(Response::builder()
                        .status(StatusCode::METHOD_NOT_ALLOWED)
                        .body(Body::empty())?)
                }
            }
            _ => Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())?),
        }
    } else {
        Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::empty())?)
    }
}

async fn service(
    global_state: Arc<GlobalState>,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    match serve(global_state, req).await {
        Ok(response) => Ok(response),
        Err(err) => {
            error!("Unexpected error while processing request: {:#}", err);
            Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap())
        }
    }
}

async fn run(global_state: Arc<GlobalState>) -> Result<()> {
    println!("Server started, you can gracefully stop this server with Ctrl-C. Reload its config by sending the SIGUSR1 signal.");
    let gs = global_state.clone();
    let make_service = make_service_fn(move |_conn| {
        let gs = gs.clone();
        async move {
            Ok::<_, eyre::Report>(service_fn(move |req| {
                let global_state = gs.clone();
                async move { service(global_state, req).await }
            }))
        }
    });

    let global_state = global_state.load();
    let socket_addr = global_state.config.socket_address.clone();
    // drop here so that the first config does not have to live in memory indefenitely after a reload
    drop(global_state);

    Server::bind(&socket_addr)
        .serve(make_service)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn watch_for_changes(
    rt: Runtime,
    send: Sender<Notify>,
    watch_path: impl AsRef<Path> + Debug,
) -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_secs(2))?;
    watcher.watch(&watch_path, notify::RecursiveMode::Recursive)?;
    info!(
        "Watching for changes on {:#?}. Will reload state after change",
        watch_path
    );
    loop {
        let ev = rx.recv()?;
        debug!("Notify event was: {:?}", ev);
        // ignore notify events as we read files only after a reload anyways
        match ev {
            notify::DebouncedEvent::NoticeWrite(_) | notify::DebouncedEvent::NoticeRemove(_) => {
                continue
            }
            notify::DebouncedEvent::Error(error, path) => {
                warn!("File watch error: {:#}, in file: {:?}", error, path);
                continue;
            }
            _ => {}
        }
        info!("Files have changed, issuing reload request...");
        // Sadly have to re-clone here
        let send = send.clone();
        rt.block_on(async move {
            send.send(Notify::Reload).await?;
            Ok::<(), Report>(())
        })?;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    let cli = Cli::parse();
    let config_path = cli.config.into();
    let (send, recv) = channel(1);
    let global_state = GlobalState::new(config_path, Some(recv)).await?;
    let gs = global_state.load();

    // Watch for changes and reload server (mainly for cert changes)
    if let Some(watch_path) = &gs.config.watch_path {
        let watch_path = watch_path.to_owned();
        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        std::thread::spawn(move || {
            if let Err(err) = watch_for_changes(rt, send, watch_path) {
                warn!(
                    "File watching error: {:#}, reload by file change is disabled from now on",
                    err
                );
            }
        });
    }

    match cli.command {
        Commands::Run => run(global_state).await?,
    }
    Ok(())
}
