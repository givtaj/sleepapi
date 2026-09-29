mod config;
mod error;
mod scenario;
mod wait;

use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{MethodRouter, get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;

pub use config::Config;
use error::{ApiError, ErrorDetails, validate_simulated_status, with_simulation_headers};
use scenario::Scenario;
pub use wait::WaitMethod;

#[derive(Clone)]
struct AppState {
    config: Config,
    slots: Arc<Semaphore>,
}

pub fn router(config: Config) -> std::io::Result<Router> {
    config.validate()?;
    let cors_origins = config.cors_origins()?;
    let state = AppState {
        slots: Arc::new(Semaphore::new(config.max_in_flight)),
        config,
    };
    let app = Router::new()
        .route("/health", get(|| async { Json(json!({ "status": "ok" })) }))
        .route("/methods", get(methods))
        .route("/scenarios", get(scenarios))
        .route("/sleep", post(sleep))
        .route("/sleep/tokio", sleep_route(WaitMethod::TokioSleep))
        .route("/sleep/tokio-spawn", sleep_route(WaitMethod::TokioSpawn))
        .route("/sleep/thread", sleep_route(WaitMethod::ThreadSleep))
        .route("/sleep/park", sleep_route(WaitMethod::ThreadPark))
        .route("/simulate/{operation}", post(simulate))
        .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "route not found") })
        .method_not_allowed_fallback(|| async {
            ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "HTTP method not allowed for this route",
            )
        })
        .layer(DefaultBodyLimit::max(8 * 1024))
        .with_state(state);
    if cors_origins.is_empty() {
        return Ok(app);
    }
    Ok(app.layer(
        CorsLayer::new()
            .allow_origin(cors_origins)
            .allow_methods([Method::GET, Method::HEAD, Method::POST])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
            .expose_headers([header::RETRY_AFTER, header::WWW_AUTHENTICATE, header::ALLOW]),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelayRequest {
    duration_ms: Option<u64>,
    #[serde(default)]
    method: WaitMethod,
    status_code: Option<u16>,
    scenario: Option<Scenario>,
}

// Named routes select the method themselves, so a body cannot override it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SleepRequest {
    duration_ms: Option<u64>,
    status_code: Option<u16>,
    scenario: Option<Scenario>,
}

fn sleep_route(method: WaitMethod) -> MethodRouter<AppState> {
    post(
        move |State(state): State<AppState>, payload: Result<Json<SleepRequest>, JsonRejection>| async move {
            let Json(request) = payload.map_err(ApiError::from)?;
            run_delay(
                state,
                "sleep",
                DelayRequest {
                    duration_ms: request.duration_ms,
                    method,
                    status_code: request.status_code,
                    scenario: request.scenario,
                },
            )
            .await
        },
    )
}

fn default_duration() -> u64 {
    400
}
fn default_status() -> u16 {
    200
}

async fn scenarios() -> Json<Value> {
    let scenarios: Vec<_> = Scenario::ALL
        .iter()
        .map(|scenario| {
            let (duration_ms, status_code) = scenario.defaults();
            json!({
                "scenario": scenario,
                "duration_ms": duration_ms,
                "status_code": status_code,
                "description": scenario.description(),
            })
        })
        .collect();
    Json(json!({"scenarios": scenarios}))
}

async fn methods(State(state): State<AppState>) -> Json<Value> {
    let methods: Vec<_> = WaitMethod::ALL
        .iter()
        .map(|method| {
            json!({
                "method": method,
                "description": method.description(),
                "blocks_thread": method.blocks_thread(),
            })
        })
        .collect();
    Json(json!({
        "methods": methods,
        "defaults": { "duration_ms": default_duration(), "method": WaitMethod::default(), "status_code": default_status() },
        "limits": { "max_duration_ms": state.config.max_duration_ms, "max_in_flight": state.config.max_in_flight },
    }))
}

async fn sleep(
    State(state): State<AppState>,
    payload: Result<Json<DelayRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload.map_err(ApiError::from)?;
    run_delay(state, "sleep", request).await
}

async fn simulate(
    State(state): State<AppState>,
    operation: Result<Path<String>, PathRejection>,
    payload: Result<Json<DelayRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Path(operation) =
        operation.map_err(|error| ApiError::new(error.status(), error.body_text()))?;
    if !matches!(operation.as_str(), "email" | "sms") {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown simulation; use email or sms",
        ));
    }
    let Json(request) = payload.map_err(ApiError::from)?;
    run_delay(state, &operation, request).await
}

async fn run_delay(
    state: AppState,
    operation: &str,
    request: DelayRequest,
) -> Result<Response, ApiError> {
    if request.scenario.is_some() && request.status_code.is_some() {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "scenario selects the response status; omit status_code or omit scenario",
        ));
    }
    let (preset_duration, preset_status) = request
        .scenario
        .map(Scenario::defaults)
        .unwrap_or((default_duration(), default_status()));
    let duration_ms = request.duration_ms.unwrap_or(preset_duration);
    let status_code = request.status_code.unwrap_or(preset_status);
    if duration_ms > state.config.max_duration_ms {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "duration_ms must be in 0..={}",
                state.config.max_duration_ms
            ),
        ));
    }
    let status = validate_simulated_status(status_code)?;
    let permit = state
        .slots
        .try_acquire_owned()
        .map_err(|_| ApiError::capacity_exceeded())?;

    let started = tokio::time::Instant::now();
    request
        .method
        .wait(Duration::from_millis(duration_ms), permit)
        .await
        .map_err(|error| {
            tracing::error!(%error, "waiting task failed");
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "waiting task failed")
        })?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    tracing::info!(operation, method = ?request.method, duration_ms, elapsed_ms, status_code, "simulation complete");

    let mut result = json!({
        "operation": operation,
        "simulated": true,
        "method": request.method,
        "requested_duration_ms": duration_ms,
        "elapsed_ms": elapsed_ms,
        "status_code": status_code,
        "blocks_thread": request.method.blocks_thread(),
    });
    if let Some(scenario) = request.scenario {
        result["scenario"] = json!(scenario);
    }
    if status.is_client_error() || status.is_server_error() {
        result["error"] = json!(ErrorDetails::simulated(status));
    }
    Ok(with_simulation_headers(
        (status, Json(result)).into_response(),
    ))
}
