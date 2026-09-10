use crate::models::download::{DownloadJob, DownloadPhase};
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

    pub fn start_download_attempt(&self, id: &str) -> Result<DownloadJob, String> {
        let mut job = self.download_job(id)?;
        if job.phase.is_active()
            || matches!(
                job.phase,
                DownloadPhase::Completed | DownloadPhase::RecoveryRequired
            )
        {
            return Err("当前任务状态不能重新开始下载".into());
        }
        job.attempt = job
            .attempt
            .checked_add(1)
            .ok_or("Download attempt overflow")?;
        job.phase = DownloadPhase::Resolving;
        job.directory = Some(self.directory()?.path);
        job.output_path = None;
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
    ) -> Result<Media, String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        let mut job = active_job(&tx, id, attempt)?;
        let media_id = put_media(&tx, media)?;
        let other_running: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM download_jobs WHERE id != ?1 AND json_extract(data, '$.media_id') = ?2 AND json_extract(data, '$.phase') IN ('resolving', 'downloading', 'publishing'))",
            params![id, media_id], |row| row.get(0),
        ).map_err(database_error)?;
        if other_running {
            return Err("该媒体已有正在进行的下载任务".into());
        }
        job.media_id = Some(media_id.clone());
        job.title = media.title.clone();
        job.phase = DownloadPhase::Downloading;
        write_job(&tx, &job)?;
        let media = read_media(&tx, &media_id)?;
        tx.commit().map_err(database_error)?;
        Ok(media)
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
