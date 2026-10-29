//! `watt-connect status` and `watt-connect search`.

use super::print_json;
use crate::cli::SearchArgs;
use anyhow::{Result, anyhow};
use connect_server::{ConnectContext, ConnectServer};
use serde_json::Value;
use std::sync::Arc;

pub async fn run(context: &Arc<ConnectContext>) -> Result<()> {
    let server = ConnectServer::new(context.clone());
    let result = server.run_tool("list_connections", None).await;
    let value = result
        .structured_content
        .ok_or_else(|| anyhow!("no status available"))?;
    if result.is_error == Some(true) {
        print_json(&value);
        return Ok(());
    }
    println!(
        "catalog {} ({})   credential store: {}",
        value["catalog_version"].as_str().unwrap_or("?"),
        value["catalog_source"].as_str().unwrap_or("?"),
        value["vault"].as_str().unwrap_or("?"),
    );
    let connections = value["connections"].as_array().cloned().unwrap_or_default();
    if connections.is_empty() {
        println!(
            "No services connected. Try `watt-connect search` and `watt-connect connect <service>`."
        );
        return Ok(());
    }
    println!(
        "{:<24} {:<8} {:<20} DETAIL",
        "SERVICE", "MODE", "CREDENTIAL"
    );
    for connection in connections {
        let detail = match connection["credential_status"].as_str() {
            Some("credential_invalid" | "missing") => connection["portal_url"]
                .as_str()
                .map(|portal| format!("get a new token: {portal}"))
                .unwrap_or_default(),
            _ if connection["mode"] == "direct" => format!(
                "agents: {}",
                connection["agents"]
                    .as_array()
                    .map(|agents| agents
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", "))
                    .unwrap_or_default()
            ),
            _ if connection["sandbox"] == true => "sandbox".into(),
            _ => String::new(),
        };
        println!(
            "{:<24} {:<8} {:<20} {detail}",
            connection["service"].as_str().unwrap_or("?"),
            connection["mode"].as_str().unwrap_or("?"),
            connection["credential_status"].as_str().unwrap_or("?"),
        );
    }
    Ok(())
}

pub fn search(context: &ConnectContext, args: &SearchArgs) {
    let loaded = context.catalog.current();
    let results =
        connect_catalog::search::search(&loaded.catalog, &args.query, args.category.as_deref());
    if results.is_empty() {
        println!("No services match.");
    }
    for entry in results {
        let connected = context
            .connections
            .get(&entry.name)
            .map(|record| format!(" [connected: {}]", record.mode))
            .unwrap_or_default();
        println!(
            "{:<24} {}  {}{connected}",
            entry.name,
            entry.display_title(),
            entry.description
        );
    }
}
