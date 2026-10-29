//! rmcp `ServerHandler` exposing the meta tools.

use crate::ConnectContext;
use crate::tools::{self, Output};
use connect_policy::PolicySession;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, InitializeResult,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::json;
use std::sync::{Arc, Mutex, MutexGuard};

pub const INSTRUCTIONS: &str = "watt-connect gives you access to merchant MCP services (coffee, fast food, rides, ...) \
through six meta tools. Typical flow: search_services -> describe_service -> (connect_service if not connected) -> \
call_service. Tokens are entered by the user in a terminal or local page, never in chat. Before any order: call the \
preview tool, show the user the price, and get explicit confirmation. Payment links must be handed to the user; you \
never pay.";

/// One instance per agent session (stdio = one process = one session).
#[derive(Clone)]
pub struct ConnectServer {
    context: Arc<ConnectContext>,
    policy: Arc<Mutex<PolicySession>>,
}

impl ConnectServer {
    #[must_use]
    pub fn new(context: Arc<ConnectContext>) -> Self {
        Self {
            context,
            policy: Arc::new(Mutex::new(PolicySession::new())),
        }
    }

    #[must_use]
    pub fn context(&self) -> &ConnectContext {
        &self.context
    }

    pub(crate) fn policy(&self) -> MutexGuard<'_, PolicySession> {
        self.policy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Run a meta tool by name. Errors become `isError` results carrying a
    /// structured `{ "error": { code, message, portal_url?, hint? } }`.
    pub async fn run_tool(
        &self,
        name: &str,
        arguments: Option<rmcp::model::JsonObject>,
    ) -> CallToolResult {
        let arguments = arguments.unwrap_or_default();
        match tools::dispatch(self, name, &arguments).await {
            Ok(Output::Json(value)) => CallToolResult::structured(value),
            Ok(Output::Passthrough(result)) => result,
            Err(mut error) => {
                error.message = connect_vault::redact::redact(&error.message);
                tracing::debug!(tool = name, code = error.code.as_str(), "meta tool error");
                CallToolResult::structured_error(json!({ "error": error }))
            }
        }
    }

    /// Shared with the CLI.
    pub async fn install(
        &self,
        entry: &connect_catalog::ServerEntry,
        agent: connect_installer::AgentKind,
        sandbox: bool,
        token: Option<&connect_vault::Secret>,
    ) -> Result<connect_installer::InstallReport, connect_types::ConnectError> {
        tools::install_direct(self, entry, agent, sandbox, token).await
    }
}

impl ServerHandler for ConnectServer {
    fn get_info(&self) -> ServerConfig {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "watt-connect",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tools::definitions()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        Ok(self.run_tool(&request.name, request.arguments).await.into())
    }
}
