//! `search_services(query, category?)`.

use super::{modes_json, opt_str_arg};
use crate::ConnectServer;
use rmcp::model::JsonObject;
use serde_json::{Value, json};

pub(super) fn run(server: &ConnectServer, args: &JsonObject) -> Value {
    let query = opt_str_arg(args, "query").unwrap_or("");
    let category = opt_str_arg(args, "category");
    let loaded = server.context().catalog.current();
    let connections = server.context().connections.list();
    let services: Vec<Value> = connect_catalog::search::search(&loaded.catalog, query, category)
        .into_iter()
        .map(|entry| {
            let record = connections
                .iter()
                .find(|record| record.service == entry.name);
            json!({
                "service": entry.name,
                "title": entry.display_title(),
                "description": entry.description,
                "category": entry.connect().category,
                "status": entry.connect().status.as_str(),
                "modes": modes_json(entry),
                "connected": record.is_some(),
                "mode": record.map(|record| record.mode.as_str()),
            })
        })
        .collect();
    json!({
        "catalog_version": loaded.catalog.version,
        "count": services.len(),
        "services": services,
    })
}
