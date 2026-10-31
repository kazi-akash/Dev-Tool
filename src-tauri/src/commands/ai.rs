use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::diagnosis::gather;
use crate::ai::keystore::{SecretStore, clean_key};
use crate::ai::payload::{AiPayload, build_payload};
use crate::ai::provider::{AiConfig, AiError};
use crate::ai::{ask, client};
use crate::docker::DockerMonitor;
use crate::monitoring::service::MonitorState;
use crate::storage::state::StorageState;

type Store<'a> = State<'a, Arc<dyn SecretStore>>;

#[derive(Debug, Serialize)]
pub struct AiAnswer {
    /// Plain text. The UI must render it as text, never as HTML or as a command.
    pub text: String,
}

/// Whether a key is stored. The key itself is never returned.
#[tauri::command]
pub fn ai_key_status(store: Store<'_>) -> bool {
    store.get().map(|k| k.is_some()).unwrap_or(false)
}

#[tauri::command]
pub fn set_ai_key(store: Store<'_>, key: String) -> Result<(), String> {
    store.set(&clean_key(&key)?)
}

#[tauri::command]
pub fn clear_ai_key(store: Store<'_>) -> Result<(), String> {
    store.clear()
}

/// Exactly what would be sent to the provider, so the user can inspect it first.
#[tauri::command]
pub async fn preview_ai_payload(
    monitor: State<'_, Arc<MonitorState>>,
    docker: State<'_, Arc<DockerMonitor>>,
    storage: State<'_, Arc<StorageState>>,
) -> Result<AiPayload, String> {
    let (monitor, docker, storage) = (
        monitor.inner().clone(),
        docker.inner().clone(),
        storage.inner().clone(),
    );
    tauri::async_runtime::spawn_blocking(move || {
        let g = gather(&monitor, &docker, &storage)?;
        Ok(build_payload(
            &g.snapshot,
            &g.workloads,
            &g.processes,
            &g.report,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ask the configured provider. `enabled` must be true: the UI sends it from the opt-in setting, and
/// the backend refuses otherwise, so no code path sends data without the user turning AI on.
#[tauri::command]
pub async fn ask_ai(
    store: Store<'_>,
    monitor: State<'_, Arc<MonitorState>>,
    docker: State<'_, Arc<DockerMonitor>>,
    storage: State<'_, Arc<StorageState>>,
    enabled: bool,
    config: AiConfig,
    question: Option<String>,
) -> Result<AiAnswer, AiError> {
    if !enabled {
        return Err(AiError::NotConfigured(
            "AI assistance is turned off in Settings.".into(),
        ));
    }
    let key = store.get().map_err(AiError::NotConfigured)?;
    let (monitor, docker, storage) = (
        monitor.inner().clone(),
        docker.inner().clone(),
        storage.inner().clone(),
    );

    tauri::async_runtime::spawn_blocking(move || {
        let g = gather(&monitor, &docker, &storage).map_err(AiError::NotConfigured)?;
        let payload = build_payload(&g.snapshot, &g.workloads, &g.processes, &g.report);
        let text = ask(
            &config,
            key.as_deref(),
            &payload,
            question.as_deref(),
            &client::send,
        )?;
        Ok(AiAnswer { text })
    })
    .await
    .map_err(|e| AiError::Network(e.to_string()))?
}
