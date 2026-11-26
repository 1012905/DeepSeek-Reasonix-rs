// Copyright 2010, 2024 The Go Authors. All rights reserved.
// Use of this source code is governed by a BSD-style
// license that can be found in the NOTICE file.

use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

type CleanUnit = u8;

#[cfg(windows)]
const SEPARATOR: CleanUnit = b'\\' as CleanUnit;
#[cfg(not(windows))]
const SEPARATOR: CleanUnit = b'/' as CleanUnit;
const DOT: CleanUnit = b'.' as CleanUnit;

pub(super) fn lexical_clean(path: &Path) -> PathBuf {
    // Go indexes Windows paths as UTF-8 bytes; Windows native strings expose WTF-8 here.
    let original = path.as_os_str().as_encoded_bytes().to_vec();
    #[cfg(windows)]
    let volume_len = windows_volume_name_len(&original);
    #[cfg(not(windows))]
    let volume_len = 0;
    if original.len() == volume_len {
        #[cfg(windows)]
        if volume_len > 1 && is_path_separator(original[0]) && is_path_separator(original[1]) {
            let original = windows_from_slash(original);
            return os_string_from_encoded_units(original).into();
        }
        let mut clean = original;
        clean.push(DOT);
        return os_string_from_encoded_units(clean).into();
    }

    let rest = original[volume_len..].to_vec();
    let rooted = is_path_separator(rest[0]);
    let mut out = LazyBuf::new(original, volume_len);
    let mut read = 0;
    let mut dotdot = 0;
    if rooted {
        out.append(SEPARATOR);
        read = 1;
        dotdot = 1;
    }

    while read < rest.len() {
        if is_path_separator(rest[read])
            || (rest[read] == DOT && (read + 1 == rest.len() || is_path_separator(rest[read + 1])))
        {
            read += 1;
        } else if rest[read] == DOT
            && read + 1 < rest.len()
            && rest[read + 1] == DOT
            && (read + 2 == rest.len() || is_path_separator(rest[read + 2]))
        {
            read += 2;
            if out.written > dotdot {
                out.written -= 1;
                while out.written > dotdot && !is_path_separator(out.index(out.written)) {
                    out.written -= 1;
                }
            } else if !rooted {
                if out.written > 0 {
                    out.append(SEPARATOR);
                }
                out.append(DOT);
                out.append(DOT);
                dotdot = out.written;
            }
        } else {
            if (rooted && out.written != 1) || (!rooted && out.written != 0) {
                out.append(SEPARATOR);
            }
            while read < rest.len() && !is_path_separator(rest[read]) {
                out.append(rest[read]);
                read += 1;
            }
        }
    }
    if out.written == 0 {
        out.append(DOT);
    }
    #[cfg(windows)]
    windows_post_clean(&mut out);
    let clean = out.into_units();
    #[cfg(windows)]
    let clean = windows_from_slash(clean);
    os_string_from_encoded_units(clean).into()
}

struct LazyBuf {
    original: Vec<CleanUnit>,
    volume_len: usize,
    buf: Option<Vec<CleanUnit>>,
    written: usize,
}

impl LazyBuf {
    fn new(original: Vec<CleanUnit>, volume_len: usize) -> Self {
        Self {
            original,
            volume_len,
            buf: None,
            written: 0,
        }
    }

    fn index(&self, index: usize) -> CleanUnit {
        match &self.buf {
            Some(buf) => buf[index],
            None => self.original[self.volume_len + index],
        }
    }

    fn append(&mut self, unit: CleanUnit) {
        if self.buf.is_none() {
            let index = self.volume_len + self.written;
            if index < self.original.len() && self.original[index] == unit {
                self.written += 1;
                return;
            }
            let mut buf = vec![0; self.original.len() - self.volume_len];
            buf[..self.written]
                .copy_from_slice(&self.original[self.volume_len..self.volume_len + self.written]);
            self.buf = Some(buf);
        }
        let buf = self.buf.as_mut().expect("the lazy buffer is allocated");
        buf[self.written] = unit;
        self.written += 1;
    }

    #[cfg(windows)]
    fn prepend(&mut self, prefix: &[CleanUnit]) {
        let buf = self
            .buf
            .as_mut()
            .expect("postClean only prepends to a buffer");
        drop(buf.splice(0..0, prefix.iter().copied()));
        self.written += prefix.len();
    }

    fn into_units(self) -> Vec<CleanUnit> {
        let Self {
            mut original,
            volume_len,
            buf,
            written,
        } = self;
        if let Some(buf) = buf {
            original.truncate(volume_len);
            original.extend_from_slice(&buf[..written]);
            original
        } else {
            original.truncate(volume_len + written);
            original
        }
    }
}

