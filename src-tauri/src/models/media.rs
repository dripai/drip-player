use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum MediaType {
    Audio,
    Video,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaOrigin {
    Local {
        path: PathBuf,
    },
    Remote {
        url: String,
        provider: String,
        external_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Playback,
    Subtitle,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AssetSource {
    Local,
    Download,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MediaAsset {
    pub id: String,
    pub media_id: String,
    pub kind: AssetKind,
    pub path: PathBuf,
    pub language: Option<String>,
    pub source: AssetSource,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Media {
    pub id: String,
    pub canonical_key: String,
    pub title: String,
    pub media_type: MediaType,
    pub origin: MediaOrigin,
    pub assets: Vec<MediaAsset>,
}

impl Media {
    pub fn cached_path(&self) -> Option<&Path> {
        self.assets
            .iter()
            .find(|asset| asset.kind == AssetKind::Playback)
            .map(|asset| asset.path.as_path())
    }

    pub fn local_path(&self) -> Option<&Path> {
        match &self.origin {
            MediaOrigin::Local { path } => Some(path),
            MediaOrigin::Remote { .. } => self.cached_path(),
        }
    }
}

pub fn canonical_local_identity(path: &Path) -> Result<(PathBuf, String), String> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("Cannot resolve {}: {error}", path.display()))?;
    let mut key = path
        .to_str()
        .ok_or("Media path is not UTF-8")?
        .replace('\\', "/");
    if cfg!(windows) {
        key = key.to_lowercase();
    }
    Ok((path, format!("local:{key}")))
}

pub fn canonical_remote_key(provider: &str, external_id: &str) -> String {
    format!("remote:{}:{}", provider.to_lowercase(), external_id)
}

pub fn provider_key_for_url(value: &str) -> Result<String, String> {
    let url = url::Url::parse(value).map_err(|error| error.to_string())?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err("Expected an HTTP(S) media URL".into());
    }
    let host = url
        .host_str()
        .ok_or("Media URL has no host")?
        .to_lowercase();
    for (provider, domains) in [
        ("bilibili", &["bilibili.com", "b23.tv"][..]),
        ("youtube", &["youtube.com", "youtu.be"][..]),
        ("douyin", &["douyin.com", "iesdouyin.com"][..]),
        (
            "weixin",
            &["channels.weixin.qq.com", "finder.video.qq.com"][..],
        ),
        ("tencent", &["v.qq.com"][..]),
    ] {
        if domains
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
        {
            return Ok(provider.into());
        }
    }
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_media_identity_does_not_depend_on_url_shape() {
        let long = provider_key_for_url("https://www.youtube.com/watch?v=ysaGeSbcnJA").unwrap();
        let short = provider_key_for_url("https://youtu.be/ysaGeSbcnJA").unwrap();
        assert_eq!(
            canonical_remote_key(&long, "ysaGeSbcnJA"),
            canonical_remote_key(&short, "ysaGeSbcnJA")
        );
    }
}
