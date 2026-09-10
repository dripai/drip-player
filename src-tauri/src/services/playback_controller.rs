use crate::models::media::Media;
use crate::models::playback::{
    BrowserPlaybackReport, PlaybackPlan, PlaybackSession, PlaybackSnapshot, PlaybackStatus,
};
use crate::services::audio_wrapper::AudioWrapper;
use crate::services::{online_resolver::OnlineResolver, toolchain};
use std::process::Child;

pub struct PlaybackController {
    revision: u64,
    generation: u64,
    pub session: Option<PlaybackSession>,
    audio: AudioWrapper,
    external: Option<Child>,
}
impl PlaybackController {
    pub fn new() -> Self {
        Self {
            revision: 0,
            generation: 0,
            session: None,
            audio: AudioWrapper::new(),
            external: None,
        }
    }
    pub fn is_requested(&self, id: u64) -> bool {
        self.generation == id
    }
    pub fn matches(&self, id: u64) -> bool {
        self.is_requested(id)
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.id == id)
    }
    pub fn reserve(&mut self) -> Result<u64, String> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Playback session overflow")?;
        self.revision += 1;
        Ok(self.generation)
    }
    fn stop_engine(&mut self) -> Result<(), String> {
        self.audio.stop()?;
        if let Some(child) = self.external.as_mut() {
            if child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_none()
            {
                child.kill().map_err(|error| error.to_string())?;
            }
            child.wait().map_err(|error| error.to_string())?;
        }
        self.external = None;
        Ok(())
    }
    pub fn begin(&mut self, media: Media, entry_id: Option<String>) -> Result<u64, String> {
        let id = self.reserve()?;
        self.begin_reserved(id, media, entry_id)?;
        Ok(id)
    }
    pub fn begin_reserved(
        &mut self,
        id: u64,
        media: Media,
        entry_id: Option<String>,
    ) -> Result<bool, String> {
        if !self.is_requested(id) {
            return Ok(false);
        }
        self.stop_engine()?;
        self.session = Some(PlaybackSession {
            id,
            media,
            playlist_entry_id: entry_id,
            plan: None,
            status: PlaybackStatus::Preparing,
            position: 0.0,
            duration: 0.0,
            error: None,
            browser_sequence: 0,
            browser_owner: None,
        });
        self.revision += 1;
        Ok(true)
    }
    pub fn stop(&mut self) -> Result<(), String> {
        self.reserve()?;
        self.stop_engine()?;
        self.session = None;
        self.revision += 1;
        Ok(())
    }
    pub fn activate(&mut self, id: u64, media: Media, plan: PlaybackPlan) -> Result<(), String> {
        if !self.matches(id) {
            return Ok(());
        }
        let status = match &plan {
            PlaybackPlan::Audio { path } => {
                self.audio.load(id, path.clone())?;
                PlaybackStatus::Preparing
            }
            PlaybackPlan::BrowserVideo { .. } => PlaybackStatus::Ready,
            PlaybackPlan::ExternalVideo { path } => {
                let mpv = OnlineResolver::get_mpv_path().ok_or("MPV is unavailable")?;
                self.external = Some(
                    toolchain::hidden_command(&mpv)
                        .arg(path)
                        .args([
                            "--force-window=yes",
                            "--title=Shadow Player",
                            "--osd-level=1",
                        ])
                        .spawn()
                        .map_err(|error| format!("Failed to start MPV: {error}"))?,
                );
                PlaybackStatus::External
            }
        };
        if let Some(session) = self.session.as_mut() {
            session.media = media;
            session.plan = Some(plan);
            session.status = status;
        }
        self.revision += 1;
        Ok(())
    }
    pub fn fail(&mut self, id: u64, message: String) {
        if self.matches(id) {
            let message = match self.stop_engine() {
                Ok(()) => message,
                Err(error) => format!("{message}; failed to stop player: {error}"),
            };
            if let Some(session) = self.session.as_mut() {
                session.status = PlaybackStatus::Failed;
                session.error = Some(message);
            }
            self.revision += 1;
        }
    }
    pub fn report_browser(&mut self, report: BrowserPlaybackReport) -> Result<(), String> {
        report.validate()?;
        if let Some(session) = self.session.as_mut() {
            if session.id != report.session_id
                || session.browser_owner.as_deref() != Some(report.owner.as_str())
                || report.sequence <= session.browser_sequence
                || !matches!(session.plan, Some(PlaybackPlan::BrowserVideo { .. }))
                || matches!(
                    session.status,
                    PlaybackStatus::Failed | PlaybackStatus::Stopped
                )
            {
                return Ok(());
            }
            session.browser_sequence = report.sequence;
            session.position = report.position;
            session.duration = report.duration;
            session.status = report.status;
            self.revision += 1;
        }
        Ok(())
    }
    pub fn attach_browser(&mut self, id: u64, owner: String) -> Result<PlaybackSnapshot, String> {
        let session = self
            .session
            .as_mut()
            .filter(|session| session.id == id)
            .ok_or("Playback session changed")?;
        if !matches!(session.plan, Some(PlaybackPlan::BrowserVideo { .. })) {
            return Err("Not a browser playback session".into());
        }
        session.browser_owner = Some(owner);
        session.browser_sequence = 0;
        self.revision += 1;
        Ok(self.snapshot())
    }
    pub fn audio_for_session(&self, id: u64) -> Result<AudioWrapper, String> {
        let session = self
            .session
            .as_ref()
            .filter(|session| session.id == id)
            .ok_or("Playback session changed")?;
        if !matches!(session.plan, Some(PlaybackPlan::Audio { .. })) {
            return Err("This engine does not support audio controls".into());
        }
        Ok(self.audio.clone())
    }
    pub fn snapshot(&mut self) -> PlaybackSnapshot {
        if let Some(session) = self.session.as_mut() {
            if matches!(session.plan, Some(PlaybackPlan::Audio { .. }))
                && !matches!(
                    session.status,
                    PlaybackStatus::Failed | PlaybackStatus::Stopped
                )
            {
                if let Some(audio) = self
                    .audio
                    .snapshot()
                    .filter(|audio| audio.session_id == session.id)
                {
                    if session.status != audio.status
                        || session.position != audio.position
                        || session.duration != audio.duration
                        || session.error != audio.error
                    {
                        session.status = audio.status;
                        session.position = audio.position;
                        session.duration = audio.duration;
                        session.error = audio.error;
                        self.revision += 1;
                    }
                }
            }
            if let Some(child) = self.external.as_mut() {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        session.status = PlaybackStatus::Stopped;
                        self.external = None;
                        self.revision += 1;
                    }
                    Err(error) => {
                        session.status = PlaybackStatus::Failed;
                        session.error = Some(error.to_string());
                        self.external = None;
                        self.revision += 1;
                    }
                    Ok(None) => {}
                }
            }
        }
        PlaybackSnapshot {
            revision: self.revision,
            session: self.session.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::media::{MediaOrigin, MediaType};
    #[test]
    fn browser_can_repeat_after_end_but_stale_owners_and_sequences_are_rejected() {
        let mut controller = PlaybackController::new();
        let media = Media {
            id: "m".into(),
            canonical_key: "test".into(),
            title: "Fixture".into(),
            media_type: MediaType::Video,
            origin: MediaOrigin::Local {
                path: "fixture.mp4".into(),
            },
            assets: vec![],
        };
        let id = controller.begin(media.clone(), None).unwrap();
        controller
            .activate(
                id,
                media,
                PlaybackPlan::BrowserVideo {
                    path: "fixture.mp4".into(),
                },
            )
            .unwrap();
        controller.attach_browser(id, "owner".into()).unwrap();
        let report = |sequence, owner: &str, position, status| BrowserPlaybackReport {
            session_id: id,
            owner: owner.into(),
            sequence,
            position,
            duration: 8.0,
            status,
        };
        controller
            .report_browser(report(1, "owner", 8.0, PlaybackStatus::Ended))
            .unwrap();
        controller
            .report_browser(report(2, "owner", 1.0, PlaybackStatus::Playing))
            .unwrap();
        assert_eq!(
            controller.snapshot().session.unwrap().status,
            PlaybackStatus::Playing
        );
        controller
            .report_browser(report(1, "owner", 8.0, PlaybackStatus::Ended))
            .unwrap();
        controller
            .report_browser(report(3, "old-owner", 8.0, PlaybackStatus::Ended))
            .unwrap();
        assert_eq!(controller.snapshot().session.unwrap().position, 1.0);
        controller.fail(id, "decode error".into());
        controller
            .report_browser(report(4, "owner", 2.0, PlaybackStatus::Playing))
            .unwrap();
        assert_eq!(
            controller.snapshot().session.unwrap().status,
            PlaybackStatus::Failed
        );
    }
}
