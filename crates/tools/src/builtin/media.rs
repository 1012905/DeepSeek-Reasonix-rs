//! The media face (Go `internal/tools/builtin/chart.go`, `readvideo.go`):
//! `render_chart` validates a chart spec and answers its identity and summary;
//! `read_video` samples still frames from a video through the host's ffmpeg.
//!
//! The chart parser/validator/summary lives in [`chartspec`]; the video
//! container sniffing and sample-point spread are pure here. File reads,
//! ffprobe/ffmpeg execution, frame decode, base64 and path confinement are
//! host seams behind [`VideoHost`].

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::abi::{CallContext, Tool, ToolResult};
use crate::builtin::chartspec::{self, ChartError, Spec};
use crate::refusal::Refusal;

/// `render_chart`, the tool name the model calls.
pub const RENDER_CHART_TOOL_NAME: &str = chartspec::TOOL_NAME;
/// `read_video`, the tool name the model calls.
pub const READ_VIDEO_TOOL_NAME: &str = "read_video";

/// Go `renderChart.Description()`, verbatim.
pub const RENDER_CHART_DESCRIPTION: &str = "Show the user a chart inline. Rows hold only numbers, strings and nulls; no URLs, markup or colours. Returns a chart_id and a text summary, not the rows.";

/// Go `readVideo.Description()`, verbatim.
pub const READ_VIDEO_DESCRIPTION: &str =
    "Look at a video: native if the model watches video, else ffmpeg frames.";

const RENDER_CHART_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["spec_version","title","data","marks"],"properties":{"spec_version":{"type":"integer","minimum":1,"maximum":1},"title":{"type":"string"},"data":{"type":"object","required":["columns","rows"],"properties":{"columns":{"type":"array","items":{"type":"object","required":["name","type"],"properties":{"name":{"type":"string"},"type":{"type":"string","enum":["number","string","date"]}}}},"rows":{"type":"array","items":{"type":"array"}}}},"marks":{"type":"array","items":{"type":"object","required":["type","x","y"],"properties":{"type":{"type":"string","enum":["bar","line","pie"]},"x":{"type":"string"},"y":{"type":"array","items":{"type":"string"}},"color":{"type":"string"},"stacked":{"type":"boolean"},"donut":{"type":"boolean"}}}},"x_axis":{"type":"object"},"y_axis":{"type":"object"}}}"#;

const READ_VIDEO_SCHEMA: &str = r#"{"type":"object","properties":{"path":{"type":"string"},"url":{"type":"string","description":"public https address the user wrote"},"mode":{"type":"string","enum":["auto","frames","native"]},"frames":{"type":"integer","description":"max 12"},"start":{"type":"number","description":"seconds"},"end":{"type":"number","description":"seconds"}},"additionalProperties":false}"#;

// --- read_video refusal identities (Go `readvideo.go:26–33`) ------------------

pub const CODE_VIDEO_FFMPEG_MISSING: &str = "video.ffmpeg_missing";
pub const CODE_VIDEO_UNSUPPORTED: &str = "video.unsupported_container";
pub const CODE_VIDEO_TOO_LARGE: &str = "video.too_large";
pub const CODE_VIDEO_NO_STREAM: &str = "video.no_video_stream";
pub const CODE_VIDEO_PROBE_FAILED: &str = "video.probe_failed";
pub const CODE_VIDEO_DECODE_FAILED: &str = "video.decode_failed";

const READ_VIDEO_DEFAULT_FRAMES: i64 = 8;
const READ_VIDEO_MAX_FRAMES: i64 = 12;
/// Size and frame limits the ffmpeg seam enforces; the pure layer only names
/// them so the host can check the same caps (Go readvideo.go:36–43).
#[allow(dead_code)]
const READ_VIDEO_FRAME_WIDTH: i64 = 768;
#[allow(dead_code)]
const READ_VIDEO_MAX_BYTES: i64 = 4 << 30;
#[allow(dead_code)]
const READ_VIDEO_MAX_FRAME_BYTES: i64 = 4 << 20;
#[allow(dead_code)]
const READ_VIDEO_MAX_PROBE_BYTES: i64 = 1 << 20;
#[allow(dead_code)]
const READ_VIDEO_SNIFF_BYTES: usize = 4096;

const MOV_DEMUXER: &str = "mov,mp4,m4a,3gp,3g2,mj2";

// --- read_video pure helpers --------------------------------------------------

