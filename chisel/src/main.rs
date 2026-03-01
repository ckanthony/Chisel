use chisel::{config::Config, server::run_server, state::AppState};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("chisel=info")),
        )
        .init();

    let config = Config::load().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    let state = AppState::new(config);

    if let Err(e) = run_server(state).await {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}
