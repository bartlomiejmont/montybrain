//! Morwisko MCP server: exposes the Morwisko vault to coding agents over stdio.
//!
//! stdout is the protocol channel; all logging goes to stderr.

mod render;
mod resolve;
mod server;

use morwisko_core::Vault;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    // Read-only mode is decided first: a read-only server opens an existing
    // vault and never creates anything (no skeleton, no lock dir). With no
    // vault it still starts, and every tool answers with the reason.
    let read_only = server::read_only_from_env(std::env::var(server::ENV_READONLY).ok().as_deref());
    let morwisko = if read_only {
        match server::open_read_only(Vault::default_root()) {
            Ok(vault) => {
                tracing::info!(
                    "morwisko-mcp serving vault {} (read-only)",
                    vault.root().display()
                );
                server::Morwisko::new(vault)
            }
            Err(problem) => {
                tracing::error!("{problem}");
                server::Morwisko::unavailable(problem)
            }
        }
        .with_read_only(true)
    } else {
        let vault = match Vault::init_default() {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("cannot open the Morwisko vault: {e}");
                std::process::exit(1);
            }
        };
        tracing::info!("morwisko-mcp serving vault {}", vault.root().display());
        server::Morwisko::new(vault)
    };

    let service = match morwisko.serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("MCP handshake failed: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = service.waiting().await {
        tracing::error!("MCP session ended with an error: {e}");
    }
    // tokio's stdin reader would otherwise keep the runtime alive until EOF.
    std::process::exit(0);
}
