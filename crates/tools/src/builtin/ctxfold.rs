//! The `compress` tool: one explicit, model-requested context fold (tools.md
//! §1.22; Go `internal/tools/builtin/compress.go`,
//! `internal/contract/tool/compress.go`).
//!
//! The fold rewrites the owning agent's context projection; the actual
//! summarizer and projection state live in the agent slice behind the
//! [`ContextCompressor`] seam bound on the call context. Parent and child
//! agents share the stable tool schema without sharing projection state.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::abi::{CallContext, Capability, CapabilitySet, SequentialTool, Tool, ToolResult};

/// `compress`, the tool name the model calls.
pub const COMPRESS_TOOL_NAME: &str = "compress";

/// Anchor byte ceiling (`maxCompressAnchorBytes`, compress.go:13). The oracle
/// measures UTF-8 bytes, not runes.
pub const MAX_COMPRESS_ANCHOR_BYTES: usize = 512;
/// Focus byte ceiling (`maxCompressFocusBytes`, compress.go:14).
pub const MAX_COMPRESS_FOCUS_BYTES: usize = 2000;

/// One explicit, model-requested context fold.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompressRequest {
    pub direction: String,
    pub anchor: String,
    pub focus: String,
}

/// The stable model-facing outcome of a context fold (Go `CompressResult`).
/// The JSON field names are the frozen provider-facing contract.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CompressResult {
    pub status: String,
    pub direction: String,
    pub anchor: String,
    pub messages: i64,
    #[serde(rename = "source_tokens")]
    pub source_tokens: i64,
    #[serde(rename = "projection_tokens")]
    pub projection_tokens: i64,
    pub mode: String,
    pub reason: String,
}

/// Supplied by the agent that owns the active session (Go
/// `tool.ContextCompressor`). Keeping the binding on the call context lets
/// parent and child agents share one tool schema without sharing projection
/// state.
pub trait ContextCompressor: Send + Sync {
    fn compress_context(
        &self,
        req: &CompressRequest,
    ) -> Result<CompressResult, Box<dyn std::error::Error + Send + Sync>>;

    /// Type-erased view for host-side recognition.
    fn as_any(&self) -> &dyn Any;
}

/// The compressor bound to one call, when the owning agent installed one (Go
/// `tool.ContextCompressorFromContext`).
pub fn compressor_from(ctx: &CallContext) -> Option<Arc<dyn ContextCompressor>> {
    ctx.value()?
        .downcast_ref::<Arc<dyn ContextCompressor>>()
        .cloned()
}

/// Go `compressContext.ReadOnly()`: true in the permission/workspace sense —
/// the fold changes only the owning agent's context projection. The agent
/// batcher separately forces it into a serial lane because projection
/// installation is stateful.
#[derive(Debug, Default, Clone, Copy)]
pub struct CompressTool;

/// The one instance a composition root registers.
pub fn compress_tool() -> Arc<dyn Tool> {
    Arc::new(CompressTool)
}

/// The fold rewrites the transcript every other call in the batch is reading
/// from, so it runs sequentially even though it is read-only.
struct AlwaysSequential;

impl Capability for AlwaysSequential {}
impl SequentialTool for AlwaysSequential {
    fn sequential(&self, _ctx: &CallContext, _args: &Value) -> bool {
        true
    }
}

/// Go `compressContext.Description()`, verbatim.
pub const COMPRESS_DESCRIPTION: &str = "Compress a selected part of the current model-visible conversation without deleting visible history. Use only when the user explicitly asks for context compression. Choose `before` to summarize everything before the uniquely matched user turn while keeping that turn and later context, or `after` to summarize from that turn through the last completed turn while keeping the active turn. The anchor must be an exact, unique excerpt from a real user message; use a longer excerpt if the tool reports multiple matches.";

/// Go `compressContext.Schema()`, verbatim.
const GO_SCHEMA: &str = r#"{
"type":"object",
"additionalProperties":false,
"properties":{
  "direction":{"type":"string","enum":["before","after"],"description":"Which side of the anchor user turn to compress."},
  "anchor":{"type":"string","minLength":1,"maxLength":512,"description":"An exact, unique excerpt from one real user message in the current model-visible conversation."},
  "focus":{"type":"string","maxLength":2000,"description":"Optional guidance about facts or decisions the summary must preserve."}
},
"required":["direction","anchor"]
}"#;

