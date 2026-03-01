use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub root: PathBuf,
    pub port: u16,
    pub secret: String,
    pub read_only: bool,
    /// Maximum requests per second (token-bucket). 0 = unlimited.
    pub rate_limit_rps: u64,
    /// Maximum HTTP request body size in bytes.
    pub body_limit_bytes: usize,
}

#[derive(Parser, Debug)]
#[command(name = "mcp-fs")]
struct Cli {
    /// Root directory to serve
    #[arg(long)]
    root: PathBuf,

    /// Port to bind on (default: 3000)
    #[arg(long, default_value_t = 3000)]
    port: u16,

    /// Bearer token secret (overridden by MCP_APP_SECRET)
    #[arg(long)]
    secret: Option<String>,

    /// Disable all write operations
    #[arg(long)]
    read_only: bool,

    /// Maximum requests allowed per second; 0 to disable (default: 100)
    #[arg(long, default_value_t = 100)]
    rate_limit: u64,

    /// Maximum HTTP request body size in bytes (default: 4 MiB)
    #[arg(long, default_value_t = 4 * 1024 * 1024)]
    body_limit: usize,
}

/// Returns the effective secret, preferring `env` over `cli`.
/// Treats empty strings as absent.
pub fn resolve_secret(cli: Option<String>, env: Option<String>) -> Option<String> {
    env.filter(|s| !s.is_empty())
        .or_else(|| cli.filter(|s| !s.is_empty()))
}

impl Config {
    /// Parse CLI args, apply env-var override, fail-fast if secret is missing.
    pub fn load() -> Result<Self, String> {
        let cli = Cli::parse();
        let env_secret = std::env::var("MCP_APP_SECRET").ok();
        let secret = resolve_secret(cli.secret, env_secret)
            .ok_or("secret is required: set --secret or MCP_APP_SECRET")?;

        Ok(Config {
            root: cli.root,
            port: cli.port,
            secret,
            read_only: cli.read_only,
            rate_limit_rps: cli.rate_limit,
            body_limit_bytes: cli.body_limit,
        })
    }

    /// Construct and validate a Config from parts (used in tests and wiring).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_parts(
        root: PathBuf,
        port: u16,
        secret: Option<String>,
        read_only: bool,
    ) -> Result<Self, String> {
        let secret = secret
            .filter(|s| !s.is_empty())
            .ok_or("secret is required")?;
        Ok(Config {
            root,
            port,
            secret,
            read_only,
            rate_limit_rps: 100,
            body_limit_bytes: 4 * 1024 * 1024,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // --- resolve_secret ---

    #[test]
    fn env_takes_precedence_over_cli() {
        let result = resolve_secret(Some("cli-val".into()), Some("env-val".into()));
        assert_eq!(result, Some("env-val".into()));
    }

    #[test]
    fn cli_used_when_env_absent() {
        let result = resolve_secret(Some("cli-val".into()), None);
        assert_eq!(result, Some("cli-val".into()));
    }

    #[test]
    fn empty_env_falls_back_to_cli() {
        let result = resolve_secret(Some("cli-val".into()), Some("".into()));
        assert_eq!(result, Some("cli-val".into()));
    }

    #[test]
    fn empty_cli_and_no_env_gives_none() {
        let result = resolve_secret(Some("".into()), None);
        assert_eq!(result, None);
    }

    #[test]
    fn both_absent_gives_none() {
        let result = resolve_secret(None, None);
        assert_eq!(result, None);
    }

    // --- Config::from_parts ---

    #[test]
    fn missing_secret_fails_validation() {
        let err = Config::from_parts(PathBuf::from("/tmp"), 3000, None, false);
        assert!(err.is_err());
    }

    #[test]
    fn empty_secret_fails_validation() {
        let err = Config::from_parts(PathBuf::from("/tmp"), 3000, Some("".into()), false);
        assert!(err.is_err());
    }

    #[test]
    fn valid_config_is_ok() {
        let cfg = Config::from_parts(PathBuf::from("/tmp"), 3000, Some("s3cr3t".into()), false)
            .unwrap();
        assert_eq!(cfg.secret, "s3cr3t");
        assert_eq!(cfg.port, 3000);
        assert_eq!(cfg.root, PathBuf::from("/tmp"));
        assert!(!cfg.read_only);
    }

    #[test]
    fn read_only_flag_is_preserved() {
        let cfg =
            Config::from_parts(PathBuf::from("/data"), 8080, Some("tok".into()), true).unwrap();
        assert!(cfg.read_only);
        assert_eq!(cfg.port, 8080);
    }
}
