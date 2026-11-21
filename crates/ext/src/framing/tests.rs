use super::*;
use std::io::{self, BufRead, BufReader, Cursor, Read, Write};

fn raw(s: &str) -> RawJson {
    RawJson::parse(s).expect("valid raw json")
}

#[test]
fn strict_request_preserves_exact_frame_and_remainder() {
    let first = "  {\"jsonrpc\":\"2.0\",\"id\":\"init-1\",\"method\":\"remote/initialize\",\"params\":{\"x\":1}} \r\n";
    let second = "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"next\",\"params\":{}}\n";
    let mut reader = BufReader::new(Cursor::new(format!("{first}{second}")));
    let frame = read_strict_request_frame(&mut reader, 8 << 20)
        .unwrap()
        .unwrap();
    assert_eq!(frame.id, "\"init-1\"");
    assert_eq!(frame.method, "remote/initialize");
    assert_eq!(frame.params.as_deref(), Some("{\"x\":1}"));
    assert!(frame.raw.contains("init-1"));
    let mut remainder = String::new();
    reader.read_to_string(&mut remainder).unwrap();
    assert_eq!(remainder, second);
}

#[test]
fn strict_request_rejects_non_requests_and_limit() {
    for input in [
        "{\"jsonrpc\":\"2.0\",\"method\":\"note\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n",
        "{\"jsonrpc\":\"1.0\",\"id\":1,\"method\":\"bad\"}\n",
    ] {
        assert!(
            read_strict_request_frame(&mut BufReader::new(Cursor::new(input)), 8 << 20).is_err(),
            "invalid bootstrap frame accepted: {input}"
        );
    }
    let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\"}\n";
    let err = read_strict_request_frame(&mut BufReader::new(Cursor::new(input)), 8)
        .expect_err("limit error");
    match err {
        StrictFrameError::Read(ReadError::TooLarge(e)) => {
            assert_eq!(e.direction, crate::error::Direction::Inbound);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn response_id_for_error_rejects_unsafe_ids() {
    for (input, want) in [
        ("\"request\"", "\"request\""),
        ("-17", "-17"),
        ("null", "null"),
        ("", "null"),
        ("true", "null"),
        ("{}", "null"),
        ("[]", "null"),
        ("1.5", "null"),
    ] {
        assert_eq!(response_id_for_error(input), want, "input={input:?}");
    }
}

#[test]
fn inbound_limit_includes_newline() {
    let line = "{\"jsonrpc\":\"2.0\",\"method\":\"n\"}\n";
    let mut reader = BufReader::with_capacity(4, Cursor::new(line));
    let err = read_frame(&mut reader, line.len() - 1).expect_err("too large");
    match err {
        ReadError::TooLarge(e) => {
            assert_eq!(e.direction, crate::error::Direction::Inbound);
            assert_eq!(e.limit, line.len() - 1);
            assert!(e.size >= line.len());
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn outbound_limit_includes_newline() {
    let value = serde_json::json!({"body": "too large"});
    let err = encode_frame(&value, 8).expect_err("too large");
    assert_eq!(err.direction, crate::error::Direction::Outbound);
    assert!(err.size > err.limit);
}

#[test]
fn decode_frame_accepts_tolerant_request_and_strict_shape() {
    let line = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"m\",\"params\":{},\"extra\":7}";
    match decode_frame(line).unwrap() {
        Frame::Request { id, method, .. } => {
            assert_eq!(id, RequestId::Integer(1));
            assert_eq!(method, "m");
        }
        other => panic!("unexpected frame: {other:?}"),
    }
    assert_eq!(decode_frame("not json"), Err(FrameError::Parse));
    assert!(matches!(
        decode_frame("{\"jsonrpc\":\"1.0\",\"id\":1,\"method\":\"m\"}"),
        Err(FrameError::InvalidRequest(_))
    ));
    assert!(matches!(
        decode_frame("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"m\",\"result\":{}}"),
        Err(FrameError::InvalidRequest(_))
    ));
    assert!(matches!(
        decode_frame("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"m\",\"params\":7}"),
        Err(FrameError::InvalidRequest(_))
    ));
    assert!(matches!(
        decode_frame("{\"jsonrpc\":\"2.0\",\"id\":true,\"method\":\"m\"}"),
        Err(FrameError::InvalidRequest(_))
    ));
}

#[test]
fn encode_frame_is_line_delimited_and_round_trips() {
    let params = raw("{\"a\":1}");
    let env = OutboundEnvelope::request(RequestId::Integer(1), "m", &params);
    let bytes = encode_frame(&env, 8 << 20).unwrap();
    assert_eq!(bytes.last(), Some(&b'\n'));
    let text = std::str::from_utf8(&bytes).unwrap().to_owned();
    let mut reader = BufReader::new(Cursor::new(bytes));
    let (payload, _) = read_frame(&mut reader, 8 << 20).unwrap().unwrap();
    assert_eq!(payload, text.trim_end());
    match decode_frame(&payload).unwrap() {
        Frame::Request { method, params, .. } => {
            assert_eq!(method, "m");
            assert_eq!(params.unwrap().get(), "{\"a\":1}");
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}

#[test]
fn strict_dto_decoder_rejects_unknown_members() {
    #[derive(Debug, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Params {
        tool: String,
    }
    let frame = decode_frame(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"m\",\"params\":{\"tool\":\"x\",\"typo\":1}}",
    )
    .unwrap();
    assert!(decode_params::<Params>(&frame).is_err());
    let frame =
        decode_frame("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"m\",\"params\":{\"tool\":\"x\"}}")
            .unwrap();
    assert_eq!(decode_params::<Params>(&frame).unwrap().tool, "x");
}

#[test]
fn in_memory_pipe_round_trip() {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct PipeHalf(Arc<Mutex<VecDeque<u8>>>);
    impl Write for PipeHalf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut queue = self.0.lock().unwrap();
            queue.extend(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl BufRead for PipeHalf {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Ok(&[])
        }
        fn consume(&mut self, _amt: usize) {}
    }
    impl io::Read for PipeHalf {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut queue = self.0.lock().unwrap();
            let mut n = 0;
            while n < buf.len() {
                match queue.pop_front() {
                    Some(b) => buf[n] = b,
                    None => break,
                }
                n += 1;
            }
            Ok(n)
        }
    }

    let shared = Arc::new(Mutex::new(VecDeque::new()));
    let sink = PipeHalf(Arc::clone(&shared));
    let source = PipeHalf(Arc::clone(&shared));
    let writer = FrameWriter::new(sink, 8 << 20);
    let pipe_params = raw("{\"n\":1}");
    let env = OutboundEnvelope::request(RequestId::Integer(7), "ext/ping", &pipe_params);
    writer.write_frame(&env).unwrap();
    let mut reader = BufReader::new(source);
    let (payload, _) = read_frame(&mut reader, 8 << 20).unwrap().unwrap();
    match decode_frame(&payload).unwrap() {
        Frame::Request {
            id, method, params, ..
        } => {
            assert_eq!(id, RequestId::Integer(7));
            assert_eq!(method, "ext/ping");
            assert_eq!(params.unwrap().get(), "{\"n\":1}");
        }
        other => panic!("unexpected frame: {other:?}"),
    }
}
