use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use subtle::ConstantTimeEq;

use crate::state::SharedState;

/// Extract the raw token from an `Authorization: Bearer <token>` header.
/// Returns `None` for absent, non-Bearer, or malformed headers.
fn extract_bearer_token(req: &Request<Body>) -> Option<&str> {
    let value = req.headers().get("Authorization")?.to_str().ok()?;
    value.strip_prefix("Bearer ")
}

/// Axum middleware: validates the `Authorization: Bearer` token against the
/// configured secret using constant-time comparison to prevent timing attacks.
pub async fn auth_layer(
    State(state): State<SharedState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    match extract_bearer_token(&req) {
        Some(token)
            if token
                .as_bytes()
                .ct_eq(state.config.secret.as_bytes())
                .into() =>
        {
            next.run(req).await
        }
        _ => Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, middleware, routing::get};
    use std::path::PathBuf;
    use tower::ServiceExt;

    use crate::{config::Config, state::AppState};

    fn test_state(secret: &str) -> SharedState {
        let cfg =
            Config::from_parts(PathBuf::from("/tmp"), 3000, Some(secret.into()), false).unwrap();
        AppState::new(cfg)
    }

    fn app(secret: &str) -> Router {
        let state = test_state(secret);
        Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(state, auth_layer))
    }

    async fn status(app: Router, req: Request<Body>) -> StatusCode {
        app.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn valid_token_is_accepted() {
        let req = Request::builder()
            .uri("/")
            .header("Authorization", "Bearer mysecret")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(app("mysecret"), req).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn missing_authorization_header_is_rejected() {
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        assert_eq!(status(app("mysecret"), req).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn wrong_token_is_rejected() {
        let req = Request::builder()
            .uri("/")
            .header("Authorization", "Bearer wrongtoken")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(app("mysecret"), req).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn basic_scheme_is_rejected() {
        let req = Request::builder()
            .uri("/")
            .header("Authorization", "Basic mysecret")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(app("mysecret"), req).await, StatusCode::UNAUTHORIZED);
    }
}
