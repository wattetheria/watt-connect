//! End to end: the real `watt-connect` binary, driven over stdio by an MCP
//! client, talking to a mock merchant. Uses an isolated WATT_CONNECT_HOME, the
//! encrypted-file vault and an unsigned development catalog.

use connect_catalog::bundled::bundled_catalog;
use connect_testkit::{MockMerchant, profiles};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_watt-connect");
const TOKEN: &str = "lk-e2e-secret-token";

struct Env {
    home: tempfile::TempDir,
    catalog: PathBuf,
}

impl Env {
    fn new(mock: &MockMerchant) -> Self {
        let home = tempfile::tempdir().unwrap();
        let mut catalog = bundled_catalog();
        for entry in &mut catalog.services {
            if entry.name.as_str() == "cn.lkcoffee/order" {
                entry.remotes[0].url = mock.url();
            }
        }
        let path = home.path().join("dev-catalog.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&catalog).unwrap()).unwrap();
        Self {
            home,
            catalog: path,
        }
    }

    fn apply(&self, command: &mut Command) {
        command
            .env("WATT_CONNECT_HOME", self.home.path())
            .env("WATT_CONNECT_VAULT", "file")
            .env("WATT_CONNECT_DEV_CATALOG", &self.catalog)
            .env("WATT_CONNECT_NO_WEB", "1")
            .env("HOME", self.home.path())
            .env_remove("WATT_CONNECT_LOG");
    }

    fn cli(&self, args: &[&str], stdin: &str) -> std::process::Output {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        self.apply(&mut command);
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

fn all_files_text(dir: &Path) -> String {
    let mut out = String::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.push_str(&all_files_text(&path));
        } else {
            out.push_str(&String::from_utf8_lossy(&std::fs::read(&path).unwrap()));
        }
    }
    out
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: Value,
) -> CallToolResult {
    let mut params = CallToolRequestParams::new(name.to_owned());
    params.arguments = args.as_object().cloned();
    client.peer().call_tool(params).await.unwrap()
}

fn error_code(result: &CallToolResult) -> Option<String> {
    (result.is_error == Some(true))
        .then(|| {
            result.structured_content.as_ref()?["error"]["code"]
                .as_str()
                .map(str::to_owned)
        })
        .flatten()
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_then_order_through_stdio_server() {
    let mock = MockMerchant::start(profiles::luckin::config(TOKEN)).await;
    let env = Env::new(&mock);

    // A wrong token is rejected and not stored.
    let rejected = env.cli(
        &["connect", "cn.lkcoffee/order", "--token-stdin"],
        "wrong-token\n",
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("CREDENTIAL_INVALID"));

    let connected = env.cli(
        &["connect", "cn.lkcoffee/order", "--token-stdin"],
        &format!("{TOKEN}\n"),
    );
    assert!(
        connected.status.success(),
        "{}",
        String::from_utf8_lossy(&connected.stderr)
    );
    assert!(String::from_utf8_lossy(&connected.stdout).contains("5 tools available"));

    let status = env.cli(&["status"], "");
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(status_text.contains("cn.lkcoffee/order"), "{status_text}");
    assert!(status_text.contains("encrypted-file"), "{status_text}");

    let client = ()
        .serve(
            TokioChildProcess::new(tokio::process::Command::new(BIN).configure(|command| {
                command
                    .env("WATT_CONNECT_HOME", env.home.path())
                    .env("WATT_CONNECT_VAULT", "file")
                    .env("WATT_CONNECT_DEV_CATALOG", &env.catalog)
                    .env("WATT_CONNECT_NO_WEB", "1")
                    .env("HOME", env.home.path())
                    .stderr(Stdio::null());
            }))
            .unwrap(),
        )
        .await
        .unwrap();

    let tools = client.peer().list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 6);

    let described = call(
        &client,
        "describe_service",
        json!({"service": "cn.lkcoffee/order"}),
    )
    .await;
    assert_eq!(
        described.structured_content.unwrap()["tools_source"],
        "live"
    );

    let shops = call(
        &client,
        "call_service",
        json!({"service": "cn.lkcoffee/order", "tool": "queryShopList"}),
    )
    .await;
    assert_eq!(shops.is_error, Some(false));
    let blocked = call(
        &client,
        "call_service",
        json!({"service": "cn.lkcoffee/order", "tool": "createOrder"}),
    )
    .await;
    assert_eq!(error_code(&blocked).as_deref(), Some("PREVIEW_REQUIRED"));
    call(
        &client,
        "call_service",
        json!({"service": "cn.lkcoffee/order", "tool": "previewOrder"}),
    )
    .await;
    let order = call(
        &client,
        "call_service",
        json!({"service": "cn.lkcoffee/order", "tool": "createOrder"}),
    )
    .await;
    let order_text = serde_json::to_string(&order.content).unwrap();
    assert!(
        order_text.contains("https://pay.lkcoffee.example/lk-1"),
        "{order_text}"
    );
    assert!(order_text.contains("用户本人完成支付"), "{order_text}");
    client.cancel().await.unwrap();

    // The token appears nowhere on disk in clear text (config, vault, logs).
    let disk = all_files_text(env.home.path());
    assert!(!disk.contains(TOKEN), "token leaked to disk");
    assert!(
        disk.contains("call_service"),
        "log file should record calls"
    );

    let disconnected = env.cli(&["disconnect", "cn.lkcoffee/order"], "");
    assert!(disconnected.status.success());
    let status = env.cli(&["status"], "");
    assert!(String::from_utf8_lossy(&status.stdout).contains("No services connected"));
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_via_local_web_page() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mock = MockMerchant::start(profiles::luckin::config(TOKEN)).await;
    let env = Env::new(&mock);
    let mut command = tokio::process::Command::new(BIN);
    command
        .args(["connect", "cn.lkcoffee/order", "--web"])
        .env("WATT_CONNECT_HOME", env.home.path())
        .env("WATT_CONNECT_VAULT", "file")
        .env("WATT_CONNECT_DEV_CATALOG", &env.catalog)
        .env("HOME", env.home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let url = loop {
        let line = lines
            .next_line()
            .await
            .unwrap()
            .expect("page URL on stderr");
        if let Some(url) = line.trim().strip_prefix("http://") {
            break format!("http://{url}");
        }
    };
    let http = reqwest::Client::new();
    let page = http.get(&url).send().await.unwrap();
    assert_eq!(page.status(), 200);
    assert_eq!(page.headers()["cache-control"], "no-store");
    assert!(page.text().await.unwrap().contains("瑞幸咖啡"));

    let wrong_nonce = url.rsplit_once('/').unwrap().0.to_owned() + "/nope";
    assert_eq!(http.get(&wrong_nonce).send().await.unwrap().status(), 404);
    let rebinding = http
        .get(&url)
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(rebinding.status(), 403);

    let bad = http
        .post(&url)
        .form(&[("token", "wrong")])
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(bad.contains("CREDENTIAL_INVALID"), "{bad}");
    let good = http
        .post(&url)
        .form(&[("token", TOKEN)])
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(good.contains("Connected"), "{good}");

    let status = child.wait().await.unwrap();
    assert!(status.success());
    let config = std::fs::read_to_string(env.home.path().join("config/config.toml")).unwrap();
    assert!(config.contains("cn.lkcoffee/order"));
    assert!(!config.contains(TOKEN));
}
