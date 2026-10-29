//! One-time local web page for entering a token.
//!
//! Bound to `127.0.0.1` on a random port, reachable only at
//! `/connect/<random nonce>`, rejects foreign `Host` headers (DNS rebinding),
//! closes after a successful save or after ten minutes. The token goes from
//! the browser straight to this process; it never passes through the agent.

use crate::commands::connect::{render_error, verify_and_store};
use anyhow::{Context, Result};
use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use connect_catalog::ServerEntry;
use connect_server::{ConnectContext, ConnectLinkProvider};
use connect_types::ServiceId;
use connect_vault::Secret;
use rand::RngCore;
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use tokio::sync::{Notify, oneshot};

const PAGE_LIFETIME: Duration = Duration::from_mins(10);

/// Link provider used by `serve`: `connect_service` can hand the agent a
/// local URL for the user to open.
pub struct WebConnect {
    context: OnceLock<Weak<ConnectContext>>,
}

impl WebConnect {
    pub fn new() -> Self {
        Self {
            context: OnceLock::new(),
        }
    }

    pub fn attach(&self, context: Weak<ConnectContext>) {
        let _ = self.context.set(context);
    }
}

impl ConnectLinkProvider for WebConnect {
    fn start(&self, service: &ServiceId) -> Option<String> {
        let context = self.context.get()?.upgrade()?;
        let entry = context.catalog.current().catalog.get(service)?.clone();
        let sandbox = context
            .connections
            .get(service)
            .is_some_and(|record| record.sandbox);
        match open_page(context, entry, sandbox, true) {
            Ok(page) => Some(page.url),
            Err(error) => {
                tracing::warn!("cannot start local connect page: {error:#}");
                None
            }
        }
    }
}

pub struct Page {
    pub url: String,
    /// Resolves when the token was saved (or the page expired).
    pub done: oneshot::Receiver<bool>,
}

struct PageState {
    context: Arc<ConnectContext>,
    entry: ServerEntry,
    sandbox: bool,
    verify: bool,
    nonce: String,
    host: String,
    saved: AtomicBool,
    done: Mutex<Option<oneshot::Sender<bool>>>,
    shutdown: Notify,
}

/// Start a page. Must be called inside a tokio runtime.
pub fn open_page(
    context: Arc<ConnectContext>,
    entry: ServerEntry,
    sandbox: bool,
    verify: bool,
) -> Result<Page> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("bind 127.0.0.1")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let mut nonce = [0_u8; 24];
    rand::rng().fill_bytes(&mut nonce);
    let nonce = hex::encode(nonce);
    let host = format!("127.0.0.1:{port}");
    let url = format!("http://{host}/connect/{nonce}");
    let (done_tx, done_rx) = oneshot::channel();
    let state = Arc::new(PageState {
        context,
        entry,
        sandbox,
        verify,
        nonce,
        host,
        saved: AtomicBool::new(false),
        done: Mutex::new(Some(done_tx)),
        shutdown: Notify::new(),
    });
    let app = Router::new()
        .route("/connect/{nonce}", get(show).post(submit))
        .with_state(state.clone());
    tokio::spawn(async move {
        let stop = state.clone();
        let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
            tokio::select! {
                () = stop.shutdown.notified() => {}
                () = tokio::time::sleep(PAGE_LIFETIME) => {}
            }
        });
        if let Err(error) = serve.await {
            tracing::warn!(%error, "local connect page stopped");
        }
        finish(&state, false);
    });
    Ok(Page { url, done: done_rx })
}

fn finish(state: &PageState, saved: bool) {
    if let Some(sender) = state
        .done
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        let _ = sender.send(saved);
    }
}

fn guard(state: &PageState, nonce: &str, headers: &HeaderMap) -> Option<Response> {
    let host_ok = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| host == state.host);
    if !host_ok {
        return Some(StatusCode::FORBIDDEN.into_response());
    }
    if nonce != state.nonce || state.saved.load(Ordering::SeqCst) {
        return Some(StatusCode::NOT_FOUND.into_response());
    }
    None
}

