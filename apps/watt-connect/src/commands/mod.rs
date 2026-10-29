pub mod connect;
pub mod disconnect;
pub mod install;
pub mod serve;
pub mod status;

use anyhow::{Result, anyhow, bail};
use connect_catalog::ServerEntry;
use connect_server::ConnectContext;
use connect_types::ServiceId;
use connect_vault::Secret;
use std::io::{IsTerminal, Read};

pub fn lookup(context: &ConnectContext, raw: &str) -> Result<ServerEntry> {
    let id = ServiceId::parse(raw).map_err(|error| anyhow!("{error}"))?;
    context
        .catalog
        .current()
        .catalog
        .get(&id)
        .cloned()
        .ok_or_else(|| anyhow!("`{id}` is not in the catalog (try `watt-connect search`)"))
}

/// Read a token from stdin (`--token-stdin`) or a hidden terminal prompt.
pub fn read_token(entry: &ServerEntry, from_stdin: bool) -> Result<Secret> {
    let token = if from_stdin || !std::io::stdin().is_terminal() {
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else {
        if let Some(portal) = entry.portal_url() {
            eprintln!("Get your token for {} at: {portal}", entry.display_title());
        }
        rpassword::prompt_password(format!(
            "Paste token for {} (input hidden): ",
            entry.display_title()
        ))?
    };
    let token = token.trim();
    if token.is_empty() {
        bail!("no token entered");
    }
    connect_vault::redact::register_secret(token);
    Ok(Secret::new(token))
}

pub fn print_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}
