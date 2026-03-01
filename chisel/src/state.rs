use std::sync::Arc;

use crate::config::Config;

pub struct AppState {
    pub config: Config,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn new(config: Config) -> SharedState {
        Arc::new(Self { config })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    #[test]
    fn app_state_wraps_config() {
        let cfg =
            Config::from_parts(PathBuf::from("/srv"), 3000, Some("tok".into()), false).unwrap();
        let state = AppState::new(cfg);
        assert_eq!(state.config.secret, "tok");
        assert_eq!(state.config.port, 3000);
    }

    #[test]
    fn shared_state_is_arc() {
        let cfg =
            Config::from_parts(PathBuf::from("/srv"), 3000, Some("tok".into()), false).unwrap();
        let state = AppState::new(cfg);
        let cloned = Arc::clone(&state);
        assert_eq!(Arc::strong_count(&state), 2);
        assert_eq!(cloned.config.secret, "tok");
    }
}
