//! Shared URL/percent-decoding helpers.

/// Percent-decode a magnet display name (`dn=`).
///
/// A literal `+` is decoded to a space, matching URL query-string semantics.
/// This single implementation is used by both the torrent and the magnet
/// search (hook) crates so the same magnet renders the same display name
/// everywhere — keep acceptance/decoding rules in sync here.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                if let Ok(b) = u8::try_from(hi * 16 + lo) {
                    out.push(if b == b'+' { b' ' } else { b });
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_and_plus() {
        assert_eq!(percent_decode("My%20File"), "My File");
        assert_eq!(percent_decode("Ubuntu+22.04"), "Ubuntu 22.04");
        assert_eq!(percent_decode("a+b%2Bc"), "a b c");
    }

    #[test]
    fn keeps_unencoded_plus_consistent() {
        // The same `dn=` renders identically here and in the torrent crate:
        // `+` always means a space.
        assert_eq!(percent_decode("Ubuntu+22.04"), "Ubuntu 22.04");
    }
}
