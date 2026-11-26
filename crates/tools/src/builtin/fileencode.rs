//! Public file-encoding API shared with the checkpoint kernel.

#![forbid(unsafe_code)]

pub use reasonix_core::fileencoding::{ERR_UNENCODABLE, UnencodableError, encode, encode_bytes};

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_core::fileencoding::{Kind, decode, detect_and_decode};

    #[test]
    fn encode_round_trips_detected_kinds() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("utf8", "你好世界\n".as_bytes().to_vec()),
            ("utf8-bom", {
                let mut v = vec![0xEF, 0xBB, 0xBF];
                v.extend_from_slice("标题：旧的说明\r\n".as_bytes());
                v
            }),
            ("utf16le", {
                let mut v = vec![0xFF, 0xFE];
                for u in "第二行".encode_utf16() {
                    v.extend_from_slice(&u.to_le_bytes());
                }
                v
            }),
        ];
        for (label, data) in cases {
            let (kind, text) = detect_and_decode(&data);
            let back = encode_bytes(&text, kind).unwrap();
            assert_eq!(back, data, "{label} round trip");
        }
    }

    #[test]
    fn gbk_round_trip_restores_cp936_euro_byte() {
        let data = vec![0xC4, 0xE3, 0x80, 0x80, 0x80];
        let decoded = decode(&data, Kind::Gbk).into_owned();
        let back = encode_bytes(&decoded, Kind::Gbk).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn unencodable_error_names_first_character() {
        let err = encode("a\u{1F600}b", Kind::Gbk).unwrap_err();
        assert_eq!(err.charset, "GBK");
        assert_eq!(err.rune, '\u{1F600}');
        assert_eq!(err.offset, 1);
        let rendered = err.to_string();
        assert!(rendered.contains("cannot be written in GBK"), "{rendered}");
        assert!(rendered.contains(ERR_UNENCODABLE) || rendered.starts_with('"'));
    }

    #[test]
    fn lossy_kind_keeps_invalid_bytes() {
        let data: Vec<u8> = vec![0x61, 0xFF, 0x0A];
        let (kind, text) = detect_and_decode(&data);
        assert_eq!(kind, Kind::LossyUtf8);
        assert_eq!(encode_bytes(&text, kind).unwrap(), data);
    }
}
