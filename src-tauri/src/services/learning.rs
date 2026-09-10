use super::{persistence::PersistenceManager, toolchain};
use crate::models::learning::{Cue, LearningSettings, Recording, Transcript};
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

pub fn recording_wav(directory: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile, String> {
    if bytes.is_empty() || bytes.len() > 12 * 1024 * 1024 {
        return Err("录音为空或超过 12 MB".into());
    }
    let input =
        tempfile::NamedTempFile::new_in(directory).map_err(|e| format!("录音目录不可写: {e}"))?;
    input
        .as_file()
        .write_all(bytes)
        .map_err(|e| e.to_string())?;
    let output = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    let result = toolchain::hidden_command(&toolchain::tool_path("ffmpeg"))
        .args(["-v", "error", "-y", "-i"])
        .arg(input.path())
        .args([
            "-t",
            "180",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            "-f",
            "wav",
        ])
        .arg(output.path())
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "录音转换失败: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    if output
        .as_file()
        .metadata()
        .map_err(|e| e.to_string())?
        .len()
        <= 44
    {
        return Err("录音转换后没有音频数据".into());
    }
    output.as_file().sync_all().map_err(|e| e.to_string())?;
    Ok(output)
}

fn db(error: rusqlite::Error) -> String {
    format!("SQLite: {error}")
}

impl PersistenceManager {
    pub fn save_transcription_job(
        &self,
        id: &str,
        media_id: &str,
        settings: &LearningSettings,
    ) -> Result<(), String> {
        self.connection.execute("INSERT INTO learning_transcription_jobs (id, media_id, settings, status) VALUES (?1, ?2, ?3, 'pending')",
            params![id, media_id, serde_json::to_string(settings).map_err(|e| e.to_string())?]).map_err(db)?;
        Ok(())
    }
    pub fn transcription_job(
        &self,
        id: &str,
    ) -> Result<(String, LearningSettings, String), String> {
        let (media, data, status): (String, String, String) = self
            .connection
            .query_row(
                "SELECT media_id, settings, status FROM learning_transcription_jobs WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(db)?;
        Ok((
            media,
            serde_json::from_str(&data).map_err(|e| e.to_string())?,
            status,
        ))
    }
    pub fn pending_transcriptions(&self, media_id: &str) -> Result<Vec<String>, String> {
        let mut query = self.connection.prepare("SELECT id FROM learning_transcription_jobs WHERE media_id = ?1 AND status = 'pending' ORDER BY rowid").map_err(db)?;
        let rows = query.query_map([media_id], |r| r.get(0)).map_err(db)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db)
    }
    pub fn finish_transcription(
        &mut self,
        id: &str,
        result: &Result<Transcript, String>,
    ) -> Result<(), String> {
        let tx = self.connection.transaction().map_err(db)?;
        match result {
            Ok(t) => {
                tx.execute("INSERT INTO transcripts (id, media_id, label, data) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(id) DO NOTHING", params![t.id, t.media_id, t.label, serde_json::to_string(&t.cues).map_err(|e| e.to_string())?]).map_err(db)?;
                tx.execute("UPDATE learning_transcription_jobs SET status = 'completed', transcript_id = ?1 WHERE id = ?2", params![t.id, id]).map_err(db)?;
            }
            Err(error) => {
                tx.execute("UPDATE learning_transcription_jobs SET status = 'failed', error = ?1 WHERE id = ?2", params![error, id]).map_err(db)?;
            }
        }
        tx.commit().map_err(db)
    }
    pub fn learning_settings(&self) -> Result<LearningSettings, String> {
        let data: String = self
            .connection
            .query_row(
                "SELECT data FROM learning_settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(db)?;
        serde_json::from_str(&data).map_err(|e| format!("学习设置损坏: {e}"))
    }
    pub fn save_learning_settings(
        &mut self,
        mut value: LearningSettings,
    ) -> Result<LearningSettings, String> {
        value.validate()?;
        let expected = value.revision;
        value.revision = expected.checked_add(1).ok_or("设置版本超出范围")?;
        let count = self.connection.execute("UPDATE learning_settings SET revision = ?1, data = ?2 WHERE id = 1 AND revision = ?3",
            params![value.revision, serde_json::to_string(&value).map_err(|e| e.to_string())?, expected]).map_err(db)?;
        if count != 1 {
            return Err("设置已在其他窗口修改，请重新读取后保存".into());
        }
        Ok(value)
    }
    pub fn put_transcript(&self, transcript: &Transcript) -> Result<(), String> {
        self.connection.execute("INSERT INTO transcripts (id, media_id, label, data) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(id) DO NOTHING",
            params![transcript.id, transcript.media_id, transcript.label, serde_json::to_string(&transcript.cues).map_err(|e| e.to_string())?]).map_err(db)?;
        Ok(())
    }
    pub fn transcripts(&self, media_id: &str) -> Result<Vec<Transcript>, String> {
        let mut stmt = self
            .connection
            .prepare("SELECT id FROM transcripts WHERE media_id = ?1 ORDER BY rowid DESC")
            .map_err(db)?;
        let ids = stmt
            .query_map([media_id], |row| row.get::<_, String>(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        ids.iter().map(|id| self.transcript(id)).collect()
    }
    pub fn transcript(&self, id: &str) -> Result<Transcript, String> {
        let (media_id, label, data): (String, String, String) = self
            .connection
            .query_row(
                "SELECT media_id, label, data FROM transcripts WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(db)?;
        let mut cues: Vec<Cue> = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        let mut stmt = self.connection.prepare("SELECT cue_id, translation, favorite, translation_language FROM learning_cue_notes WHERE transcript_id = ?1").map_err(db)?;
        let notes = stmt
            .query_map([id], |row| {
                Ok((
                    row.get::<_, usize>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(db)?;
        for note in notes {
            let (index, translation, favorite, language) = note.map_err(db)?;
            let cue = cues.get_mut(index).ok_or("字幕注释索引损坏")?;
            cue.translation = translation;
            cue.favorite = favorite;
            cue.translation_language = language;
        }
        Ok(Transcript {
            id: id.into(),
            media_id,
            label,
            cues,
        })
    }
    pub fn favorite_cue(&self, id: &str, cue_id: usize, favorite: bool) -> Result<(), String> {
        self.transcript(id)?
            .cues
            .get(cue_id)
            .ok_or("字幕句子不存在")?;
        self.connection.execute("INSERT INTO learning_cue_notes (transcript_id, cue_id, favorite) VALUES (?1, ?2, ?3) ON CONFLICT(transcript_id, cue_id) DO UPDATE SET favorite = excluded.favorite",
            params![id, cue_id, favorite]).map_err(db)?;
        Ok(())
    }
    pub fn translate_cue(
        &self,
        id: &str,
        cue_id: usize,
        text: &str,
        language: &str,
    ) -> Result<(), String> {
        self.transcript(id)?
            .cues
            .get(cue_id)
            .ok_or("字幕句子不存在")?;
        self.connection.execute("INSERT INTO learning_cue_notes (transcript_id, cue_id, translation, translation_language) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(transcript_id, cue_id) DO UPDATE SET translation = excluded.translation, translation_language = excluded.translation_language",
            params![id, cue_id, text, language]).map_err(db)?;
        Ok(())
    }
    pub fn recordings(&self, transcript_id: &str) -> Result<Vec<Recording>, String> {
        let mut stmt = self.connection.prepare("SELECT id, cue_id, path, created_at, evaluation FROM learning_recordings WHERE transcript_id = ?1 ORDER BY created_at DESC, rowid DESC").map_err(db)?;
        let rows = stmt
            .query_map([transcript_id], |row| {
                Ok(Recording {
                    id: row.get(0)?,
                    transcript_id: transcript_id.into(),
                    cue_id: row.get(1)?,
                    path: row.get(2)?,
                    created_at: row.get(3)?,
                    evaluation: row.get(4)?,
                })
            })
            .map_err(db)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db)
    }
    pub fn recording(&self, id: &str) -> Result<Recording, String> {
        let transcript_id: String = self
            .connection
            .query_row(
                "SELECT transcript_id FROM learning_recordings WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db)?
            .ok_or("录音不存在")?;
        self.recordings(&transcript_id)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or("录音不存在".into())
    }
    pub fn insert_recording(&self, item: &Recording) -> Result<(), String> {
        self.transcript(&item.transcript_id)?
            .cues
            .get(item.cue_id)
            .ok_or("字幕句子不存在")?;
        self.connection.execute("INSERT INTO learning_recordings (id, transcript_id, cue_id, path, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![item.id, item.transcript_id, item.cue_id, item.path, item.created_at]).map_err(db)?;
        Ok(())
    }
    pub fn save_evaluation(&self, id: &str, evaluation: &str) -> Result<(), String> {
        if self
            .connection
            .execute(
                "UPDATE learning_recordings SET evaluation = ?1 WHERE id = ?2",
                params![evaluation, id],
            )
            .map_err(db)?
            != 1
        {
            return Err("录音已被移除，评测结果未保存".into());
        }
        Ok(())
    }
    pub fn delete_recording(&mut self, id: &str) -> Result<(), String> {
        let item = self.recording(id)?;
        let original = Path::new(&item.path);
        let staged = original.with_extension(format!("{}.deleted", uuid::Uuid::new_v4()));
        let tx = self.connection.transaction().map_err(db)?;
        tx.execute("DELETE FROM learning_recordings WHERE id = ?1", [id])
            .map_err(db)?;
        std::fs::rename(original, &staged).map_err(|e| format!("无法移除录音: {e}"))?;
        if let Err(error) = tx.commit() {
            return match std::fs::rename(&staged, original) {
                Ok(()) => Err(db(error)),
                Err(rollback) => Err(format!(
                    "数据库提交失败: {error}; 录音恢复失败: {rollback}; 文件在 {}",
                    staged.display()
                )),
            };
        }
        std::fs::remove_file(&staged)
            .map_err(|e| format!("录音记录已移除，但文件清理失败 {}: {e}", staged.display()))
    }
}

pub fn transcript_from_cues(
    media_id: &str,
    label: String,
    mut cues: Vec<Cue>,
) -> Result<Transcript, String> {
    if cues.is_empty() || cues.len() > 50000 {
        return Err("字幕为空或句子过多".into());
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    for (id, cue) in cues.iter_mut().enumerate() {
        if !cue.start.is_finite()
            || !cue.end.is_finite()
            || cue.start < 0.0
            || cue.end <= cue.start
            || cue.text.trim().is_empty()
        {
            return Err(format!("第 {} 句字幕的时间或文本无效", id + 1));
        }
        cue.id = id;
    }
    let hash = Sha256::digest(format!(
        "{media_id}:{}",
        serde_json::to_string(&cues).map_err(|e| e.to_string())?
    ));
    Ok(Transcript {
        id: format!("{hash:x}"),
        media_id: media_id.into(),
        label,
        cues,
    })
}

pub fn parse_transcript(media_id: &str, path: &Path) -> Result<Transcript, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len() > 10 * 1024 * 1024 {
        return Err("字幕文件不能超过 10 MB".into());
    }
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "srt" | "vtt" | "ass" | "ssa") {
        return Err("请选择 SRT、VTT、ASS 或 SSA 字幕".into());
    }
    std::fs::read_to_string(path).map_err(|e| format!("请使用 UTF-8 字幕: {e}"))?;
    // FFmpeg's native text subtitle encoder removes presentation tags and inline
    // karaoke timestamps. The learning transcript stores spoken text, never HTML.
    let output = toolchain::hidden_command(&toolchain::tool_path("ffmpeg"))
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-c:s", "text", "-f", "srt", "-"])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "字幕转换失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    parse_subtitle_text(
        media_id,
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        &text,
        "srt",
    )
}
pub fn parse_subtitle_text(
    media_id: &str,
    label: String,
    text: &str,
    extension: &str,
) -> Result<Transcript, String> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let make = |start, end, text| Cue {
        id: 0,
        start,
        end,
        text,
        translation: None,
        translation_language: None,
        favorite: false,
    };
    let cues = if extension == "srt" {
        subtp::srt::SubRip::parse(&text)
            .map_err(|e| format!("SRT 解析失败: {e}"))?
            .subtitles
            .into_iter()
            .map(|cue| {
                let seconds = |t: subtp::srt::SrtTimestamp| {
                    f64::from(t.hours) * 3600.0
                        + f64::from(t.minutes) * 60.0
                        + f64::from(t.seconds)
                        + f64::from(t.milliseconds) / 1000.0
                };
                make(seconds(cue.start), seconds(cue.end), cue.text.join("\n"))
            })
            .collect()
    } else {
        subtp::vtt::WebVtt::parse(&text)
            .map_err(|e| format!("VTT 解析失败: {e}"))?
            .blocks
            .into_iter()
            .filter_map(|block| {
                if let subtp::vtt::VttBlock::Que(cue) = block {
                    let seconds = |t: subtp::vtt::VttTimestamp| {
                        f64::from(t.hours) * 3600.0
                            + f64::from(t.minutes) * 60.0
                            + f64::from(t.seconds)
                            + f64::from(t.milliseconds) / 1000.0
                    };
                    Some(make(
                        seconds(cue.timings.start),
                        seconds(cue.timings.end),
                        cue.payload.join("\n"),
                    ))
                } else {
                    None
                }
            })
            .collect()
    };
    transcript_from_cues(media_id, label, cues)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires bundled FFmpeg"]
    fn imports_styled_subtitles_as_spoken_text_with_original_timestamps() {
        toolchain::set_resource_dir(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_path_buf(),
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("styled.vtt");
        std::fs::write(&path, "WEBVTT\n\n00:01.000 --> 00:02.000\n<v Alice><c.green>Hello</c> <00:01.500><b>world</b> &amp; friends.\n").unwrap();
        let transcript = parse_transcript("m", &path).unwrap();
        assert_eq!(transcript.cues[0].text, "Hello world & friends.");
        assert_eq!(transcript.cues[0].start, 1.0);
        assert_eq!(transcript.cues[0].end, 2.0);
        let path = directory.path().join("中文.srt");
        std::fs::write(
            &path,
            "\u{feff}1\r\n00:00:01,250 --> 00:00:02,500\r\n<i>Hello</i>\r\n你好\r\n",
        )
        .unwrap();
        let transcript = parse_transcript("m", &path).unwrap();
        assert_eq!(transcript.cues[0].text, "Hello\n你好");
        assert_eq!(transcript.cues[0].start, 1.25);
    }
    #[test]
    #[ignore = "requires bundled FFmpeg and FFprobe; no microphone or output device is used"]
    fn converts_recording_to_mono_wav_and_cleans_up_failed_conversion() {
        toolchain::set_resource_dir(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_path_buf(),
        );
        let dir = tempfile::tempdir().unwrap();
        let generated = toolchain::hidden_command(&toolchain::tool_path("ffmpeg"))
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.4",
                "-c:a",
                "libopus",
                "-f",
                "webm",
                "-",
            ])
            .output()
            .unwrap();
        assert!(generated.status.success());
        let output = recording_wav(dir.path(), &generated.stdout).unwrap();
        let probe = toolchain::hidden_command(&toolchain::tool_path("ffprobe"))
            .args(["-v", "error", "-show_streams", "-of", "json"])
            .arg(output.path())
            .output()
            .unwrap();
        assert!(probe.status.success());
        let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert_eq!(metadata["streams"][0]["codec_name"], "pcm_s16le");
        assert_eq!(metadata["streams"][0]["channels"], 1);
        assert_eq!(metadata["streams"][0]["sample_rate"], "16000");
        let pcm = super::super::learning_models::recording_pcm(output.path()).unwrap();
        assert!(pcm.len() >= 12000 && pcm.len() <= 14000);
        drop(output);
        assert!(recording_wav(dir.path(), b"invalid audio").is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[test]
    fn parses_multiline_srt_and_identifies_content_versions() {
        let text = "\u{feff}1\r\n00:00:01,250 --> 00:00:02,500\r\nHello.\r\n你好。\r\n";
        let a = parse_subtitle_text("media", "one".into(), text, "srt").unwrap();
        let b = parse_subtitle_text("media", "two".into(), text, "srt").unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.cues[0].start, 1.25);
        assert_eq!(a.cues[0].text, "Hello.\n你好。");
        assert_ne!(
            a.id,
            parse_subtitle_text("other", "one".into(), text, "srt")
                .unwrap()
                .id
        );
    }
    #[test]
    fn validates_vtt_timing_and_learning_ranges() {
        let t = parse_subtitle_text(
            "m",
            "vtt".into(),
            "WEBVTT\n\n00:01.000 --> 00:02.000\nHello\n",
            "vtt",
        )
        .unwrap();
        assert_eq!(t.cues[0].end, 2.0);
        assert!(parse_subtitle_text(
            "m",
            "bad".into(),
            "1\n00:00:02,000 --> 00:00:01,000\nNo\n",
            "srt"
        )
        .is_err());
        let settings = LearningSettings {
            repetitions: 0,
            ..Default::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn persists_settings_versions_cue_notes_and_recordings_without_resetting_media() {
        use crate::models::media::{Media, MediaOrigin, MediaType};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("learning.sqlite3");
        let mut database = PersistenceManager::open_path(&path).unwrap();
        let media = database
            .register_media(&Media {
                id: "m".into(),
                canonical_key: "local:fixture".into(),
                title: "Fixture".into(),
                media_type: MediaType::Video,
                origin: MediaOrigin::Local {
                    path: directory.path().join("fixture.mp4"),
                },
                assets: vec![],
            })
            .unwrap();
        let old = database.learning_settings().unwrap();
        let mut next = old.clone();
        next.repetitions = 4;
        database.save_learning_settings(next).unwrap();
        assert!(database.save_learning_settings(old).is_err());
        let a = parse_subtitle_text(
            &media.id,
            "first".into(),
            "1\n00:00:01,000 --> 00:00:02,000\nHello\n",
            "srt",
        )
        .unwrap();
        let b = parse_subtitle_text(
            &media.id,
            "corrected".into(),
            "1\n00:00:01,000 --> 00:00:03,000\nHello again\n",
            "srt",
        )
        .unwrap();
        database.put_transcript(&a).unwrap();
        database.put_transcript(&b).unwrap();
        database.favorite_cue(&a.id, 0, true).unwrap();
        database.translate_cue(&a.id, 0, "你好", "zh").unwrap();
        assert!(database.favorite_cue(&a.id, 50, true).is_err());
        assert!(!database.transcript(&b.id).unwrap().cues[0].favorite);
        let audio = directory.path().join("take.wav");
        std::fs::write(&audio, b"fixture").unwrap();
        let recording = Recording {
            id: "r".into(),
            transcript_id: a.id.clone(),
            cue_id: 0,
            path: audio.to_str().unwrap().into(),
            created_at: 1,
            evaluation: None,
        };
        database.insert_recording(&recording).unwrap();
        database.save_evaluation("r", "总分：90").unwrap();
        database
            .save_transcription_job("task", &media.id, &database.learning_settings().unwrap())
            .unwrap();
        database
            .finish_transcription("task", &Err("invalid URL".into()))
            .unwrap();
        assert!(database
            .pending_transcriptions(&media.id)
            .unwrap()
            .is_empty());
        drop(database);
        let mut database = PersistenceManager::open_path(&path).unwrap();
        assert_eq!(database.media(&media.id).unwrap().title, "Fixture");
        assert_eq!(database.learning_settings().unwrap().repetitions, 4);
        let saved = database.transcript(&a.id).unwrap();
        assert!(saved.cues[0].favorite);
        assert_eq!(saved.cues[0].translation.as_deref(), Some("你好"));
        assert_eq!(saved.cues[0].translation_language.as_deref(), Some("zh"));
        assert_eq!(
            database.recording("r").unwrap().evaluation.as_deref(),
            Some("总分：90")
        );
        std::fs::remove_file(&audio).unwrap();
        assert!(database.delete_recording("r").is_err()); // Failed filesystem step rolls the row back.
        assert!(database.recording("r").is_ok());
        std::fs::write(&audio, b"fixture").unwrap();
        database.delete_recording("r").unwrap();
        assert!(!audio.exists());
        assert!(database.recordings(&a.id).unwrap().is_empty());
    }
}
