//! Meta tools end to end against mock merchants (no real network).

use connect_catalog::bundled::bundled_catalog;
use connect_catalog::{CatalogConfig, CatalogStore};
use connect_installer::InstallerEnv;
use connect_server::{
    ConnectContext, ConnectServer, ConnectionRecord, ConnectionStore, MemoryConnectionStore,
};
use connect_testkit::{MockMerchant, profiles};
use connect_types::{ConnectionMode, CredentialStatus, ServiceId};
use connect_upstream::{BackoffPolicy, UpstreamPool};
use connect_vault::{MemoryVault, Secret, Vault};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

struct Harness {
    server: ConnectServer,
    vault: Arc<MemoryVault>,
    connections: Arc<MemoryConnectionStore>,
    home: tempfile::TempDir,
    _dir: tempfile::TempDir,
}

/// Catalog whose Luckin / McDonald's endpoints point at the mocks.
fn harness(endpoints: &[(&str, String)]) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let mut catalog = bundled_catalog();
    for entry in &mut catalog.services {
        if let Some((_, url)) = endpoints.iter().find(|(id, _)| entry.name.as_str() == *id) {
            entry.remotes[0].url.clone_from(url);
        }
    }
    let path = dir.path().join("catalog.json");
    std::fs::write(&path, serde_json::to_vec(&catalog).unwrap()).unwrap();
    let store = CatalogStore::load(CatalogConfig {
        url: url::Url::parse("https://example.invalid/catalog.json").unwrap(),
        trusted_keys: Vec::new(),
        cache_dir: dir.path().join("cache"),
        dev_catalog: Some(path),
    })
    .unwrap();
    let vault = Arc::new(MemoryVault::default());
    let connections = Arc::new(MemoryConnectionStore::default());
    let home = tempfile::tempdir().unwrap();
    let context = ConnectContext {
        catalog: Arc::new(store),
        vault: vault.clone(),
        connections: connections.clone(),
        upstream: Arc::new(UpstreamPool::new(BackoffPolicy {
            max_retries: 2,
            base: Duration::from_millis(5),
            max_delay: Duration::from_millis(10),
        })),
        installer_env: InstallerEnv {
            home: home.path().to_owned(),
            use_cli: false,
            codex_home: None,
        },
        link_provider: None,
        command: "npx watt-connect".into(),
    };
    Harness {
        server: ConnectServer::new(Arc::new(context)),
        vault,
        connections,
        home,
        _dir: dir,
    }
}

impl Harness {
    fn connect_proxy(&self, id: &str, token: &str) {
        let service = ServiceId::parse(id).unwrap();
        self.vault.set(&service, &Secret::new(token)).unwrap();
        self.connections
            .upsert(ConnectionRecord {
                service,
                mode: ConnectionMode::Proxy,
                connected_at: "2026-10-29T00:00:00Z".into(),
                credential_status: CredentialStatus::Ok,
                sandbox: false,
                agents: Vec::new(),
            })
            .unwrap();
    }

    async fn call(&self, tool: &str, args: Value) -> CallToolResult {
        self.server.run_tool(tool, args.as_object().cloned()).await
    }
}

fn error_code(result: &CallToolResult) -> Option<String> {
    if result.is_error != Some(true) {
        return None;
    }
    result
        .structured_content
        .as_ref()
        .and_then(|value| value["error"]["code"].as_str())
        .map(str::to_owned)
}

fn text(result: &CallToolResult) -> String {
    serde_json::to_value(&result.content).unwrap().to_string()
}

#[tokio::test]
async fn search_and_describe_unconnected() {
    let h = harness(&[]);
    let found = h.call("search_services", json!({"query": "咖啡"})).await;
    let services = &found.structured_content.as_ref().unwrap()["services"];
    assert_eq!(services[0]["service"], "cn.lkcoffee/order");
    assert_eq!(services[0]["connected"], false);

    let described = h
        .call("describe_service", json!({"service": "cn.mcd/order"}))
        .await;
    let value = described.structured_content.unwrap();
    assert_eq!(value["tools_source"], "catalog_summary");
    assert_eq!(value["modes"]["preferred"], "direct");
    assert!(value["how_to_connect"].is_string());

    let missing = h
        .call("describe_service", json!({"service": "cn.nobody/none"}))
        .await;
    assert_eq!(error_code(&missing).as_deref(), Some("SERVICE_NOT_FOUND"));
}

