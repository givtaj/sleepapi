//! Bounded in-memory recovery sequences. All state transitions are synchronous;
//! no lock is held while waiting or serializing an HTTP response.
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{
        Path, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::{
    AppState, WaitMethod, default_duration,
    error::{ApiError, ErrorDetails, validate_simulated_status, with_simulation_headers},
};

const MAX_RUNS: usize = 64;
const MAX_SEQUENCE_STEPS: usize = 64;
const MAX_ATTEMPTS_PER_RUN: u64 = 1024;
const MAX_RECEIPTS_PER_RUN: usize = 32;
const DEFAULT_TTL_MS: u64 = 300_000;
const MAX_TTL_MS: u64 = 3_600_000;

#[derive(Default)]
pub(crate) struct RunStore {
    next_id: u64,
    runs: HashMap<String, Run>,
}

struct Run {
    generation: u64,
    attempts_started: u64,
    generation_attempts: usize,
    ttl_ms: u64,
    expires_at: Instant,
    sequence: Vec<Step>,
    receipts: VecDeque<Receipt>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Step {
    status_code: u16,
    #[serde(default = "default_duration")]
    duration_ms: u64,
    #[serde(default)]
    method: WaitMethod,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRun {
    sequence: Vec<Step>,
    #[serde(default = "default_ttl")]
    ttl_ms: u64,
}

fn default_ttl() -> u64 {
    DEFAULT_TTL_MS
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationRequest {
    generation: u64,
}

#[derive(Clone, Serialize)]
struct Receipt {
    generation: u64,
    attempt: usize,
    sequence_step: usize,
    selected_status_code: u16,
    method: WaitMethod,
    requested_duration_ms: u64,
    outcome: Outcome,
    elapsed_ms: Option<f64>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Pending,
    Completed,
    HandlerCancelled,
    WaitFailed,
}

#[derive(Serialize)]
struct Summary {
    run_id: String,
    generation: u64,
    attempts_started: u64,
    generation_attempts: usize,
    ttl_ms: u64,
    expires_in_ms: u64,
    sequence: Vec<Step>,
    retained_receipts: usize,
}

impl Run {
    fn summary(&self, id: &str, now: Instant) -> Summary {
        Summary {
            run_id: id.into(),
            generation: self.generation,
            attempts_started: self.attempts_started,
            generation_attempts: self.generation_attempts,
            ttl_ms: self.ttl_ms,
            expires_in_ms: self.expires_at.saturating_duration_since(now).as_millis() as u64,
            sequence: self.sequence.clone(),
            retained_receipts: self.receipts.len(),
        }
    }

    fn check_generation(&self, generation: u64) -> Result<(), ApiError> {
        if self.generation != generation {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "stale generation; inspect the run before retrying",
            ));
        }
        Ok(())
    }
}

impl RunStore {
    fn prune(&mut self, now: Instant) {
        self.runs.retain(|_, run| now < run.expires_at);
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut Run, ApiError> {
        self.runs
            .get_mut(id)
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "run not found or expired"))
    }
}

fn lock(store: &Mutex<RunStore>) -> Result<MutexGuard<'_, RunStore>, ApiError> {
    store
        .lock()
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "run state unavailable"))
}

fn path_id(path: Result<Path<String>, PathRejection>) -> Result<String, ApiError> {
    path.map(|Path(id)| id)
        .map_err(|error| ApiError::new(error.status(), error.body_text()))
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/runs", get(limits).post(create))
        .route("/runs/{run_id}", get(inspect).delete(delete))
        .route("/runs/{run_id}/receipts", get(receipts))
        .route("/runs/{run_id}/attempts", post(attempt))
        .route("/runs/{run_id}/reset", post(reset))
}

async fn limits(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let active_runs = {
        let mut store = lock(&state.runs)?;
        store.prune(Instant::now());
        store.runs.len()
    };
    Ok(Json(json!({
        "active_runs": active_runs,
        "limits": {
            "max_runs": MAX_RUNS,
            "max_sequence_steps": MAX_SEQUENCE_STEPS,
            "max_attempts_per_run": MAX_ATTEMPTS_PER_RUN,
            "max_receipts_per_run": MAX_RECEIPTS_PER_RUN,
            "default_ttl_ms": DEFAULT_TTL_MS,
            "max_ttl_ms": MAX_TTL_MS,
        },
    })))
}

async fn create(
    State(state): State<AppState>,
    payload: Result<Json<CreateRun>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(request) = payload.map_err(ApiError::from)?;
    if request.sequence.is_empty() || request.sequence.len() > MAX_SEQUENCE_STEPS {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "sequence must contain 1..=64 steps",
        ));
    }
    if !(1..=MAX_TTL_MS).contains(&request.ttl_ms) {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "ttl_ms must be in 1..=3600000",
        ));
    }
    for step in &request.sequence {
        validate_simulated_status(step.status_code)?;
        if step.duration_ms > state.config.max_duration_ms {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "duration_ms must be in 0..={}",
                    state.config.max_duration_ms,
                ),
            ));
        }
    }
    let summary = {
        let mut store = lock(&state.runs)?;
        let now = Instant::now();
        store.prune(now);
        if store.runs.len() >= MAX_RUNS {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "run limit reached; delete a run or wait for expiry",
            ));
        }
        // IDs are never reused within a router's lifetime, including after deletion.
        let next_id = store.next_id.checked_add(1).ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "run identifier space exhausted",
            )
        })?;
        let id = format!("run-{next_id}");
        let run = Run {
            generation: 1,
            attempts_started: 0,
            generation_attempts: 0,
            ttl_ms: request.ttl_ms,
            expires_at: now + Duration::from_millis(request.ttl_ms),
            sequence: request.sequence,
            receipts: VecDeque::new(),
        };
        let summary = run.summary(&id, now);
        store.next_id = next_id;
        store.runs.insert(id, run);
        summary
    };
    Ok((StatusCode::CREATED, Json(summary)).into_response())
}

