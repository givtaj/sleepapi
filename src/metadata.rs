//! Bounded declarations, separate from health and mutable run state.
use axum::{
    Extension, Json,
    extract::{ConnectInfo, State},
    http::{Version, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::{
    AppState, MAX_REQUEST_BODY_BYTES, WaitMethod, fixtures, runs,
    scenario::Scenario,
    transport::{self, FaultConnection},
};

pub(crate) async fn get(
    State(state): State<AppState>,
    version: Version,
    connection: Option<Extension<ConnectInfo<FaultConnection>>>,
) -> Response {
    let faults = transport::faults_supported(version, connection.is_some());
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "schema_version": 1,
            "contract_version": 1,
            // A declaration from Cargo, not a verified source or image identity.
            "software": { "name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION") },
            "capabilities": {
                "delays": true,
                "status_codes": true,
                "recovery_runs": true,
                "run_reset": true,
                "run_receipts": true,
                "response_fixtures": true,
                "connection_drop": faults,
                "truncated_response": faults,
                "persistent_runs": false,
            },
            "protocols": { "transport_faults": [format!("{:?}", transport::FAULT_HTTP_VERSION)] },
            "wait_methods": WaitMethod::ALL,
            "scenarios": Scenario::ALL,
            "limits": {
                "max_duration_ms": state.config.max_duration_ms,
                "max_in_flight": state.config.max_in_flight,
                "max_request_body_bytes": MAX_REQUEST_BODY_BYTES,
                "runs": runs::declared_limits(),
                "fixtures": {
                    "max_body_bytes": fixtures::MAX_FIXTURE_BODY_BYTES,
                    "max_headers": fixtures::MAX_FIXTURE_HEADERS,
                    "max_header_value_bytes": fixtures::MAX_FIXTURE_HEADER_VALUE_BYTES,
                    "content_types": fixtures::CONTENT_TYPES,
                    "header_names": fixtures::HEADER_NAMES,
                },
            },
        })),
    )
        .into_response()
}
