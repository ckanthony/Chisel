pub mod filesystem;
pub mod shell;

use crate::{error::AppError, state::AppState};

/// Guard: returns `Err(AppError::ReadOnly)` when the server is in read-only mode.
/// Call at the top of every write tool handler.
pub fn check_writable(state: &AppState) -> Result<(), AppError> {
    if state.config.read_only {
        Err(AppError::ReadOnly)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::state::AppState;
    use std::path::PathBuf;

    fn state(read_only: bool) -> AppState {
        let cfg =
            Config::from_parts(PathBuf::from("/tmp"), 3000, Some("tok".into()), read_only).unwrap();
        AppState { config: cfg }
    }

    #[test]
    fn writable_state_passes() {
        assert!(check_writable(&state(false)).is_ok());
    }

    #[test]
    fn read_only_state_fails() {
        let err = check_writable(&state(true)).unwrap_err();
        assert!(matches!(err, AppError::ReadOnly));
    }
}
