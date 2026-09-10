use crate::models::download::{DownloadAction, DownloadJob, DownloadOption, DownloadPhase};
use crate::models::media::Media;
use crate::services::persistence::{database_error, put_media, read_media, PersistenceManager};
use rusqlite::{params, Connection, TransactionBehavior};

pub(super) fn read_job(connection: &Connection, id: &str) -> Result<DownloadJob, String> {
    let data: String = connection
        .query_row(
            "SELECT data FROM download_jobs WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    serde_json::from_str(&data).map_err(|error| format!("Invalid download task {id}: {error}"))
}

pub(super) fn write_job(connection: &Connection, job: &DownloadJob) -> Result<(), String> {
    let data = serde_json::to_string(job).map_err(|error| error.to_string())?;
    connection.execute("INSERT INTO download_jobs (id, data) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET data = excluded.data", params![job.id, data]).map_err(database_error)?;
    Ok(())
}

pub(super) fn active_job(
    connection: &Connection,
    id: &str,
    attempt: u64,
) -> Result<DownloadJob, String> {
    let job = read_job(connection, id)?;
    if job.attempt != attempt || !job.phase.is_active() {
        return Err("下载任务已停止或已开始新的尝试".into());
    }
    Ok(job)
}

impl PersistenceManager {
    pub fn download_jobs(&self) -> Result<Vec<DownloadJob>, String> {
        self.connection.prepare("SELECT data FROM download_jobs ORDER BY json_extract(data, '$.created_at') DESC, rowid DESC")
            .map_err(database_error)?
            .query_map([], |row| row.get::<_, String>(0)).map_err(database_error)?
            .map(|row| serde_json::from_str(&row.map_err(database_error)?).map_err(|error| format!("Invalid download task: {error}")))
            .collect()
    }

    pub fn download_job(&self, id: &str) -> Result<DownloadJob, String> {
        read_job(&self.connection, id)
    }
    pub fn save_download_job(&self, job: &DownloadJob) -> Result<(), String> {
        write_job(&self.connection, job)
    }

    // Caller selects inactive jobs while holding directory/database/registry locks.
    // History owns no files or media rows; only task records are removed here.
    pub fn remove_download_history(&mut self, ids: &[String]) -> Result<(), String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        for id in ids {
            let removed = tx
                .execute("DELETE FROM download_jobs WHERE id = ?1", [id])
                .map_err(database_error)?;
            if removed != 1 {
                return Err("下载记录已变化，请刷新后重试".into());
            }
        }
        tx.commit().map_err(database_error)
    }

    pub fn start_download_attempt(
        &self,
        id: &str,
        action: &DownloadAction,
    ) -> Result<DownloadJob, String> {
        let mut job = self.download_job(id)?;
        if job.phase.is_active()
            || matches!(
                job.phase,
                DownloadPhase::Completed | DownloadPhase::RecoveryRequired
            )
        {
            return Err("当前任务状态不能重新开始下载".into());
        }
        match action {
            DownloadAction::Resolve(auth) => {
                job.auth = *auth;
                job.selection = None;
                job.options.clear();
                job.expected_duration = None;
            }
            DownloadAction::Select { option_id, attempt } => {
                if job.phase != DownloadPhase::AwaitingSelection || job.attempt != *attempt {
                    return Err("下载选项已更新，请重新选择".into());
                }
                let option = job
                    .options
                    .iter()
                    .find(|option| option.id == *option_id)
                    .ok_or("请选择当前可用的下载格式")?;
                if option.limited_duration.is_some() {
                    return Err("当前格式只提供部分内容，请更换登录来源后重新解析".into());
                }
                job.selection = Some(option_id.clone());
            }
            DownloadAction::Retry => {}
        }
        job.attempt = job
            .attempt
            .checked_add(1)
            .ok_or("Download attempt overflow")?;
        job.phase = DownloadPhase::Resolving;
        job.directory = Some(self.directory()?.path);
        job.output_path = None;
        job.publication.clear();
        job.error = None;
        job.interrupt_reason = None;
        self.save_download_job(&job)?;
        Ok(job)
    }

    pub fn resolve_download_media(
        &mut self,
        id: &str,
        attempt: u64,
        media: &Media,
        options: Vec<DownloadOption>,
        expected_duration: Option<f64>,
    ) -> Result<Media, String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let mut job = active_job(&tx, id, attempt)?;
        let media_id = put_media(&tx, media)?;
        let other_running: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM download_jobs WHERE id != ?1 AND json_extract(data, '$.media_id') = ?2 AND json_extract(data, '$.phase') IN ('resolving', 'downloading', 'verifying', 'publishing'))",
            params![id, media_id], |row| row.get(0),
        ).map_err(database_error)?;
        if other_running {
            return Err("该媒体已有正在进行的下载任务".into());
        }
        job.media_id = Some(media_id.clone());
        job.title = media.title.clone();
        job.options = options;
        job.expected_duration = expected_duration;
        job.phase = DownloadPhase::AwaitingSelection;
        if let Some(selection) = &job.selection {
            if job
                .options
                .iter()
                .any(|option| &option.id == selection && option.limited_duration.is_none())
            {
                job.phase = DownloadPhase::Downloading;
            } else {
                job.selection = None;
                job.error = Some(
                    "之前选择的格式已不可用，请重新选择；当前登录来源可能只提供部分内容".into(),
                );
            }
        }
        let mut resolved = read_media(&tx, &media_id)?;
        resolved.title = job.title.clone();
        resolved.media_type = media.media_type.clone();
        write_job(&tx, &job)?;
        tx.commit().map_err(database_error)?;
        Ok(resolved)
    }

    pub fn finish_download_attempt(
        &self,
        id: &str,
        attempt: u64,
        phase: DownloadPhase,
        error: Option<String>,
        reason: Option<String>,
    ) -> Result<bool, String> {
        let mut job = self.download_job(id)?;
        if job.attempt != attempt || !job.phase.is_active() {
            return Ok(false);
        }
        job.phase = phase;
        job.error = error;
        job.interrupt_reason = reason;
        self.save_download_job(&job)?;
        Ok(true)
    }
}
