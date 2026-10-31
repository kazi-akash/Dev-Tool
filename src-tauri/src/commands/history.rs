use std::sync::{Arc, OnceLock};

use serde::Serialize;
use tauri::State;

use crate::database::{StoredAction, StoredDiagnosis};
use crate::history::aggregate::Row;
use crate::history::{HistoryHandle, now_ms};

/// Set at startup if the database opened; absent otherwise (history degrades, the app does not).
pub type HistorySlot = Arc<OnceLock<HistoryHandle>>;

const MIN_RANGE_SECS: i64 = 60;
const MAX_RANGE_SECS: i64 = 180 * 24 * 3600;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryResponse<T: Serialize> {
    /// False when the database could not be opened.
    pub available: bool,
    pub items: Vec<T>,
}

fn unavailable<T: Serialize>() -> HistoryResponse<T> {
    HistoryResponse {
        available: false,
        items: vec![],
    }
}

#[tauri::command]
pub async fn get_history(
    slot: State<'_, HistorySlot>,
    range_secs: i64,
) -> Result<HistoryResponse<Row>, String> {
    if !(MIN_RANGE_SECS..=MAX_RANGE_SECS).contains(&range_secs) {
        return Err(format!(
            "range must be between {MIN_RANGE_SECS} seconds and 180 days"
        ));
    }
    let Some(h) = slot.get() else {
        return Ok(unavailable());
    };
    let items = h.db().query_history(now_ms(), range_secs * 1000).await?;
    Ok(HistoryResponse {
        available: true,
        items,
    })
}

#[tauri::command]
pub async fn get_recent_diagnoses(
    slot: State<'_, HistorySlot>,
    limit: Option<i64>,
) -> Result<HistoryResponse<StoredDiagnosis>, String> {
    let Some(h) = slot.get() else {
        return Ok(unavailable());
    };
    Ok(HistoryResponse {
        available: true,
        items: h.db().recent_diagnoses(limit.unwrap_or(20)).await?,
    })
}

#[tauri::command]
pub async fn get_recent_actions(
    slot: State<'_, HistorySlot>,
    limit: Option<i64>,
) -> Result<HistoryResponse<StoredAction>, String> {
    let Some(h) = slot.get() else {
        return Ok(unavailable());
    };
    Ok(HistoryResponse {
        available: true,
        items: h.db().recent_actions(limit.unwrap_or(50)).await?,
    })
}