async fn show(
    State(state): State<Arc<PageState>>,
    Path(nonce): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(rejection) = guard(&state, &nonce, &headers) {
        return rejection;
    }
    page(&state, None)
}

#[derive(Deserialize)]
struct TokenForm {
    token: String,
}

async fn submit(
    State(state): State<Arc<PageState>>,
    Path(nonce): Path<String>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    if let Some(rejection) = guard(&state, &nonce, &headers) {
        return rejection;
    }
    let token = form.token.trim();
    if token.is_empty() {
        return page(&state, Some("请输入 token / Please paste the token."));
    }
    connect_vault::redact::register_secret(token);
    let secret = Secret::new(token);
    match verify_and_store(
        &state.context,
        &state.entry,
        Some(&secret),
        state.sandbox,
        state.verify,
    )
    .await
    {
        Ok(_) => {
            state.saved.store(true, Ordering::SeqCst);
            finish(&state, true);
            state.shutdown.notify_one();
            secure(Html(format!(
                "{HEAD}<h1>已连接 / Connected</h1><p>{} 的 token 已验证并保存在本机。现在可以关闭此页面，回到 agent 继续。</p>\
                 <p>The token for {} was verified and saved on this computer. You can close this page.</p></main>",
                escape(state.entry.display_title()),
                escape(state.entry.display_title())
            )))
        }
        Err(error) => page(
            &state,
            Some(&connect_vault::redact::redact(&render_error(&error))),
        ),
    }
}

const HEAD: &str = "<!doctype html><html lang=\"zh\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>watt-connect</title>\
<style>body{font-family:system-ui,sans-serif;background:#f6f6f4;color:#1d1d1b;margin:0}\
main{max-width:520px;margin:48px auto;padding:0 16px}input{width:100%;box-sizing:border-box;padding:10px;font-size:16px}\
button{margin-top:12px;padding:10px 18px;font-size:16px}.err{color:#a4161a;white-space:pre-wrap}\
@media (prefers-color-scheme:dark){body{background:#161615;color:#ecece8}}</style></head><body><main>";

fn page(state: &PageState, error: Option<&str>) -> Response {
    let title = escape(state.entry.display_title());
    let portal = state
        .entry
        .portal_url()
        .map(|url| format!("<p>获取 token / Get a token: <a href=\"{0}\" rel=\"noreferrer\" target=\"_blank\">{0}</a></p>", escape(url)))
        .unwrap_or_default();
    let error = error
        .map(|message| format!("<p class=\"err\">{}</p>", escape(message)))
        .unwrap_or_default();
    secure(Html(format!(
        "{HEAD}<h1>连接 {title}</h1><p>Connect <b>{title}</b> ({id}) to watt-connect.</p>{portal}{error}\
         <form method=\"post\"><label for=\"t\">Token</label>\
         <input id=\"t\" name=\"token\" type=\"password\" autocomplete=\"off\" autofocus required>\
         <button type=\"submit\">验证并保存 / Verify &amp; save</button></form>\
         <p><small>token 只保存在本机系统钥匙串中，不会发送给 agent 或我们的服务器。<br>\
         The token is stored only in this computer's keychain.</small></p></main>",
        id = escape(state.entry.name.as_str()),
    )))
}

fn secure(html: Html<String>) -> Response {
    let mut response = html.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        "no-referrer".parse().expect("static header"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'"
            .parse()
            .expect("static header"),
    );
    response
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// `watt-connect connect --web`: open a page and wait for it.
pub async fn run_page(
    context: Arc<ConnectContext>,
    entry: ServerEntry,
    sandbox: bool,
    verify: bool,
) -> Result<String> {
    let page = open_page(context, entry, sandbox, verify)?;
    eprintln!(
        "Open this page on this computer to enter the token:\n  {}",
        page.url
    );
    if page.done.await.unwrap_or(false) {
        Ok(page.url)
    } else {
        anyhow::bail!("the page expired before a token was saved")
    }
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn escapes_html() {
        assert_eq!(
            escape("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
    }
}
