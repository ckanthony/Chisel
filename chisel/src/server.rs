use std::{net::SocketAddr, num::NonZeroU32, sync::Arc};

use axum::extract::Request;
use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use http_body_util::BodyExt;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::{
        self,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::{
    auth::auth_layer,
    error::AppError,
    state::SharedState,
    tools::{filesystem, shell},
};

// ──────────────────────────────────────────────
// Tool parameter structs
// ──────────────────────────────────────────────

#[derive(Deserialize, JsonSchema)]
pub struct PatchApplyParams {
    /// Absolute path to the file to patch
    pub path: String,
    /// Unified diff to apply (may be wrapped in a markdown code fence)
    pub patch: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct PathContentParams {
    pub path: String,
    pub content: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct PathParam {
    pub path: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct MoveParams {
    pub source: String,
    pub destination: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ShellExecParams {
    pub command: String,
    pub args: Vec<String>,
}

// ──────────────────────────────────────────────
// MCP server handler
// ──────────────────────────────────────────────

#[derive(Clone)]
pub struct McpServer {
    state: SharedState,
    tool_router: ToolRouter<McpServer>,
}

#[tool_router]
impl McpServer {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    /// Apply a unified diff patch to a file.
    /// Accepts patches wrapped in markdown code fences.
    /// Use `--- /dev/null` as source to create new files.
    /// Returns error in read-only mode.
    #[tool(
        description = "Apply a unified diff patch to a file. Accepts markdown-fenced diffs. Use --- /dev/null to create new files. Returns error in read-only mode."
    )]
    async fn patch_apply(
        &self,
        Parameters(p): Parameters<PatchApplyParams>,
    ) -> Result<String, AppError> {
        filesystem::patch_apply(&self.state, p.path, p.patch).await
    }

    /// Append content to an existing file.
    /// Fails if the file does not exist.
    /// Returns error in read-only mode.
    #[tool(
        description = "Append content to an existing file. Fails if file does not exist. Returns error in read-only mode."
    )]
    async fn append(
        &self,
        Parameters(p): Parameters<PathContentParams>,
    ) -> Result<String, AppError> {
        filesystem::append(&self.state, p.path, p.content).await
    }

    /// Write content to a file, creating or overwriting it.
    /// Creates parent directories automatically.
    /// Returns error in read-only mode.
    #[tool(
        description = "Write content to a file (create or overwrite). Creates parent dirs. Returns error in read-only mode."
    )]
    async fn write_file(
        &self,
        Parameters(p): Parameters<PathContentParams>,
    ) -> Result<String, AppError> {
        filesystem::write_file(&self.state, p.path, p.content).await
    }

    /// Create a directory (and all missing parents).
    /// Returns error in read-only mode.
    #[tool(
        description = "Create a directory (including parents). Returns error in read-only mode."
    )]
    async fn create_directory(
        &self,
        Parameters(p): Parameters<PathParam>,
    ) -> Result<String, AppError> {
        filesystem::create_directory(&self.state, p.path).await
    }

    /// Move or rename a file within the root.
    /// Returns error in read-only mode.
    #[tool(
        description = "Move or rename a file within the root directory. Returns error in read-only mode."
    )]
    async fn move_file(&self, Parameters(p): Parameters<MoveParams>) -> Result<String, AppError> {
        filesystem::move_file(&self.state, p.source, p.destination).await
    }

    /// Execute a whitelisted shell command directly (no shell interpreter).
    /// Allowed: grep sed awk find cat head tail wc sort uniq cut tr diff file stat ls du.
    /// Path-like arguments are validated against the root directory.
    #[tool(
        description = "Execute a whitelisted shell command (grep/cat/ls/find/sed/awk/…). No shell interpreter — args are passed literally. Path args validated against root."
    )]
    async fn shell_exec(
        &self,
        Parameters(p): Parameters<ShellExecParams>,
    ) -> Result<shell::ShellOutput, AppError> {
        shell::shell_exec(&self.state, p.command, p.args).await
    }
}