impl Tool for CompressTool {
    fn name(&self) -> &str {
        COMPRESS_TOOL_NAME
    }

    fn description(&self) -> &str {
        COMPRESS_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(GO_SCHEMA).expect("the compress schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let request: RawRequest = serde_json::from_value(args.clone())
            .map_err(|e| Box::new(std::io::Error::other(format!("invalid compress args: {e}"))))?;
        let direction = request.direction.trim().to_owned();
        let anchor = request.anchor.trim().to_owned();
        let focus = request.focus.trim().to_owned();

        if direction != "before" && direction != "after" {
            return Err(Box::new(std::io::Error::other(
                "compress: direction must be before or after",
            )));
        }
        if anchor.is_empty() {
            return Err(Box::new(std::io::Error::other(
                "compress: anchor must not be empty",
            )));
        }
        if anchor.len() > MAX_COMPRESS_ANCHOR_BYTES {
            return Err(Box::new(std::io::Error::other(format!(
                "compress: anchor exceeds {MAX_COMPRESS_ANCHOR_BYTES} bytes"
            ))));
        }
        if focus.len() > MAX_COMPRESS_FOCUS_BYTES {
            return Err(Box::new(std::io::Error::other(format!(
                "compress: focus exceeds {MAX_COMPRESS_FOCUS_BYTES} bytes"
            ))));
        }

        let Some(compressor) = compressor_from(ctx) else {
            return Err(Box::new(std::io::Error::other(
                "compress is unavailable outside an active agent session",
            )));
        };
        let result = compressor.compress_context(&CompressRequest {
            direction,
            anchor,
            focus,
        })?;
        let out = serde_json::to_string(&result).map_err(|e| {
            Box::new(std::io::Error::other(format!(
                "encode compress result: {e}"
            ))) as Box<dyn std::error::Error + Send + Sync>
        })?;
        Ok(out)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "sequential",
            Arc::new(AlwaysSequential) as Arc<dyn SequentialTool>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Deserialize)]
struct RawRequest {
    #[serde(default)]
    direction: String,
    #[serde(default)]
    anchor: String,
    #[serde(default)]
    focus: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::runs_sequentially;
    use serde_json::json;

    struct StubCompressor {
        seen: std::sync::Mutex<Vec<CompressRequest>>,
        result: CompressResult,
    }

    impl ContextCompressor for StubCompressor {
        fn compress_context(
            &self,
            req: &CompressRequest,
        ) -> Result<CompressResult, Box<dyn std::error::Error + Send + Sync>> {
            self.seen.lock().expect("seen lock").push(req.clone());
            Ok(self.result.clone())
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn stub(result: CompressResult) -> Arc<StubCompressor> {
        Arc::new(StubCompressor {
            seen: std::sync::Mutex::new(Vec::new()),
            result,
        })
    }

    #[test]
    fn schema_name_description_and_flags_match_the_go_contract() {
        let t = CompressTool;
        assert_eq!(t.name(), COMPRESS_TOOL_NAME);
        assert_eq!(t.name(), "compress");
        assert_eq!(t.description(), COMPRESS_DESCRIPTION);
        assert!(t.read_only(), "the fold changes only the projection");
        let s = t.schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["required"], json!(["direction", "anchor"]));
        assert_eq!(
            s["properties"]["direction"]["enum"],
            json!(["before", "after"])
        );
        assert_eq!(s["properties"]["anchor"]["maxLength"], 512);
        assert_eq!(s["properties"]["focus"]["maxLength"], 2000);
        let ctx = CallContext::new();
        assert!(
            runs_sequentially(&t, &ctx, &json!({})),
            "a fold rewrites the transcript other calls read from"
        );
    }