fn os_string_from_encoded_units(units: Vec<CleanUnit>) -> OsString {
    #[cfg(unix)]
    {
        OsString::from_vec(units)
    }
    #[cfg(windows)]
    {
        let mut wide = Vec::with_capacity(units.len());
        let mut index = 0;
        while index < units.len() {
            let first = units[index];
            let (width, scalar) = match first {
                0x00..=0x7f => (1, u32::from(first)),
                0xc2..=0xdf => (
                    2,
                    (u32::from(first & 0x1f) << 6) | u32::from(units[index + 1] & 0x3f),
                ),
                0xe0..=0xef => (
                    3,
                    (u32::from(first & 0x0f) << 12)
                        | (u32::from(units[index + 1] & 0x3f) << 6)
                        | u32::from(units[index + 2] & 0x3f),
                ),
                0xf0..=0xf4 => (
                    4,
                    (u32::from(first & 0x07) << 18)
                        | (u32::from(units[index + 1] & 0x3f) << 12)
                        | (u32::from(units[index + 2] & 0x3f) << 6)
                        | u32::from(units[index + 3] & 0x3f),
                ),
                _ => unreachable!("native paths are encoded as WTF-8"),
            };
            if scalar <= 0xffff {
                wide.push(scalar as u16);
            } else {
                let supplementary = scalar - 0x10000;
                wide.push(0xd800 | (supplementary >> 10) as u16);
                wide.push(0xdc00 | (supplementary & 0x3ff) as u16);
            }
            index += width;
        }
        OsString::from_wide(&wide)
    }
}

#[cfg(not(windows))]
fn is_path_separator(unit: CleanUnit) -> bool {
    unit == SEPARATOR
}

#[cfg(windows)]
fn is_path_separator(unit: CleanUnit) -> bool {
    unit == SEPARATOR || unit == b'/' as CleanUnit
}

#[cfg(windows)]
fn windows_from_slash(mut units: Vec<CleanUnit>) -> Vec<CleanUnit> {
    for unit in &mut units {
        if *unit == b'/' as CleanUnit {
            *unit = SEPARATOR;
        }
    }
    units
}

#[cfg(windows)]
fn windows_post_clean(out: &mut LazyBuf) {
    if out.volume_len != 0 || out.buf.is_none() {
        return;
    }
    for &unit in out.buf.as_ref().expect("the lazy buffer is allocated") {
        if is_path_separator(unit) {
            break;
        }
        if unit == b':' as CleanUnit {
            out.prepend(&[DOT, SEPARATOR]);
            return;
        }
    }
    let buf = out.buf.as_ref().expect("the lazy buffer is allocated");
    if buf.len() >= 3
        && is_path_separator(buf[0])
        && buf[1] == b'?' as CleanUnit
        && buf[2] == b'?' as CleanUnit
    {
        out.prepend(&[SEPARATOR, DOT]);
    }
}

#[cfg(windows)]
fn windows_volume_name_len(path: &[CleanUnit]) -> usize {
    if path.len() >= 2 && path[1] == b':' as CleanUnit {
        return 2;
    }
    if path.is_empty() || !is_path_separator(path[0]) {
        return 0;
    }
    if windows_has_prefix_fold(path, br"\\.\UNC") {
        return windows_unc_len(path, 8);
    }
    if windows_has_prefix_fold(path, br"\\.")
        || windows_has_prefix_fold(path, br"\\?")
        || windows_has_prefix_fold(path, br"\??")
    {
        if path.len() == 3 {
            return 3;
        }
        return path[4..]
            .iter()
            .position(|unit| is_path_separator(*unit))
            .map_or(path.len(), |index| index + 4);
    }
    if path.len() >= 2 && is_path_separator(path[1]) {
        return windows_unc_len(path, 2);
    }
    0
}

#[cfg(windows)]
fn windows_has_prefix_fold(path: &[CleanUnit], prefix: &[u8]) -> bool {
    if path.len() < prefix.len() {
        return false;
    }
    for (index, &expected) in prefix.iter().enumerate() {
        let actual = path[index];
        let matches = if is_path_separator(expected as CleanUnit) {
            is_path_separator(actual)
        } else {
            actual.eq_ignore_ascii_case(&expected)
        };
        if !matches {
            return false;
        }
    }
    path.len() == prefix.len() || is_path_separator(path[prefix.len()])
}

#[cfg(windows)]
fn windows_unc_len(path: &[CleanUnit], prefix_len: usize) -> usize {
    let mut separators = 0;
    for (index, &unit) in path.iter().enumerate().skip(prefix_len) {
        if is_path_separator(unit) {
            separators += 1;
            if separators == 2 {
                return index;
            }
        }
    }
    path.len()
}