fn os_detail() -> &'static str {
    match std::env::consts::OS {
        "macos" => "BSD sed (macOS)",
        "linux" => "GNU sed (Linux)",
        "windows" => "Windows",
        other => other,
    }
}

fn sed_i_note() -> &'static str {
    match std::env::consts::OS {
        "macos" => r#"BSD sed requires an explicit backup suffix — use ["-i", "", "s/old/new/g", "/abs/path"] (the empty string is mandatory and is passed through correctly)"#,
        _ => r#"GNU sed — use ["-i", "s/old/new/g", "/abs/path"]"#,
    }
}

#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        let root = self.state.config.root.display().to_string();
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities {
                tools: Some(Default::default()),
                ..Default::default()
            },
            server_info: Implementation {
                name: "chisel".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                title: None,
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(format!(
                "Filesystem and shell tool server.\n\
                Root directory: {root}\n\
                Platform: {platform} ({os_detail})\n\
                IMPORTANT: All path arguments must be absolute paths starting with \"{root}/\". \
                Relative paths (\".\", \"./foo\", bare names) are rejected. \
                Always use the full path, e.g. \"{root}/myfile.txt\" not \"myfile.txt\" or \".\".\n\
                IMPORTANT: For sed -i on this platform: {sed_i_note}",
                platform = std::env::consts::OS,
                os_detail = os_detail(),
                sed_i_note = sed_i_note(),
            )),
        }
    }
}

// ──────────────────────────────────────────────
// HTTP server
// ──────────────────────────────────────────────

async fn mcp_handler(
    State(svc): State<Arc<StreamableHttpService<McpServer>>>,
    req: Request<Body>,
) -> Response<Body> {
    let resp = svc.handle(req).await;
    resp.map(|body| Body::new(body.map_err(|infallible| match infallible {})))
}

/// Build the axum router (used in integration tests and by `run_server`).
pub fn run_server_router(state: SharedState) -> Router {
    let mcp_svc = Arc::new(StreamableHttpService::new(
        {
            let state = state.clone();
            move || Ok(McpServer::new(state.clone()))
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    ));

    let body_limit = state.config.body_limit_bytes;
    let rate_limit_rps = state.config.rate_limit_rps;

    let router = Router::new()
        .route("/mcp", axum::routing::any(mcp_handler))
        .with_state(mcp_svc)
        // Auth is innermost — only authenticated requests reach the handler.
        .layer(middleware::from_fn_with_state(state, auth_layer))
        // Body limit is outermost — reject oversized bodies before any work.
        .layer(DefaultBodyLimit::max(body_limit));

    // Rate limit: reject with 429 when burst is exceeded.
    // governor uses a token-bucket; no thread serialisation.
    if rate_limit_rps > 0 {
        let quota = Quota::per_second(NonZeroU32::new(rate_limit_rps as u32).unwrap());
        let limiter: Arc<DefaultDirectRateLimiter> = Arc::new(RateLimiter::direct(quota));
        router.layer(middleware::from_fn(move |req: Request, next: Next| {
            let limiter = limiter.clone();
            async move {
                if limiter.check().is_err() {
                    StatusCode::TOO_MANY_REQUESTS.into_response()
                } else {
                    next.run(req).await
                }
            }
        }))
    } else {
        router
    }
}

pub async fn run_server(state: SharedState) -> anyhow::Result<()> {
    let port = state.config.port;
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;

    let app = run_server_router(state);

    let listener = TcpListener::bind(addr).await?;
    eprintln!("mcp-fs listening on http://{addr}/mcp");

    axum::serve(listener, app).await?;
    Ok(())
}

/// Run as an MCP stdio server — used when launched as a .mcpb Desktop Extension.
/// Reads JSON-RPC from stdin, writes to stdout. No HTTP, no auth header required.
pub async fn run_server_stdio(state: SharedState) -> anyhow::Result<()> {
    let server = McpServer::new(state);
    let running = server.serve(transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
