use chisel::{
    config::{Config, Transport},
    server::{run_server, run_server_stdio},
    state::AppState,
};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() {
    // Always write logs to stderr — stdout is reserved for MCP stdio transport.
    fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("chisel=info")),
        )
        .init();

    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    let transport = config.transport.clone();
    let state = AppState::new(config);

    let result = match transport {
        Transport::Stdio => run_server_stdio(state).await,
        Transport::Http => run_server(state).await,
    };

    if let Err(e) = result {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}
