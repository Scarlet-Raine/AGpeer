//! Shared URL/percent-decoding and path helpers.

/// Platform-neutral "absolute path" check for user-supplied storage roots.
///
/// `std::path::Path::is_absolute` is platform-relative: `/mnt/user/media` is
/// absolute on Linux but not on a Windows build host, and `E:\Media` is
/// absolute on Windows only. agpeer's storage roots are interpreted by the
/// machine running the core (often a Linux container), so accept either form
/// regardless of the platform this binary was compiled on.
pub fn is_absolute_path(text: &str) -> bool {
    std::path::Path::new(text).is_absolute()
        || text.starts_with('/')
        || text.starts_with('\\')
        || (text
            .as_bytes()
            .first()
            .map(|b| b.is_ascii_alphabetic())
            .unwrap_or(false)
            && text.as_bytes().get(1) == Some(&b':'))
}

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

    #[test]
    fn absolute_path_accepts_posix_and_windows_forms() {
        assert!(is_absolute_path("/mnt/user/media"));
        assert!(is_absolute_path("E:\\Media"));
        assert!(is_absolute_path("E:/Media"));
        assert!(is_absolute_path("\\\\server\\share"));
        assert!(!is_absolute_path("relative/path"));
        assert!(!is_absolute_path("./here"));
        assert!(!is_absolute_path(""));
    }
}