async fn inspect(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<Summary>, ApiError> {
    let id = path_id(path)?;
    let summary = {
        let mut store = lock(&state.runs)?;
        let now = Instant::now();
        store.prune(now);
        store.get_mut(&id)?.summary(&id, now)
    };
    Ok(Json(summary))
}

async fn receipts(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<Value>, ApiError> {
    let id = path_id(path)?;
    let (generation, receipts) = {
        let mut store = lock(&state.runs)?;
        store.prune(Instant::now());
        let run = store.get_mut(&id)?;
        (run.generation, run.receipts.clone())
    };
    Ok(Json(
        json!({ "run_id": id, "generation": generation, "receipts": receipts }),
    ))
}

async fn reset(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    payload: Result<Json<GenerationRequest>, JsonRejection>,
) -> Result<Json<Summary>, ApiError> {
    let id = path_id(path)?;
    let Json(request) = payload.map_err(ApiError::from)?;
    let summary = {
        let mut store = lock(&state.runs)?;
        let now = Instant::now();
        store.prune(now);
        let run = store.get_mut(&id)?;
        run.check_generation(request.generation)?;
        run.generation = run.generation.checked_add(1).ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "generation space exhausted; create a new run",
            )
        })?;
        run.generation_attempts = 0;
        run.receipts.clear();
        run.summary(&id, now)
    };
    Ok(Json(summary))
}

async fn delete(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let id = path_id(path)?;
    let mut store = lock(&state.runs)?;
    store.prune(Instant::now());
    store.get_mut(&id)?;
    store.runs.remove(&id);
    Ok(StatusCode::NO_CONTENT)
}

struct AttemptGuard {
    store: Arc<Mutex<RunStore>>,
    run_id: String,
    generation: u64,
    attempt: usize,
    started: Instant,
    finished: bool,
}

impl AttemptGuard {
    fn finish(&mut self, outcome: Outcome) {
        if self.finished {
            return;
        }
        self.finished = true;
        // Cancellation cleanup cannot return an API error. A poisoned store is
        // unavailable to future handlers too; do not panic during unwinding.
        let Ok(mut store) = self.store.lock() else {
            return;
        };
        let now = Instant::now();
        store.prune(now);
        let Some(run) = store.runs.get_mut(&self.run_id) else {
            return;
        };
        if run.generation != self.generation {
            return;
        }
        // Never insert here: a reset, deletion, expiry, or retention eviction
        // must not be undone by an old request finishing or being dropped.
        if let Some(receipt) = run.receipts.iter_mut().find(|r| r.attempt == self.attempt) {
            receipt.outcome = outcome;
            receipt.elapsed_ms = Some(now.duration_since(self.started).as_secs_f64() * 1000.0);
        }
    }
}

impl Drop for AttemptGuard {
    fn drop(&mut self) {
        self.finish(Outcome::HandlerCancelled);
    }
}

async fn attempt(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    payload: Result<Json<GenerationRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let id = path_id(path)?;
    let Json(request) = payload.map_err(ApiError::from)?;
    // Validation and global admission precede sequence consumption. A rejected
    // request consumes no step. This permit is released if reservation fails.
    let permit = state
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::capacity_exceeded())?;
    let (step, mut guard) = {
        let mut store = lock(&state.runs)?;
        let now = Instant::now();
        store.prune(now);
        let run = store.get_mut(&id)?;
        run.check_generation(request.generation)?;
        if run.attempts_started >= MAX_ATTEMPTS_PER_RUN {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "run attempt budget exhausted; create a new run",
            ));
        }
        let Some(step) = run.sequence.get(run.generation_attempts).copied() else {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "sequence exhausted; reset the run before retrying",
            ));
        };
        run.attempts_started += 1;
        run.generation_attempts += 1;
        let attempt = run.generation_attempts;
        if run.receipts.len() == MAX_RECEIPTS_PER_RUN {
            run.receipts.pop_front();
        }
        run.receipts.push_back(Receipt {
            generation: run.generation,
            attempt,
            sequence_step: attempt,
            selected_status_code: step.status_code,
            method: step.method,
            requested_duration_ms: step.duration_ms,
            outcome: Outcome::Pending,
            elapsed_ms: None,
        });
        let guard = AttemptGuard {
            store: state.runs.clone(),
            run_id: id.clone(),
            generation: run.generation,
            attempt,
            started: now,
            finished: false,
        };
        (step, guard)
    };
    if let Err(error) = step
        .method
        .wait(Duration::from_millis(step.duration_ms), permit)
        .await
    {
        guard.finish(Outcome::WaitFailed);
        tracing::error!(%error, "waiting task failed");
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "waiting task failed",
        ));
    }
    let elapsed_ms = guard.started.elapsed().as_secs_f64() * 1000.0;
    guard.finish(Outcome::Completed);
    // Status was validated before storing the sequence.
    let status = validate_simulated_status(step.status_code)?;
    let mut result = json!({
        "operation": "run", "simulated": true, "run_id": id,
        "generation": guard.generation, "attempt": guard.attempt, "sequence_step": guard.attempt,
        "method": step.method, "requested_duration_ms": step.duration_ms,
        "elapsed_ms": elapsed_ms, "status_code": step.status_code,
        "blocks_thread": step.method.blocks_thread(),
    });
    if status.is_client_error() || status.is_server_error() {
        result["error"] = json!(ErrorDetails::simulated(status));
    }
    Ok(with_simulation_headers(
        (status, Json(result)).into_response(),
    ))
}