    #[test]
    fn execute_forwards_the_trimmed_request_and_echoes_the_result() {
        let result = CompressResult {
            status: "ok".to_owned(),
            direction: "before".to_owned(),
            anchor: "unique".to_owned(),
            messages: 4,
            source_tokens: 900,
            projection_tokens: 200,
            mode: "summarized".to_owned(),
            reason: String::new(),
        };
        let c = stub(result.clone());
        let port: Arc<dyn ContextCompressor> = c.clone();
        let ctx = CallContext::new().with_value(Arc::new(port));
        let out = CompressTool
            .execute(
                &ctx,
                &json!({"direction": "before", "anchor": "  unique excerpt  ", "focus": " keep decisions "}),
            )
            .expect("compress executes");
        let seen = c.seen.lock().expect("seen lock").clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].direction, "before");
        assert_eq!(seen[0].anchor, "unique excerpt");
        assert_eq!(seen[0].focus, "keep decisions");
        let got: CompressResult = serde_json::from_str(&out).expect("decode result");
        assert_eq!(got, result);
        let wire: Value = serde_json::from_str(&out).expect("wire json");
        assert_eq!(wire["source_tokens"], 900);
        assert_eq!(wire["projection_tokens"], 200);
    }

    #[test]
    fn invalid_args_are_refused_before_the_compressor_is_asked() {
        let ctx = CallContext::new();
        let err = CompressTool
            .execute(&ctx, &json!("not an object"))
            .unwrap_err();
        assert!(err.to_string().starts_with("invalid compress args:"));
    }

    #[test]
    fn invalid_direction_and_empty_anchor_are_refused() {
        let ctx = CallContext::new();
        for (args, want) in [
            (json!({"direction": "around", "anchor": "x"}), "direction"),
            (json!({"direction": "Before", "anchor": "x"}), "direction"),
            (json!({"direction": "before", "anchor": "  "}), "empty"),
        ] {
            let err = CompressTool.execute(&ctx, &args).unwrap_err();
            assert!(err.to_string().contains(want), "{err}");
        }
    }

    #[test]
    fn anchor_and_focus_byte_limits_hold_at_the_boundary() {
        // 512 ASCII bytes pass; 513 fail. The oracle counts bytes.
        let anchor_ok = "a".repeat(MAX_COMPRESS_ANCHOR_BYTES);
        let anchor_over = "a".repeat(MAX_COMPRESS_ANCHOR_BYTES + 1);
        let focus_ok = "b".repeat(MAX_COMPRESS_FOCUS_BYTES);
        let focus_over = "b".repeat(MAX_COMPRESS_FOCUS_BYTES + 1);

        let c = stub(CompressResult::default());
        let port: Arc<dyn ContextCompressor> = c.clone();
        let ctx = CallContext::new().with_value(Arc::new(port));
        let ok = CompressTool.execute(
            &ctx,
            &json!({"direction": "before", "anchor": anchor_ok, "focus": focus_ok}),
        );
        assert!(ok.is_ok(), "512-byte anchor and 2000-byte focus pass");

        let err = CompressTool
            .execute(
                &CallContext::new(),
                &json!({"direction": "before", "anchor": anchor_over}),
            )
            .unwrap_err();
        assert!(err.to_string().contains("512 bytes"), "{err}");

        let err = CompressTool
            .execute(
                &CallContext::new(),
                &json!({"direction": "before", "anchor": "x", "focus": focus_over}),
            )
            .unwrap_err();
        assert!(err.to_string().contains("2000 bytes"), "{err}");
    }

    #[test]
    fn a_cjk_anchor_is_counted_by_bytes_not_runes() {
        // Each CJK character is 3 UTF-8 bytes: 170 characters = 510 bytes,
        // under the 512-byte ceiling; 171 = 513 bytes, over it.
        let cjk_ok = "界".repeat(170);
        assert_eq!(cjk_ok.len(), 510);
        let cjk_over = "界".repeat(171);
        assert_eq!(cjk_over.len(), 513);
        let err = CompressTool
            .execute(
                &CallContext::new(),
                &json!({"direction": "before", "anchor": cjk_over}),
            )
            .unwrap_err();
        assert!(err.to_string().contains("512 bytes"), "{err}");
    }

    #[test]
    fn outside_an_active_session_the_tool_is_unavailable() {
        let err = CompressTool
            .execute(
                &CallContext::new(),
                &json!({"direction": "after", "anchor": "unique"}),
            )
            .unwrap_err();
        assert!(err.to_string().contains("active agent session"), "{err}");
    }
}
