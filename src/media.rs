use crate::util::format_duration_ms;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

/// What kind of media a file is, based on its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Other,
}

pub const IMAGE_EXTS: [&str; 14] = [
    "jpg", "jpeg", "jfif", "png", "gif", "bmp", "webp", "tiff", "tif", "heic", "heif", "avif",
    "svg", "ico",
];
pub const VIDEO_EXTS: [&str; 16] = [
    "mp4", "mkv", "avi", "mov", "webm", "m4v", "flv", "wmv", "mpg", "mpeg", "ts", "m2ts", "3gp",
    "ogv", "rmvb", "vob",
];

pub fn classify(path: &Path) -> MediaKind {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return MediaKind::Other;
    };
    let ext = ext.to_ascii_lowercase();
    if IMAGE_EXTS.contains(&ext.as_str()) {
        MediaKind::Image
    } else if VIDEO_EXTS.contains(&ext.as_str()) {
        MediaKind::Video
    } else {
        MediaKind::Other
    }
}

/// Metadata extracted from a media file via `ffprobe`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub codec: Option<String>,
}

impl MediaInfo {
    pub fn resolution(&self) -> Option<String> {
        match (self.width, self.height) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            _ => None,
        }
    }

    pub fn summary(&self) -> String {
        let res = self
            .resolution()
            .unwrap_or_else(|| "?x?".to_string());
        match self.duration_ms {
            Some(ms) => format!("{res}, {}", format_duration_ms(ms)),
            None => res,
        }
    }
}

#[derive(Deserialize)]
struct FfprobeStream {
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    codec_name: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    #[serde(default)]
    duration: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    #[serde(default)]
    format: Option<FfprobeFormat>,
}

/// True when an `ffprobe` executable is reachable on PATH.
pub fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// True when an `ffmpeg` executable is reachable on PATH (needed for video
/// frame sampling in `--similar` mode).
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Probe width/height/codec (video stream) and duration via `ffprobe`.
/// Returns `None` on any failure (missing binary, parse error, no video stream).
pub fn probe_media(path: &Path) -> Option<MediaInfo> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,codec_name",
            "-show_entries",
            "format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_probe_output(&output.stdout)
}

/// Parse the JSON produced by the `ffprobe` invocation above.
/// Exposed separately so the parsing logic is unit-testable without ffmpeg.
fn parse_probe_output(stdout: &[u8]) -> Option<MediaInfo> {
    let parsed: FfprobeOutput = serde_json::from_slice(stdout).ok()?;
    let stream = parsed.streams.first();
    Some(MediaInfo {
        width: stream.and_then(|s| s.width),
        height: stream.and_then(|s| s.height),
        codec: stream.and_then(|s| s.codec_name.clone()),
        duration_ms: parsed
            .format
            .and_then(|f| f.duration)
            .and_then(|d| d.parse::<f64>().ok())
            .map(|secs| (secs * 1000.0).round() as u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_by_extension() {
        assert_eq!(classify(Path::new("a.JPG")), MediaKind::Image);
        assert_eq!(classify(Path::new("a.jpeg")), MediaKind::Image);
        assert_eq!(classify(Path::new("clip.MP4")), MediaKind::Video);
        assert_eq!(classify(Path::new("clip.mkv")), MediaKind::Video);
        assert_eq!(classify(Path::new("notes.txt")), MediaKind::Other);
        assert_eq!(classify(Path::new("noext")), MediaKind::Other);
    }

    #[test]
    fn parses_ffprobe_video_json() {
        let blob = br#"{
            "streams": [{"width": 1920, "height": 1080, "codec_name": "h264"}],
            "format": {"duration": "65.234000"}
        }"#;
        let info = parse_probe_output(blob).expect("valid ffprobe JSON");
        assert_eq!(info.width, Some(1920));
        assert_eq!(info.height, Some(1080));
        assert_eq!(info.codec.as_deref(), Some("h264"));
        assert_eq!(info.duration_ms, Some(65_234));
        assert_eq!(info.summary(), "1920x1080, 1:05");
    }

    #[test]
    fn parses_ffprobe_image_json_without_duration() {
        let blob = br#"{
            "streams": [{"width": 800, "height": 600, "codec_name": "png"}],
            "format": {}
        }"#;
        let info = parse_probe_output(blob).expect("valid ffprobe JSON");
        assert_eq!(info.width, Some(800));
        assert_eq!(info.duration_ms, None);
        assert_eq!(info.summary(), "800x600");
    }

    #[test]
    fn rejects_garbage_and_failures() {
        assert!(parse_probe_output(b"not json").is_none());
        assert!(parse_probe_output(b"null").is_none());
        assert!(parse_probe_output(b"42").is_none());
        // serde_json accepts structs from sequences positionally, so an empty
        // array yields a fully-defaulted probe output (all-None metadata).
        assert!(parse_probe_output(b"{}").is_some());
        assert!(parse_probe_output(b"[]").is_some());
    }
}