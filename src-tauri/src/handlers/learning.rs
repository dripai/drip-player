use crate::app_state::{lock, AppState};
use crate::models::learning::{LearningSettings, LearningTextResult, Recording, Transcript};
use crate::services::{learning, learning_models};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub async fn start_learning_transcription(
    media_id: String,
    source: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let _submission = state.transcription_submission.lock().await;
    let settings = {
        let db = lock(&state.database)?;
        db.media(&media_id)?;
        if !db.pending_transcriptions(&media_id)?.is_empty() {
            return Err("该媒体已有转写任务，请先查询已有任务".into());
        }
        db.learning_settings()?
    };
    let id = learning_models::submit_transcription(&settings, &source).await?;
    lock(&state.database)?
        .save_transcription_job(&id, &media_id, &settings)
        .map_err(|e| format!("转写任务已提交（{id}），本地记录失败: {e}"))?;
    Ok(id)
}
#[tauri::command]
pub fn pending_learning_transcriptions(
    media_id: String,
    state: State<AppState>,
) -> Result<Vec<String>, String> {
    lock(&state.database)?.pending_transcriptions(&media_id)
}
#[tauri::command]
pub async fn poll_learning_transcription(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<Transcript>, String> {
    let (media_id, settings, status) = lock(&state.database)?.transcription_job(&id)?;
    if status != "pending" {
        return Err("转写任务已结束，请重新载入字幕".into());
    }
    // Transport errors leave the job pending so retrying does not submit a second paid task.
    let result = match learning_models::poll_transcription(&settings, &id).await? {
        learning_models::TranscriptionPoll::Pending => return Ok(None),
        learning_models::TranscriptionPoll::Completed(cues) => {
            learning::transcript_from_cues(&media_id, "Paraformer 字幕".into(), cues)
        }
        learning_models::TranscriptionPoll::Failed(error) => Err(error),
    };
    lock(&state.database)?.finish_transcription(&id, &result)?;
    result.map(Some)
}

#[tauri::command]
pub fn get_learning_settings(state: State<AppState>) -> Result<LearningSettings, String> {
    lock(&state.database)?.learning_settings()
}
#[tauri::command]
pub fn update_learning_settings(
    value: LearningSettings,
    state: State<AppState>,
    app: AppHandle,
) -> Result<LearningSettings, String> {
    let value = lock(&state.database)?.save_learning_settings(value)?;
    app.emit("learning-settings-changed", &value)
        .map_err(|e| format!("设置已保存，窗口通知失败: {e}"))?;
    Ok(value)
}
#[tauri::command]
pub async fn learning_secret_status() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(learning_models::secret_status)
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn save_learning_secret(kind: String, value: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || learning_models::save_secret(&kind, &value))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn list_learning_transcripts(
    media_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Transcript>, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let database = lock(&state.database)?;
        database.media(&media_id)?;
        database.transcripts(&media_id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn import_learning_subtitle(
    media_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<Transcript, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        lock(&state.database)?.media(&media_id)?;
        let transcript = learning::parse_transcript(&media_id, Path::new(&path))?;
        let database = lock(&state.database)?;
        database.put_transcript(&transcript)?;
        database.transcript(&transcript.id)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn set_learning_favorite(
    transcript_id: String,
    cue_id: usize,
    favorite: bool,
    state: State<AppState>,
) -> Result<(), String> {
    lock(&state.database)?.favorite_cue(&transcript_id, cue_id, favorite)
}
#[tauri::command]
pub fn list_learning_recordings(
    transcript_id: String,
    state: State<AppState>,
) -> Result<Vec<Recording>, String> {
    lock(&state.database)?.recordings(&transcript_id)
}
#[tauri::command]
pub async fn save_learning_recording(
    transcript_id: String,
    cue_id: usize,
    bytes: Vec<u8>,
    state: State<'_, AppState>,
) -> Result<Recording, String> {
    if bytes.is_empty() || bytes.len() > 12 * 1024 * 1024 {
        return Err("录音为空或超过 12 MB".into());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let directory = {
            let database = lock(&state.database)?;
            database
                .transcript(&transcript_id)?
                .cues
                .get(cue_id)
                .ok_or("字幕句子不存在")?;
            database.learning_settings()?.recording_directory
        };
        if directory.is_empty() {
            return Err("请先在设置 → 录音中选择保存目录".into());
        }
        let directory = PathBuf::from(directory);
        let output = learning::recording_wav(&directory, &bytes)?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = directory.join(format!("{id}.wav"));
        let item = Recording {
            id,
            transcript_id,
            cue_id,
            path: path.to_str().ok_or("录音路径不是 UTF-8")?.into(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs(),
            evaluation: None,
        };
        output.persist_noclobber(&path).map_err(|e| e.to_string())?;
        let saved = lock(&state.database).and_then(|db| db.insert_recording(&item));
        if let Err(error) = saved {
            std::fs::remove_file(&path)
                .map_err(|cleanup| format!("{error}; 录音文件清理失败: {cleanup}"))?;
            return Err(error);
        }
        Ok(item)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn delete_learning_recording(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || lock(&state.database)?.delete_recording(&id))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn explain_learning_cue(
    transcript_id: String,
    cue_id: usize,
    translate: bool,
    state: State<'_, AppState>,
) -> Result<LearningTextResult, String> {
    let (settings, text) = {
        let db = lock(&state.database)?;
        (
            db.learning_settings()?,
            db.transcript(&transcript_id)?
                .cues
                .get(cue_id)
                .ok_or("字幕句子不存在")?
                .text
                .clone(),
        )
    };
    let result = learning_models::explain(&settings, &text, translate).await?;
    if translate {
        lock(&state.database)?.translate_cue(
            &transcript_id,
            cue_id,
            &result,
            &settings.translation_language,
        )?;
    }
    Ok(LearningTextResult {
        text: result,
        language: settings.translation_language,
    })
}
#[tauri::command]
pub async fn evaluate_learning_recording(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let (settings, item, text) = {
        let db = lock(&state.database)?;
        let item = db.recording(&id)?;
        let text = db
            .transcript(&item.transcript_id)?
            .cues
            .get(item.cue_id)
            .ok_or("字幕句子不存在")?
            .text
            .clone();
        (db.learning_settings()?, item, text)
    };
    let path = item.path;
    let pcm = tauri::async_runtime::spawn_blocking(move || {
        learning_models::recording_pcm(Path::new(&path))
    })
    .await
    .map_err(|e| e.to_string())??;
    let result = learning_models::evaluate(&settings, &text, &pcm).await?;
    lock(&state.database)?.save_evaluation(&id, &result)?;
    Ok(result)
}
