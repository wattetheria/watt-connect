//! `watt-connect`: one MCP server for the user's agent, plus a small CLI for
//! the steps that must not go through the agent (entering tokens).

mod cli;
mod commands;
mod config;
mod connect_web;
mod logging;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Command};
use config::{AppPaths, TomlConnectionStore, read_config};
use connect_catalog::{CatalogConfig, CatalogStore, DEFAULT_CATALOG_URL};
use connect_installer::InstallerEnv;
use connect_server::ConnectContext;
use connect_upstream::UpstreamPool;
use connect_vault::{VaultPreference, open_vault};
use std::sync::Arc;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::resolve()?;
    let command = cli.command.unwrap_or(Command::Serve);
    match &command {
        Command::Serve => logging::init("info", Some(&paths.log_dir), true),
        _ => logging::init("warn", None, true),
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start async runtime")?;
    runtime.block_on(async move {
        let serving = matches!(command, Command::Serve);
        let context = build_context(&paths, serving)?;
        match command {
            Command::Serve => Box::pin(commands::serve::run(context)).await,
            Command::Connect(args) => commands::connect::run(&context, args).await,
            Command::Disconnect(args) => commands::disconnect::run(&context, args).await,
            Command::Status => commands::status::run(&context).await,
            Command::Install(args) => commands::install::run(&context, args).await,
            Command::Uninstall(args) => commands::install::uninstall(&context, &args),
            Command::Search(args) => {
                commands::status::search(&context, &args);
                Ok(())
            }
        }
    })
}

/// Wire up catalog, vault, connection store and upstream pool.
fn build_context(paths: &AppPaths, serving: bool) -> Result<Arc<ConnectContext>> {
    let settings = read_config(&paths.config_file())?;
    let url = std::env::var("WATT_CONNECT_CATALOG_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .or(settings.catalog.url)
        .unwrap_or_else(|| DEFAULT_CATALOG_URL.to_owned());
    let catalog = CatalogStore::load(CatalogConfig {
        url: url::Url::parse(&url).with_context(|| format!("invalid catalog URL {url}"))?,
        trusted_keys: connect_catalog::verify::builtin_trusted_keys(),
        cache_dir: paths.cache_dir.clone(),
        dev_catalog: std::env::var_os("WATT_CONNECT_DEV_CATALOG")
            .filter(|value| !value.is_empty())
            .map(Into::into),
    })
    .context("load catalog")?;
    for entry in &catalog.current().catalog.services {
        connect_server::target::register_redaction(entry);
    }

    let preference: VaultPreference = std::env::var("WATT_CONNECT_VAULT")
        .unwrap_or_default()
        .parse()
        .map_err(anyhow::Error::msg)?;
    let vault = open_vault(preference, &paths.data_dir).context("open credential store")?;

    let mut context = ConnectContext {
        catalog: Arc::new(catalog),
        vault: Arc::from(vault),
        connections: Arc::new(TomlConnectionStore::new(paths.config_file())),
        upstream: Arc::new(UpstreamPool::default()),
        installer_env: InstallerEnv::from_process(paths.home_dir.clone()),
        link_provider: None,
        command: std::env::var("WATT_CONNECT_COMMAND")
            .unwrap_or_else(|_| "npx watt-connect".into()),
    };
    if serving && std::env::var_os("WATT_CONNECT_NO_WEB").is_none() {
        let shared = Arc::new(connect_web::WebConnect::new());
        context.link_provider = Some(shared.clone());
        let context = Arc::new(context);
        shared.attach(Arc::downgrade(&context));
        return Ok(context);
    }
    Ok(Arc::new(context))
}
