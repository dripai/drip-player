use std::path::PathBuf;
use std::time::Duration;
use serde::{Serialize, Deserialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum MediaType {
    Audio,
    Video,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PlaylistItem {
    Track(Track),
    Folder {
        name: String,
        path: PathBuf,
        children: Vec<PlaylistItem>,
    },
}

fn default_media_type() -> MediaType {
    MediaType::Audio
}

fn default_false() -> bool {
    false
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TrackSource {
    Local(PathBuf),
    Remote {
        url: String,
        id: String,
        cached_path: Option<PathBuf>,
        title: String,
        #[allow(dead_code)]
        duration: Option<Duration>,
        #[serde(default = "default_media_type")]
        media_type: MediaType,
        #[serde(default = "default_false")]
        is_downloading: bool,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub source: TrackSource,
}

impl Track {
    pub fn new_local(path: PathBuf) -> Self {
        Self {
            source: TrackSource::Local(path),
        }
    }
    
    pub fn new_remote(url: String, id: String, title: String, duration: Option<Duration>, media_type: MediaType) -> Self {
        Self {
            source: TrackSource::Remote {
                url,
                id,
                cached_path: None,
                title,
                duration,
                media_type,
                is_downloading: false,
            },
        }
    }

    pub fn name(&self) -> String {
        match &self.source {
            TrackSource::Local(path) => path.file_stem().unwrap_or_default().to_string_lossy().to_string(),
            TrackSource::Remote { title, .. } => title.clone(),
        }
    }
}

pub struct Playlist {
    pub tracks: Vec<Track>,
    pub current_index: Option<usize>,
}

impl Playlist {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            current_index: None,
        }
    }

    pub fn add_local_tracks(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() { return; }

        // Don't auto-set current_index when adding tracks
        // let was_empty = self.tracks.is_empty();
        for path in paths {
            self.tracks.push(Track::new_local(path));
        }

        // if was_empty {
        //     self.current_index = Some(0);
        // }
    }

    pub fn add_track(&mut self, track: Track) {
        // Don't auto-set current_index when adding tracks
        // let was_empty = self.tracks.is_empty();
        self.tracks.push(track);
        // if was_empty {
        //     self.current_index = Some(0);
        // }
    }
    
    pub fn current_track(&self) -> Option<&Track> {
        self.current_index.and_then(|i| self.tracks.get(i))
    }
    
    #[allow(dead_code)]
    pub fn current_track_mut(&mut self) -> Option<&mut Track> {
        self.current_index.and_then(|i| self.tracks.get_mut(i))
    }
    
    #[allow(dead_code)]
    pub fn current_track_name(&self) -> String {
        self.current_track()
            .map(|t| t.name())
            .unwrap_or_else(|| "No Track Selected".to_string())
    }

    pub fn next(&mut self) -> Option<&Track> {
        if self.tracks.is_empty() { return None; }
        
        let next_idx = match self.current_index {
            Some(i) => (i + 1) % self.tracks.len(),
            None => 0,
        };
        
        self.current_index = Some(next_idx);
        self.current_track()
    }
    
    pub fn prev(&mut self) -> Option<&Track> {
        if self.tracks.is_empty() { return None; }
        
        let prev_idx = match self.current_index {
            Some(i) => {
                if i == 0 { self.tracks.len() - 1 } else { i - 1 }
            },
            None => 0,
        };
        
        self.current_index = Some(prev_idx);
        self.current_track()
    }
}
