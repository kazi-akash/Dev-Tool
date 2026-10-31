use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::actions::automation::ActionsState;
use crate::actions::engine::ActionRecord;
use crate::actions::model::{ActionError, ActionSpec, Confirmation, Ctx, Facts, Preview};
use crate::actions::real::{RealKiller, RealRunner};
use crate::monitoring::service::MonitorState;
use crate::storage::state::StorageState;

/// Fresh facts for every request: the engine never trusts anything cached by the UI.
fn build_facts(monitor: &MonitorState, storage: &StorageState) -> Facts {
    Facts {
        processes: monitor.processes.snapshot(),
        scan_entries: storage
            .latest_result()
            .map(|r| r.entries)
            .unwrap_or_default(),
        temp_dir: std::env::var_os("TEMP").map(Into::into),
        user_profile: std::env::var_os("USERPROFILE").map(Into::into),
    }
}

#[tauri::command]
pub fn list_actions(actions: State<'_, Arc<ActionsState>>) -> Vec<ActionSpec> {
    actions.engine.list()
}

#[tauri::command]
pub async fn preview_action(
    actions: State<'_, Arc<ActionsState>>,
    monitor: State<'_, Arc<MonitorState>>,
    storage: State<'_, Arc<StorageState>>,
    action_id: String,
    params: Value,
) -> Result<Preview, ActionError> {
    let (actions, monitor, storage) = (
        actions.inner().clone(),
        monitor.inner().clone(),
        storage.inner().clone(),
    );
    tauri::async_runtime::spawn_blocking(move || {
        let facts = build_facts(&monitor, &storage);
        let ctx = Ctx {
            facts: &facts,
            runner: &RealRunner,
            killer: &RealKiller,
        };
        actions.engine.preview(&action_id, &params, &ctx)
    })
    .await
    .map_err(|e| ActionError::Failed(e.to_string()))?
}

#[tauri::command]
pub async fn execute_action(
    actions: State<'_, Arc<ActionsState>>,
    monitor: State<'_, Arc<MonitorState>>,
    storage: State<'_, Arc<StorageState>>,
    action_id: String,
    params: Value,
    confirmed: bool,
    double_confirmed: bool,
) -> Result<ActionRecord, ActionError> {
    let (actions, monitor, storage) = (
        actions.inner().clone(),
        monitor.inner().clone(),
        storage.inner().clone(),
    );
    tauri::async_runtime::spawn_blocking(move || {
        let facts = build_facts(&monitor, &storage);
        let ctx = Ctx {
            facts: &facts,
            runner: &RealRunner,
            killer: &RealKiller,
        };
        actions.engine.execute(
            &action_id,
            &params,
            Confirmation {
                confirmed,
                double_confirmed,
            },
            &ctx,
        )
    })
    .await
    .map_err(|e| ActionError::Failed(e.to_string()))?
}

#[tauri::command]
pub async fn rollback_action(
    actions: State<'_, Arc<ActionsState>>,
    monitor: State<'_, Arc<MonitorState>>,
    storage: State<'_, Arc<StorageState>>,
    record_id: u64,
) -> Result<String, ActionError> {
    let (actions, monitor, storage) = (
        actions.inner().clone(),
        monitor.inner().clone(),
        storage.inner().clone(),
    );
    tauri::async_runtime::spawn_blocking(move || {
        let facts = build_facts(&monitor, &storage);
        let ctx = Ctx {
            facts: &facts,
            runner: &RealRunner,
            killer: &RealKiller,
        };
        actions.engine.rollback(record_id, &ctx)
    })
    .await
    .map_err(|e| ActionError::Failed(e.to_string()))?
}

#[tauri::command]
pub fn get_action_history(actions: State<'_, Arc<ActionsState>>) -> Vec<ActionRecord> {
    actions.engine.history()
}

#[tauri::command]
pub fn get_safe_automation(actions: State<'_, Arc<ActionsState>>) -> bool {
    actions.automation.enabled()
}

#[tauri::command]
pub fn set_safe_automation(actions: State<'_, Arc<ActionsState>>, enabled: bool) {
    actions.automation.set_enabled(enabled);
}
