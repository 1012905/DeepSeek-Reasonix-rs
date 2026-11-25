use serde::de::Error as _;

pub(crate) fn skip_space(raw: &[u8], mut offset: usize) -> usize {
    while matches!(raw.get(offset), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        offset += 1;
    }
    offset
}

pub(crate) fn nullable<T: serde::de::DeserializeOwned>(
    raw: &[u8],
) -> Result<Option<T>, serde_json::Error> {
    serde_json::from_slice(raw)
}

pub(crate) fn string(raw: &[u8]) -> Result<Option<String>, serde_json::Error> {
    nullable(raw)
}

pub(crate) fn object_fields<F>(
    raw: &[u8],
    offset: usize,
    mut visit: F,
) -> Result<(), serde_json::Error>
where
    F: FnMut(&str, usize, usize) -> Result<(), serde_json::Error>,
{
    if raw.get(offset) != Some(&b'{') {
        return Err(serde_json::Error::custom("expected JSON object"));
    }
    let mut offset = skip_space(raw, offset + 1);
    if raw.get(offset) == Some(&b'}') {
        return Ok(());
    }
    loop {
        let key_end = value_end(raw, offset);
        let key: String = serde_json::from_slice(&raw[offset..key_end])?;
        offset = skip_space(raw, key_end);
        if raw.get(offset) != Some(&b':') {
            return Err(serde_json::Error::custom("expected object colon"));
        }
        let begin = skip_space(raw, offset + 1);
        let end = value_end(raw, begin);
        visit(&key, begin, end)?;
        offset = skip_space(raw, end);
        match raw.get(offset) {
            Some(b'}') => return Ok(()),
            Some(b',') => offset = skip_space(raw, offset + 1),
            _ => return Err(serde_json::Error::custom("expected object separator")),
        }
    }
}

pub(crate) fn value_end(raw: &[u8], offset: usize) -> usize {
    match raw.get(offset) {
        Some(b'"') => string_end(raw, offset),
        Some(b'{' | b'[') => {
            let mut depth = 1usize;
            let mut offset = offset + 1;
            while offset < raw.len() && depth != 0 {
                match raw[offset] {
                    b'"' => {
                        offset = string_end(raw, offset);
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => depth -= 1,
                    _ => {}
                }
                offset += 1;
            }
            offset
        }
        _ => {
            let mut offset = offset;
            while offset < raw.len() {
                if matches!(
                    raw[offset],
                    b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}'
                ) {
                    break;
                }
                offset += 1;
            }
            offset
        }
    }
}

fn string_end(raw: &[u8], mut offset: usize) -> usize {
    offset += 1;
    while offset < raw.len() {
        match raw[offset] {
            b'\\' => offset += 2,
            b'"' => return offset + 1,
            _ => offset += 1,
        }
    }
    offset
}

pub(crate) fn has_host_origin(raw: &[u8]) -> Result<bool, serde_json::Error> {
    let mut offset = skip_space(raw, 0);
    if raw.get(offset) != Some(&b'[') {
        return Ok(false);
    }
    offset = skip_space(raw, offset + 1);
    if raw.get(offset) != Some(&b'{') {
        return Ok(false);
    }
    let mut origin = None::<String>;
    object_fields(raw, offset, |key, begin, _end| {
        if key.eq_ignore_ascii_case("origin")
            && let Some(value) = string(&raw[begin..value_end(raw, begin)])?
        {
            origin = Some(value);
        }
        Ok(())
    })?;
    Ok(origin.as_deref() == Some("host"))
}
