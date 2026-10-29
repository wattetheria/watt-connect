//! Shared editing of JSON config files with a top-level `mcpServers` object
//! (Cursor's `mcp.json`, Claude Code's `~/.claude.json`). Only our entry is
//! touched; key order and every other field are preserved.

use crate::backup::{backup, write_atomic};
use crate::{DirectTransport, InstallError, io_error};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub(crate) const SERVERS_KEY: &str = "mcpServers";

pub(crate) fn read(path: &Path) -> Result<Map<String, Value>, InstallError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(io_error(path)(error)),
    };
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(parse_error(path, "top level must be an object".into())),
        Err(error) => Err(parse_error(path, error.to_string())),
    }
}

fn parse_error(path: &Path, message: String) -> InstallError {
    InstallError::Parse {
        path: path.to_owned(),
        format: "JSON",
        message,
    }
}

fn write(path: &Path, root: &Map<String, Value>) -> Result<Option<PathBuf>, InstallError> {
    let backup = backup(path)?;
    let mut text =
        serde_json::to_string_pretty(root).map_err(|error| parse_error(path, error.to_string()))?;
    text.push('\n');
    write_atomic(path, text.as_bytes())?;
    Ok(backup)
}

pub(crate) fn upsert(
    path: &Path,
    name: &str,
    server: Value,
) -> Result<Option<PathBuf>, InstallError> {
    let mut root = read(path)?;
    let servers = root
        .entry(SERVERS_KEY)
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(servers) = servers else {
        return Err(parse_error(
            path,
            format!("`{SERVERS_KEY}` must be an object"),
        ));
    };
    servers.insert(name.to_owned(), server);
    write(path, &root)
}

pub(crate) fn remove(path: &Path, name: &str) -> Result<bool, InstallError> {
    let mut root = read(path)?;
    let removed = root
        .get_mut(SERVERS_KEY)
        .and_then(Value::as_object_mut)
        .is_some_and(|servers| servers.remove(name).is_some());
    if removed {
        write(path, &root)?;
    }
    Ok(removed)
}

pub(crate) fn names(path: &Path) -> Result<Vec<String>, InstallError> {
    Ok(read(path)?
        .get(SERVERS_KEY)
        .and_then(Value::as_object)
        .map(|servers| servers.keys().cloned().collect())
        .unwrap_or_default())
}

/// `{"url", "headers"}` / `{"command", "args", "env"}`, plus `"type"` when
/// `with_type` (Claude Code requires it, Cursor infers it).
pub(crate) fn server_value(transport: &DirectTransport, with_type: bool) -> Value {
    let mut server = Map::new();
    match transport {
        DirectTransport::Http { url, headers } => {
            if with_type {
                server.insert("type".into(), "http".into());
            }
            server.insert("url".into(), url.clone().into());
            if !headers.is_empty() {
                server.insert(
                    "headers".into(),
                    Value::Object(
                        headers
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone().into()))
                            .collect(),
                    ),
                );
            }
        }
        DirectTransport::Stdio { command, args, env } => {
            if with_type {
                server.insert("type".into(), "stdio".into());
            }
            server.insert("command".into(), command.clone().into());
            server.insert(
                "args".into(),
                args.iter().cloned().map(Value::from).collect(),
            );
            if !env.is_empty() {
                server.insert(
                    "env".into(),
                    Value::Object(
                        env.iter()
                            .map(|(k, v)| (k.clone(), v.clone().into()))
                            .collect(),
                    ),
                );
            }
        }
    }
    Value::Object(server)
}
