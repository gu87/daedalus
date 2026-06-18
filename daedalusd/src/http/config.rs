//! P4.4: `GET /api/config/models` — read-only model list summary.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use super::health::HttpState;

#[derive(Serialize)]
pub(crate) struct ModelsResponse {
    pub(crate) models: Vec<ModelSummary>,
}

#[derive(Serialize)]
pub(crate) struct ModelSummary {
    pub(crate) id: String,
    pub(crate) provider: String,
    #[serde(rename = "type")]
    pub(crate) provider_type: String,
}

#[derive(Serialize)]
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
}

pub(crate) async fn list_models(
    State(state): State<Arc<HttpState>>,
) -> Result<Json<ModelsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let path = state.ctx.config.models_yaml_path.clone();

    let models = tokio::task::spawn_blocking(move || {
        let config = match crate::config::load_models_yaml(&path) {
            Ok(c) => c,
            Err(crate::error::DaedalusError::ConfigMissing { .. }) => {
                // File not found → empty list, not an error.
                return Ok(Vec::new());
            }
            Err(e) => return Err(format!("{e}")),
        };

        let mut summaries = Vec::new();
        for entry in &config.models {
            // Resolve provider type.
            let provider_type = match config.providers.get(&entry.provider) {
                Some(pe) => pe.provider_type.clone(),
                None => {
                    return Err(format!(
                        "model '{}' references unknown provider '{}'",
                        entry.id, entry.provider
                    ));
                }
            };
            summaries.push(ModelSummary {
                id: entry.id.clone(),
                provider: entry.provider.clone(),
                provider_type,
            });
        }
        Ok(summaries)
    })
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "spawn_blocking panic".into(),
            }),
        )
    })?
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: e }),
        )
    })?;

    Ok(Json(ModelsResponse { models }))
}
