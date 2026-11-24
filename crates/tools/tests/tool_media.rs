//! The media face at the tools boundary (Go `internal/tools/builtin/chart.go`,
//! `readvideo.go`, `internal/contract/chartspec/`): chart card and summary,
//! video container sniffing, sample-time spread, frame normalization, and the
//! ffmpeg-missing seam refusal.

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{
    READ_VIDEO_TOOL_NAME, RENDER_CHART_TOOL_NAME, ReadVideoTool, RenderChartTool, VideoHost,
    VideoInfo, normalize_frame_count, sample_times, video_demuxer,
};
use serde_json::json;

struct FakeVideo;

impl VideoHost for FakeVideo {
    fn sniff(&self, _path: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok("mov,mp4,m4a,3gp,3g2,mj2".into())
    }
    fn probe(
        &self,
        _demuxer: &str,
        _path: &str,
    ) -> Result<VideoInfo, Box<dyn std::error::Error + Send + Sync>> {
        Ok(VideoInfo {
            width: 1280,
            height: 720,
            duration: 20.0,
        })
    }
    fn frame(
        &self,
        _demuxer: &str,
        _path: &str,
        _at: f64,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(vec![1, 2, 3])
    }
}

fn ctx_with(host: Arc<FakeVideo>) -> CallContext {
    let bound: Arc<Arc<dyn VideoHost>> = Arc::new(host as Arc<dyn VideoHost>);
    CallContext::new().with_value(bound)
}

#[test]
fn render_chart_card_is_read_only() {
    let tool = RenderChartTool;
    assert_eq!(tool.name(), RENDER_CHART_TOOL_NAME);
    assert!(tool.description().contains("chart_id"));
    assert!(tool.schema().is_object());
    assert!(tool.read_only());
}

#[test]
fn render_chart_answers_id_and_summary_without_rows() {
    let out = RenderChartTool
        .execute(
            &CallContext::new(),
            &json!({
                "spec_version": 1,
                "title": "Q3 revenue",
                "data": {
                    "columns": [{"name": "month", "type": "string"}, {"name": "value", "type": "number"}],
                    "rows": [["Jul", 30], ["Aug", 42], ["Sep", 51]]
                },
                "marks": [{"type": "bar", "x": "month", "y": ["value"]}]
            }),
        )
        .expect("chart renders");
    assert!(out.starts_with("chart_id: chart-"));
    assert!(out.contains("chart \"Q3 revenue\", 3 rows, 2 columns"));
    assert!(out.contains("series \"value\" n=3 min=30 max=51"));
    // The rows themselves never ride the answer.
    assert!(!out.contains("\"Jul\""));
}

#[test]
fn render_chart_rejects_a_bad_spec_with_a_typed_code() {
    let err = RenderChartTool
        .execute(
            &CallContext::new(),
            &json!({
                "spec_version": 2,
                "title": "t",
                "data": {"columns": [{"name": "a", "type": "string"}], "rows": []},
                "marks": [{"type": "bar", "x": "a", "y": []}]
            }),
        )
        .expect_err("bad spec refused");
    assert!(err.to_string().contains("chart.spec_version_unsupported"));
}

#[test]
fn read_video_card_is_read_only() {
    let tool = ReadVideoTool;
    assert_eq!(tool.name(), READ_VIDEO_TOOL_NAME);
    assert!(tool.description().contains("ffmpeg"));
    assert!(tool.schema().is_object());
    assert!(tool.read_only());
}

#[test]
fn demuxer_reads_container_magic_bytes() {
    assert_eq!(
        video_demuxer(b"\x00\x00\x00\x18ftypmp42"),
        "mov,mp4,m4a,3gp,3g2,mj2"
    );
    assert_eq!(
        video_demuxer(&[0x1a, 0x45, 0xdf, 0xa3, 1, 2]),
        "matroska,webm"
    );
    assert_eq!(video_demuxer(b"RIFF\x00\x00\x00\x00AVI "), "avi");
    assert_eq!(video_demuxer(b"OggS"), "ogg");
    assert_eq!(video_demuxer(b"plain"), "");
}

#[test]
fn sample_times_land_in_the_middle_of_slices() {
    assert_eq!(sample_times(0.0, 10.0, 1), vec![0.0]);
    let times = sample_times(0.0, 10.0, 4);
    assert_eq!(times.len(), 4);
    assert!((times[0] - 1.25).abs() < f64::EPSILON);
    assert!((times[3] - 8.75).abs() < f64::EPSILON);
}

#[test]
fn frame_count_normalizes_default_and_cap() {
    assert_eq!(normalize_frame_count(0), (8, false));
    assert_eq!(normalize_frame_count(3), (3, false));
    assert_eq!(normalize_frame_count(99), (12, true));
}

#[test]
fn read_video_summarizes_sampled_frames() {
    let host = Arc::new(FakeVideo);
    let out = ReadVideoTool
        .execute(&ctx_with(host), &json!({"path": "clip.mp4", "frames": 4}))
        .expect("video samples through the seam");
    assert!(out.contains("video clip.mp4: 1280×720, 20.0fs"));
    assert!(out.contains("4 frame(s) sampled evenly between 0.0fs and 20.0fs"));
}

#[test]
fn read_video_without_host_fails_closed_as_ffmpeg_missing() {
    let err = ReadVideoTool
        .execute(&CallContext::new(), &json!({"path": "clip.mp4"}))
        .expect_err("seam absent refuses");
    assert!(err.to_string().contains("video.ffmpeg_missing"));
}
