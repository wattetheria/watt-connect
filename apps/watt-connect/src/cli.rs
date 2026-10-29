use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "watt-connect",
    version,
    about = "One MCP for your agent to discover and call official merchant MCP servers.",
    long_about = "Run without a subcommand to serve MCP over stdio (what agents launch via `npx watt-connect`).\n\
                  Merchant tokens are kept on this machine (encrypted file on macOS, system keychain elsewhere) and never leave it."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Serve MCP over stdio (default).
    Serve,
    /// Connect a service in proxy mode: enter, verify and store its token.
    Connect(ConnectArgs),
    /// Forget a service's token and connection (and remove direct-mode entries).
    Disconnect(DisconnectArgs),
    /// Show connected services, credential status and catalog version.
    Status,
    /// Direct mode: write a service into an agent's own MCP configuration.
    Install(InstallArgs),
    /// Remove a direct-mode service from an agent's configuration.
    Uninstall(UninstallArgs),
    /// Search the catalog.
    Search(SearchArgs),
}

#[derive(Debug, Args)]
pub struct TokenSource {
    /// Read the token from stdin instead of prompting.
    #[arg(long)]
    pub token_stdin: bool,
    /// Store the token without verifying it with the merchant first.
    #[arg(long)]
    pub no_verify: bool,
}

#[derive(Debug, Args)]
pub struct ConnectArgs {
    /// Service id, e.g. cn.lkcoffee/order.
    pub service: String,
    /// Use the merchant's sandbox endpoint (where available, e.g. Didi).
    #[arg(long)]
    pub sandbox: bool,
    /// Enter the token in a one-time local web page instead of the terminal.
    #[arg(long, conflicts_with = "token_stdin")]
    pub web: bool,
    #[command(flatten)]
    pub token: TokenSource,
}

#[derive(Debug, Args)]
pub struct DisconnectArgs {
    pub service: String,
    /// Keep direct-mode entries in agent configs.
    #[arg(long)]
    pub keep_agent_config: bool,
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    pub service: String,
    /// claude-code, cursor or codex.
    #[arg(long)]
    pub agent: String,
    #[arg(long)]
    pub sandbox: bool,
    #[command(flatten)]
    pub token: TokenSource,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    pub service: String,
    #[arg(long)]
    pub agent: String,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    #[arg(default_value = "")]
    pub query: String,
    #[arg(long)]
    pub category: Option<String>,
}
