use chisel_core::ops::shell;
use rmcp::model::{Content, IntoContents};
use tracing::{info, warn};

use crate::{error::AppError, state::AppState};

/// Re-exported output type with rmcp glue attached.
#[derive(Debug)]
pub struct ShellOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl std::fmt::Display for ShellOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exit_code: {}\nstdout:\n{}\nstderr:\n{}",
            self.exit_code, self.stdout, self.stderr
        )
    }
}

impl IntoContents for ShellOutput {
    fn into_contents(self) -> Vec<Content> {
        vec![Content::text(self.to_string())]
    }
}

impl From<shell::ShellOutput> for ShellOutput {
    fn from(s: shell::ShellOutput) -> Self {
        Self {
            exit_code: s.exit_code,
            stdout: s.stdout,
            stderr: s.stderr,
        }
    }
}

pub async fn shell_exec(
    state: &AppState,
    command: String,
    args: Vec<String>,
) -> Result<ShellOutput, AppError> {
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = shell::shell_exec(&state.config.root, &command, &args_ref)
        .map(ShellOutput::from)
        .map_err(AppError::from);
    match &result {
        Ok(out) => info!(op = "shell_exec", cmd = %command, exit_code = out.exit_code, "ok"),
        Err(e) => warn!(op = "shell_exec", cmd = %command, error = %e, "failed"),
    }
    result
}