/// Names the ffmpeg demuxer for a container by its magic bytes, or empty when
/// unrecognized (Go `videoDemuxer`).
pub fn video_demuxer(head: &[u8]) -> &'static str {
    if head.len() >= 12 && head[4..8] == *b"ftyp" {
        return MOV_DEMUXER;
    }
    if head.len() >= 4 && head[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        return "matroska,webm";
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"AVI " {
        return "avi";
    }
    if head.len() >= 4 && &head[..3] == b"FLV" && head[3] == 1 {
        return "flv";
    }
    if head.len() >= 16
        && head[..16]
            == [
                0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62,
                0xce, 0x6c,
            ]
    {
        return "asf";
    }
    if head.len() >= 4 && head[..4] == [0x00, 0x00, 0x01, 0xba] {
        return "mpeg";
    }
    if head.len() >= 4 && &head[..4] == b"OggS" {
        return "ogg";
    }
    if head.len() >= 3 * 188 && head[0] == 0x47 && head[188] == 0x47 && head[376] == 0x47 {
        return "mpegts";
    }
    ""
}

/// Spreads `n` sample points over `[from, to]` at the middle of `n` equal
/// slices, so the last one never sits on the end of the file (Go
/// `sampleTimes`).
pub fn sample_times(from: f64, to: f64, n: i64) -> Vec<f64> {
    if n <= 1 || to <= from {
        return vec![from];
    }
    let step = (to - from) / n as f64;
    (0..n).map(|i| from + (i as f64 + 0.5) * step).collect()
}

/// Normalizes the requested frame count: non-positive to the default, above
/// the cap clamped to the max. Returns the count and whether it was capped
/// (Go readvideo.go:113–119).
pub fn normalize_frame_count(frames: i64) -> (i64, bool) {
    if frames <= 0 {
        return (READ_VIDEO_DEFAULT_FRAMES, false);
    }
    if frames > READ_VIDEO_MAX_FRAMES {
        return (READ_VIDEO_MAX_FRAMES, true);
    }
    (frames, false)
}

/// Probe facts the seam read back (Go `videoInfo`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoInfo {
    pub width: i64,
    pub height: i64,
    pub duration: f64,
}

