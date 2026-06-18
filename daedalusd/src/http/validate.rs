//! P4.5: `POST /api/models/validate` — single-model connectivity probe.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::config::ModelStrategy;
use crate::llm::router::Router;
use crate::types::{ChatMessage, ModelConfig};

use super::health::HttpState;

// ── request / response types ──────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ValidateRequest {
    model_id: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ValidateResponse {
    model_id: String,
    reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ErrorResponse {
    error: String,
}

// ── handler ───────────────────────────────────────────────────────────

pub(crate) async fn validate_model(
    State(state): State<Arc<HttpState>>,
    Json(body): Json<ValidateRequest>,
) -> Result<Json<ValidateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // 1. model_id must be present and non-empty.
    let model_id = match body.model_id.as_deref() {
        None | Some("") => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "model_id is required".into(),
                }),
            ));
        }
        Some(id) => id.to_string(),
    };

    let models_path = state.ctx.config.models_yaml_path.clone();

    // 2. Load models.yaml, find the local model entry.
    let models_config = crate::config::load_models_yaml(&models_path).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("{e}"),
            }),
        )
    })?;

    let _model_entry = models_config
        .models
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("unknown model_id: {model_id}"),
                }),
            )
        })?;

    // 3. Build a temporary Router for this model only.
    let strategy = ModelStrategy {
        primary: ModelConfig {
            model: model_id.clone(),
            max_tokens: 1,
            temperature: 0.0,
        },
        fallback_chain: vec![],
    };

    let router = Router::from_models_config(&models_config, &strategy).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("{e}"),
            }),
        )
    })?;

    // 4. Send a minimal probe with a 10 s timeout.
    let messages = vec![ChatMessage {
        role: "user".into(),
        content: "ping".into(),
    }];
    let tools = vec![];

    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        router.chat_with_fallback(&strategy, &messages, &tools),
    )
    .await;

    let latency_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(_response)) => Ok(Json(ValidateResponse {
            model_id,
            reachable: true,
            latency_ms: Some(latency_ms),
            error: None,
        })),
        Ok(Err(provider_err)) => {
            // Use Display, not Debug — avoid leaking response bodies.
            Ok(Json(ValidateResponse {
                model_id,
                reachable: false,
                latency_ms: None,
                error: Some(format!("{provider_err}")),
            }))
        }
        Err(_elapsed) => Ok(Json(ValidateResponse {
            model_id,
            reachable: false,
            latency_ms: None,
            error: Some("timeout after 10s".into()),
        })),
    }
}