#[tokio::test]
async fn connect_service_returns_instructions_and_refuses_tokens() {
    let h = harness(&[]);
    let result = h
        .call("connect_service", json!({"service": "cn.lkcoffee/order"}))
        .await;
    let value = result.structured_content.unwrap();
    assert_eq!(value["mode"], "proxy");
    assert_eq!(
        value["terminal_command"],
        "npx watt-connect connect cn.lkcoffee/order"
    );
    assert!(value["warning"].as_str().unwrap().contains("Do not ask"));

    let direct = h
        .call("connect_service", json!({"service": "cn.mcd/order"}))
        .await;
    assert_eq!(direct.structured_content.unwrap()["mode"], "direct");

    let with_token = h
        .call(
            "connect_service",
            json!({"service": "cn.lkcoffee/order", "token": "abc"}),
        )
        .await;
    assert_eq!(
        error_code(&with_token).as_deref(),
        Some("INVALID_ARGUMENTS")
    );

    let proxy_on_direct_only = h
        .call(
            "connect_service",
            json!({"service": "io.wattetheria/node", "mode": "proxy"}),
        )
        .await;
    assert_eq!(
        error_code(&proxy_on_direct_only).as_deref(),
        Some("MODE_NOT_ALLOWED")
    );
}

#[tokio::test]
async fn luckin_preview_then_order_with_payment_link() {
    let mock = MockMerchant::start(profiles::luckin::config("lk-token")).await;
    let h = harness(&[("cn.lkcoffee/order", mock.url())]);
    h.connect_proxy("cn.lkcoffee/order", "lk-token");

    let described = h
        .call("describe_service", json!({"service": "cn.lkcoffee/order"}))
        .await;
    let value = described.structured_content.unwrap();
    assert_eq!(value["tools_source"], "live", "{value}");
    let create = value["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "createOrder")
        .unwrap();
    assert_eq!(create["effect"], "order");
    assert_eq!(create["requires_preview"], "previewOrder");

    let shops = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "queryShopList", "arguments": {}}),
        )
        .await;
    assert_eq!(shops.is_error, Some(false));
    assert!(text(&shops).contains("望京店"));

    let blocked = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "createOrder"}),
        )
        .await;
    assert_eq!(error_code(&blocked).as_deref(), Some("PREVIEW_REQUIRED"));
    assert!(!mock.calls().contains(&"createOrder".to_owned()));

    let preview = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "previewOrder"}),
        )
        .await;
    assert_eq!(preview.is_error, Some(false));

    let order = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "createOrder"}),
        )
        .await;
    assert_eq!(order.is_error, Some(false), "{}", text(&order));
    let body = text(&order);
    assert!(body.contains("用户本人完成支付"), "{body}");
    assert!(
        body.contains("payUrl: https://pay.lkcoffee.example/lk-1"),
        "{body}"
    );

    // The preview was consumed by the order.
    let again = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "createOrder"}),
        )
        .await;
    assert_eq!(error_code(&again).as_deref(), Some("PREVIEW_REQUIRED"));
    assert_eq!(
        mock.calls(),
        vec!["queryShopList", "previewOrder", "createOrder"]
    );
}

#[tokio::test]
async fn expired_token_is_reported_once_and_short_circuits() {
    let mock = MockMerchant::start(profiles::luckin::config("lk-token")).await;
    let h = harness(&[("cn.lkcoffee/order", mock.url())]);
    h.connect_proxy("cn.lkcoffee/order", "lk-token");
    mock.set_valid_token(Some("rotated"));

    let first = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "queryShopList"}),
        )
        .await;
    assert_eq!(error_code(&first).as_deref(), Some("CREDENTIAL_INVALID"));
    let error = &first.structured_content.as_ref().unwrap()["error"];
    assert_eq!(error["portal_url"], "https://open.lkcoffee.com/");
    assert!(!text(&first).contains("lk-token"));
    let id = ServiceId::parse("cn.lkcoffee/order").unwrap();
    assert_eq!(
        h.connections.get(&id).unwrap().credential_status,
        CredentialStatus::CredentialInvalid
    );

    let requests_before = mock.requests().len();
    let second = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "queryShopList"}),
        )
        .await;
    assert_eq!(error_code(&second).as_deref(), Some("CREDENTIAL_INVALID"));
    assert_eq!(
        mock.requests().len(),
        requests_before,
        "no request after the token is known bad"
    );

    let listed = h.call("list_connections", json!({})).await;
    assert_eq!(
        listed.structured_content.unwrap()["connections"][0]["credential_status"],
        "credential_invalid"
    );
}