/// The video host seam: file sniff, ffprobe and ffmpeg frame extraction. The
/// tool layer owns argument validation, sample timing and summary rendering;
/// the seam owns confinement, process execution and decoding.
pub trait VideoHost: Send + Sync {
    /// Names the container from the file's own bytes and applies the size cap.
    fn sniff(&self, path: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
    /// Runs ffprobe and reports the first video stream's size and duration.
    fn probe(
        &self,
        demuxer: &str,
        path: &str,
    ) -> Result<VideoInfo, Box<dyn std::error::Error + Send + Sync>>;
    /// Extracts one still frame as JPEG bytes at `at` seconds.
    fn frame(
        &self,
        demuxer: &str,
        path: &str,
        at: f64,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>;
}

/// The video host bound to one call, when installed.
pub fn video_host_from(ctx: &CallContext) -> Option<Arc<dyn VideoHost>> {
    ctx.value()?.downcast_ref::<Arc<dyn VideoHost>>().cloned()
}

fn ffmpeg_missing() -> Refusal {
    Refusal::code(
        CODE_VIDEO_FFMPEG_MISSING,
        "read_video needs ffmpeg and ffprobe on this machine's PATH and found at least one of them missing. Install ffmpeg (it includes ffprobe), then try again; no other tool can read a video",
    )
}

fn invalid_err(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

#[derive(Deserialize, Default)]
struct VideoArgs {
    #[serde(default)]
    path: String,
    #[serde(default)]
    frames: i64,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
}

// --- render_chart tool --------------------------------------------------------

/// The `render_chart` tool (Go `renderChart`).
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderChartTool;

pub fn render_chart_tool() -> Arc<dyn Tool> {
    Arc::new(RenderChartTool)
}

impl Tool for RenderChartTool {
    fn name(&self) -> &str {
        RENDER_CHART_TOOL_NAME
    }

    fn description(&self) -> &str {
        RENDER_CHART_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(RENDER_CHART_SCHEMA).expect("the render_chart schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let spec: Spec = chartspec::parse(args.to_string().as_bytes())
            .map_err(|e: ChartError| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let id = chartspec::chart_id(&spec)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        Ok(format!("chart_id: {id}\n{}", spec.summary()))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// --- read_video tool ----------------------------------------------------------

/// The `read_video` tool (Go `readVideo`).
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadVideoTool;

pub fn read_video_tool() -> Arc<dyn Tool> {
    Arc::new(ReadVideoTool)
}

impl Tool for ReadVideoTool {
    fn name(&self) -> &str {
        READ_VIDEO_TOOL_NAME
    }

    fn description(&self) -> &str {
        READ_VIDEO_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(READ_VIDEO_SCHEMA).expect("the read_video schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: VideoArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        if parsed.path.is_empty() {
            return Err(invalid_err("path is required"));
        }
        let Some(host) = video_host_from(ctx) else {
            return Err(ffmpeg_missing().into());
        };

        let demuxer = host.sniff(&parsed.path)?;
        let info = host.probe(&demuxer, &parsed.path)?;

        let (frames, capped) = normalize_frame_count(parsed.frames);
        let from = parsed.start.unwrap_or(0.0).max(0.0);
        let to = if info.duration > 0.0 && parsed.end.is_some_and(|e| e < info.duration) {
            parsed.end.unwrap_or(info.duration).max(0.0)
        } else {
            info.duration
        };
        let times = sample_times(from, to, frames);

        let mut images = 0usize;
        let mut lines: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        for (i, at) in times.iter().enumerate() {
            match host.frame(&demuxer, &parsed.path, *at) {
                Ok(_jpeg) => {
                    images += 1;
                    lines.push(format!("[frame {}/{} at {at:.1}fs]", i + 1, times.len()));
                }
                Err(err) => {
                    failed.push(format!("{at:.1}fs"));
                    if images == 0 && i == times.len() - 1 {
                        return Err(err);
                    }
                }
            }
        }
        if images == 0 {
            return Err(Refusal::code(
                CODE_VIDEO_DECODE_FAILED,
                format!("no frame could be read from {}", parsed.path),
            )
            .into());
        }

        Ok(render_video_summary(
            &parsed.path,
            &info,
            images,
            times.len(),
            from,
            to,
            capped,
            &failed,
            &lines,
        ))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The summary line plus frame list (Go `readvideo.go:148–169`). The image
/// payloads themselves ride the host's detailed-result channel.
#[allow(clippy::too_many_arguments)]
pub fn render_video_summary(
    path: &str,
    info: &VideoInfo,
    images: usize,
    total_times: usize,
    from: f64,
    to: f64,
    capped: bool,
    failed: &[String],
    lines: &[String],
) -> String {
    let mut b = String::new();
    b.push_str(&format!("video {path}: {}×{}", info.width, info.height));
    if info.duration > 0.0 {
        b.push_str(&format!(", {:.1}fs", info.duration));
    }
    b.push_str(&format!(". {images} frame(s) sampled"));
    if total_times > 1 {
        b.push_str(&format!(" evenly between {from:.1}fs and {to:.1}fs"));
    }
    b.push_str(". Audio and motion are not seen.");
    if info.duration <= 0.0 {
        b.push_str(" The video's length is unknown, so only its first moment was sampled.");
    }
    if capped {
        b.push_str(&format!(" frames was capped at {READ_VIDEO_MAX_FRAMES}."));
    }
    if !failed.is_empty() {
        b.push_str(&format!(
            " Frames at {} could not be read.",
            failed.join(", ")
        ));
    }
    b.push('\n');
    b.push_str(&lines.join("\n"));
    b
}

/// The media face for one composition root.
pub fn media_tools() -> Vec<Arc<dyn Tool>> {
    vec![render_chart_tool(), read_video_tool()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_valid_json() {
        let _ = RenderChartTool.schema();
        let _ = ReadVideoTool.schema();
    }

    #[test]
    fn demuxer_magic_bytes() {
        assert_eq!(video_demuxer(&b"\x00\x00\x00\x18ftypmp42"[..]), MOV_DEMUXER);
        assert_eq!(
            video_demuxer(&[0x1a, 0x45, 0xdf, 0xa3, 1, 2][..]),
            "matroska,webm"
        );
        assert_eq!(video_demuxer(b"RIFF\x00\x00\x00\x00AVI "), "avi");
        assert_eq!(video_demuxer(b"OggS"), "ogg");
        assert_eq!(video_demuxer(b"not a container"), "");
    }

    #[test]
    fn sample_times_middle_slices() {
        assert_eq!(sample_times(0.0, 10.0, 1), vec![0.0]);
        let times = sample_times(0.0, 10.0, 4);
        assert_eq!(times.len(), 4);
        assert!((times[0] - 1.25).abs() < f64::EPSILON);
        assert!((times[3] - 8.75).abs() < f64::EPSILON);
    }

    #[test]
    fn frame_count_normalization() {
        assert_eq!(normalize_frame_count(0), (8, false));
        assert_eq!(normalize_frame_count(3), (3, false));
        assert_eq!(normalize_frame_count(99), (12, true));
    }

    #[test]
    fn render_chart_answers_id_and_summary() {
        let args = serde_json::json!({
            "spec_version": 1,
            "title": "Q3",
            "data": {
                "columns": [{"name": "m", "type": "string"}, {"name": "v", "type": "number"}],
                "rows": [["a", 3], ["b", 7]]
            },
            "marks": [{"type": "line", "x": "m", "y": ["v"]}]
        });
        let out = RenderChartTool
            .execute(&CallContext::default(), &args)
            .expect("chart renders");
        assert!(out.starts_with("chart_id: chart-"));
        assert!(out.contains("chart \"Q3\", 2 rows, 2 columns"));
    }
}
