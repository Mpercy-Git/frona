//! Single-range `Range: bytes=…` parsing for file downloads.
//!
//! Browsers need `206 Partial Content` to seek in `<audio>`/`<video>`: without
//! it a media element can only play straight through from byte zero, and
//! `preload="metadata"` can't work out the duration. Multi-range requests are
//! deliberately unsupported — RFC 9110 lets a server ignore a `Range` it
//! doesn't want to honour, and no browser needs multipart ranges for playback.

/// Resolved byte offsets, inclusive on both ends (as on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RangeSpec {
    /// No `Range` header, or one we don't honour: send the whole file with 200.
    Full,
    /// Send `206` with these offsets.
    Partial(ByteRange),
    /// Syntactically valid but outside the file: send `416`.
    Unsatisfiable,
}

/// Anything malformed resolves to [`RangeSpec::Full`] — RFC 9110 §14.2 requires
/// an invalid `Range` to be ignored rather than rejected.
pub(super) fn parse(header: Option<&str>, file_len: u64) -> RangeSpec {
    let Some(header) = header else {
        return RangeSpec::Full;
    };

    let Some(spec) = header
        .trim()
        .strip_prefix("bytes=")
        .or_else(|| header.trim().strip_prefix("BYTES="))
    else {
        return RangeSpec::Full;
    };

    // Multi-range ("bytes=0-9,20-29") would need a multipart/byteranges body.
    if spec.contains(',') {
        return RangeSpec::Full;
    }

    let spec = spec.trim();
    let Some((first, last)) = spec.split_once('-') else {
        return RangeSpec::Full;
    };
    let (first, last) = (first.trim(), last.trim());

    // A zero-length file can't satisfy any range.
    if file_len == 0 {
        return RangeSpec::Unsatisfiable;
    }
    let max = file_len - 1;

    match (first, last) {
        // "bytes=-500" — the final 500 bytes. A suffix of 0 bytes is
        // unsatisfiable; a suffix longer than the file clamps to the whole file.
        ("", suffix) => match suffix.parse::<u64>() {
            Ok(0) => RangeSpec::Unsatisfiable,
            Ok(n) => RangeSpec::Partial(ByteRange {
                start: file_len.saturating_sub(n),
                end: max,
            }),
            Err(_) => RangeSpec::Full,
        },
        // "bytes=500-" — from 500 to the end.
        (start, "") => match start.parse::<u64>() {
            Ok(start) if start > max => RangeSpec::Unsatisfiable,
            Ok(start) => RangeSpec::Partial(ByteRange { start, end: max }),
            Err(_) => RangeSpec::Full,
        },
        // "bytes=0-499" — an explicit window, clamped to the file's last byte.
        (start, end) => match (start.parse::<u64>(), end.parse::<u64>()) {
            (Ok(start), Ok(end)) if start > end || start > max => RangeSpec::Unsatisfiable,
            (Ok(start), Ok(end)) => RangeSpec::Partial(ByteRange {
                start,
                end: end.min(max),
            }),
            _ => RangeSpec::Full,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn partial(start: u64, end: u64) -> RangeSpec {
        RangeSpec::Partial(ByteRange { start, end })
    }

    #[test]
    fn no_header_serves_whole_file() {
        assert_eq!(parse(None, 1000), RangeSpec::Full);
    }

    #[test]
    fn explicit_window() {
        assert_eq!(parse(Some("bytes=0-499"), 1000), partial(0, 499));
        assert_eq!(parse(Some("bytes=500-999"), 1000), partial(500, 999));
        assert_eq!(parse(Some("bytes=0-0"), 1000), partial(0, 0));
    }

    #[test]
    fn open_ended_window_runs_to_last_byte() {
        assert_eq!(parse(Some("bytes=500-"), 1000), partial(500, 999));
        // The probe every media element opens with.
        assert_eq!(parse(Some("bytes=0-"), 1000), partial(0, 999));
    }

    #[test]
    fn suffix_window_counts_back_from_the_end() {
        assert_eq!(parse(Some("bytes=-500"), 1000), partial(500, 999));
        // Longer than the file: clamp rather than underflow.
        assert_eq!(parse(Some("bytes=-5000"), 1000), partial(0, 999));
    }

    #[test]
    fn end_past_eof_clamps() {
        assert_eq!(parse(Some("bytes=900-5000"), 1000), partial(900, 999));
    }

    #[test]
    fn unsatisfiable_ranges() {
        // Start at or past EOF.
        assert_eq!(parse(Some("bytes=1000-"), 1000), RangeSpec::Unsatisfiable);
        assert_eq!(parse(Some("bytes=1000-1500"), 1000), RangeSpec::Unsatisfiable);
        // Reversed bounds.
        assert_eq!(parse(Some("bytes=500-100"), 1000), RangeSpec::Unsatisfiable);
        // Zero-length suffix.
        assert_eq!(parse(Some("bytes=-0"), 1000), RangeSpec::Unsatisfiable);
        // Nothing satisfies a range over an empty file.
        assert_eq!(parse(Some("bytes=0-"), 0), RangeSpec::Unsatisfiable);
    }

    #[test]
    fn malformed_and_unsupported_are_ignored() {
        assert_eq!(parse(Some("bytes=abc-def"), 1000), RangeSpec::Full);
        assert_eq!(parse(Some("bytes="), 1000), RangeSpec::Full);
        assert_eq!(parse(Some("bytes=0"), 1000), RangeSpec::Full);
        // Units other than bytes.
        assert_eq!(parse(Some("items=0-10"), 1000), RangeSpec::Full);
        // Multi-range needs a multipart body; serve the whole file instead.
        assert_eq!(parse(Some("bytes=0-99,200-299"), 1000), RangeSpec::Full);
    }

    #[test]
    fn whitespace_and_case_tolerated() {
        assert_eq!(parse(Some(" bytes=0-99 "), 1000), partial(0, 99));
        assert_eq!(parse(Some("bytes= 0 - 99"), 1000), partial(0, 99));
        assert_eq!(parse(Some("BYTES=0-99"), 1000), partial(0, 99));
    }

    #[test]
    fn range_length_is_inclusive() {
        assert_eq!(ByteRange { start: 0, end: 0 }.len(), 1);
        assert_eq!(ByteRange { start: 0, end: 499 }.len(), 500);
        assert_eq!(ByteRange { start: 500, end: 999 }.len(), 500);
    }
}