#[tokio::test]
async fn call_service_guards() {
    let h = harness(&[]);
    let not_connected = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "queryShopList"}),
        )
        .await;
    assert_eq!(
        error_code(&not_connected).as_deref(),
        Some("SERVICE_NOT_CONNECTED")
    );

    let direct_only = h
        .call(
            "call_service",
            json!({"service": "io.wattetheria/node", "tool": "x"}),
        )
        .await;
    assert_eq!(
        error_code(&direct_only).as_deref(),
        Some("MODE_NOT_ALLOWED")
    );

    let id = ServiceId::parse("cn.lkcoffee/order").unwrap();
    h.connections
        .upsert(ConnectionRecord {
            service: id.clone(),
            mode: ConnectionMode::Proxy,
            connected_at: String::new(),
            credential_status: CredentialStatus::Ok,
            sandbox: false,
            agents: Vec::new(),
        })
        .unwrap();
    let no_token = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "queryShopList"}),
        )
        .await;
    assert_eq!(error_code(&no_token).as_deref(), Some("CREDENTIAL_MISSING"));

    let bad_args = h
        .call(
            "call_service",
            json!({"service": "cn.lkcoffee/order", "tool": "x", "arguments": [1]}),
        )
        .await;
    assert_eq!(error_code(&bad_args).as_deref(), Some("INVALID_ARGUMENTS"));
}

#[tokio::test]
async fn install_service_moves_token_into_cursor_config() {
    let h = harness(&[]);
    std::fs::create_dir_all(h.home.path().join(".cursor")).unwrap();
    h.connect_proxy("cn.mcd/order", "mcd-secret");

    let result = h
        .call(
            "install_service",
            json!({"service": "cn.mcd/order", "agent": "cursor"}),
        )
        .await;
    let value = result.structured_content.unwrap();
    assert_eq!(value["installed"], true, "{value}");
    assert_eq!(value["server_name"], "mcd-order");

    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(h.home.path().join(".cursor/mcp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        config["mcpServers"]["mcd-order"]["headers"]["Authorization"],
        "Bearer mcd-secret"
    );

    let id = ServiceId::parse("cn.mcd/order").unwrap();
    assert!(
        h.vault.get(&id).unwrap().is_none(),
        "watt-connect keeps no copy in direct mode"
    );
    let record = h.connections.get(&id).unwrap();
    assert_eq!(record.mode, ConnectionMode::Direct);
    assert_eq!(record.agents, vec!["cursor"]);

    let call = h
        .call(
            "call_service",
            json!({"service": "cn.mcd/order", "tool": "calculate-price"}),
        )
        .await;
    assert_eq!(error_code(&call).as_deref(), Some("MODE_NOT_ALLOWED"));

    // Without a stored token, the agent is told to hand off to the terminal.
    let luckin = h
        .call(
            "install_service",
            json!({"service": "cn.lkcoffee/order", "agent": "cursor"}),
        )
        .await;
    let value = luckin.structured_content.unwrap();
    assert_eq!(value["installed"], false);
    assert_eq!(
        value["terminal_command"],
        "npx watt-connect install cn.lkcoffee/order --agent cursor"
    );

    // No-credential stdio service installs directly.
    let node = h
        .call(
            "install_service",
            json!({"service": "io.wattetheria/node", "agent": "cursor"}),
        )
        .await;
    assert_eq!(node.structured_content.unwrap()["installed"], true);
}

#[tokio::test]
async fn serves_six_tools_over_mcp() {
    use rmcp::ServiceExt;
    let h = harness(&[]);
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server = h.server.clone();
    tokio::spawn(async move {
        let running = Box::pin(server.serve(server_io)).await.unwrap();
        let _ = running.waiting().await;
    });
    let client = ().serve(client_io).await.unwrap();
    let tools = client.peer().list_all_tools().await.unwrap();
    let mut names: Vec<String> = tools.iter().map(|tool| tool.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "call_service",
            "connect_service",
            "describe_service",
            "install_service",
            "list_connections",
            "search_services"
        ]
    );
    let mut params = rmcp::model::CallToolRequestParams::new("search_services");
    params.arguments = json!({"query": "taxi"}).as_object().cloned();
    let result = client.peer().call_tool(params).await.unwrap();
    assert_eq!(
        result.structured_content.unwrap()["services"][0]["service"],
        "cn.didichuxing/ride"
    );
    client.cancel().await.unwrap();
}
