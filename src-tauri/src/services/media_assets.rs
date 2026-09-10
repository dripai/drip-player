use crate::models::media::{
    canonical_local_identity, AssetKind, AssetSource, Media, MediaAsset, MediaOrigin,
};
use crate::services::media_capabilities;
use std::path::{Path, PathBuf};

pub fn local_media(path: &Path) -> Result<Media, String> {
    let (path, canonical_key) = canonical_local_identity(path)?;
    if !path.is_file() || !media_capabilities::is_supported_media_path(&path) {
        return Err(format!("Unsupported media file: {}", path.display()));
    }
    let mut media = Media {
        id: uuid::Uuid::new_v4().to_string(),
        canonical_key,
        title: path
            .file_stem()
            .ok_or("Media has no filename")?
            .to_string_lossy()
            .into_owned(),
        media_type: media_capabilities::media_type_from_path(&path),
        origin: MediaOrigin::Local { path },
        assets: Vec::new(),
    };
    media.assets = local_subtitles(&media)?;
    Ok(media)
}

// File naming is used only when importing sidecars. Runtime lookup uses media_id.
pub fn local_subtitles(media: &Media) -> Result<Vec<MediaAsset>, String> {
    let MediaOrigin::Local { path } = &media.origin else {
        return Ok(Vec::new());
    };
    subtitle_assets(
        &media.id,
        path.parent().ok_or("Media has no parent directory")?,
        path.file_stem()
            .and_then(|value| value.to_str())
            .ok_or("Media filename is not UTF-8")?,
        AssetSource::Local,
    )
}

pub fn subtitle_assets(
    media_id: &str,
    directory: &Path,
    stem: &str,
    source: AssetSource,
) -> Result<Vec<MediaAsset>, String> {
    let prefix = format!("{stem}.");
    let mut assets = Vec::new();
    for entry in std::fs::read_dir(directory)
        .map_err(|error| format!("Cannot read {}: {error}", directory.display()))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !path.is_file() || !matches!(ext.as_str(), "srt" | "vtt" | "ass" | "ssa") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("Subtitle filename is not UTF-8")?;
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        let language = suffix
            .rsplit_once('.')
            .map(|(language, _)| language.to_string())
            .unwrap_or_else(|| "und".into());
        assets.push(MediaAsset {
            id: uuid::Uuid::new_v4().to_string(),
            media_id: media_id.into(),
            kind: AssetKind::Subtitle,
            path,
            language: Some(language),
            source: source.clone(),
        });
    }
    Ok(assets)
}

pub fn cache_root() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    Ok(executable
        .parent()
        .ok_or("Executable has no parent directory")?
        .join("cache"))
}
