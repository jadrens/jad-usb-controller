mod api;
mod at;
mod cell;
mod config;
mod euicc;
mod events;
mod modem;
mod pdu;
mod status;
mod store;
mod tlv;
mod transport;

use anyhow::Context;
use clap::Parser;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::events::EventBus;
use crate::modem::Modem;
use crate::store::Store;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        // plain text when logging to journald / a file
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn")),
        )
        .init();

    let cfg = Config::parse();
    let store = Store::open(&cfg.db).with_context(|| format!("open database {}", cfg.db.display()))?;
    let modem = Modem::new(cfg.clone(), store, EventBus::new());
    tokio::spawn(modem.clone().run());

    let app = api::router(modem)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(cfg.listen)
        .await
        .with_context(|| format!("bind {}", cfg.listen))?;
    tracing::info!("listening on http://{}", cfg.listen);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
