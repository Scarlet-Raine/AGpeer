//! Minimal bencode reader used by the in-memory reference engine to parse
//! `.torrent` metainfo (name, file list, sizes, private flag).
//!
//! This is intentionally tiny and defensive: it never panics, rejects
//! malformed input, and is not part of the BitTorrent wire protocol.

use std::collections::HashMap;

/// A parsed bencode value.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Dict(HashMap<Vec<u8>, Value>),
}

impl Value {
    fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    fn as_dict(&self) -> Option<&HashMap<Vec<u8>, Value>> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
}

/// Torrent metainfo as extracted by the reference engine.
#[derive(Debug, Clone)]
pub(crate) struct TorrentInfo {
    /// Torrent display name (`info.name`).
    pub name: String,
    /// Files as `(relative path, length in bytes)`.
    pub files: Vec<(String, u64)>,
    /// Whether the torrent declares itself private (`info.private == 1`).
    pub private: bool,
}

/// Parse `.torrent` bytes and extract the fields the backend cares about.
/// Returns `None` for anything that is not valid torrent metainfo.
pub(crate) fn torrent_info(data: &[u8]) -> Option<TorrentInfo> {
    let root = parse(data).ok()?;
    let info = root.as_dict()?.get(b"info".as_slice())?.as_dict()?;

    let name = info
        .get(b"name".as_slice())
        .and_then(Value::as_bytes)
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_else(|| "untitled".to_string());

    let private = info
        .get(b"private".as_slice())
        .and_then(Value::as_int)
        .map(|i| i == 1)
        .unwrap_or(false);

    let files = if let Some(length) = info.get(b"length".as_slice()).and_then(Value::as_int) {
        let length = u64::try_from(length).ok()?;
        vec![(name.clone(), length)]
    } else if let Some(list) = info.get(b"files".as_slice()).and_then(Value::as_list) {
        let mut files = Vec::new();
        for entry in list {
            let Some(entry) = entry.as_dict() else {
                continue;
            };
            let Some(length) = entry.get(b"length".as_slice()).and_then(Value::as_int) else {
                continue;
            };
            let length = match u64::try_from(length) {
                Ok(l) => l,
                Err(_) => continue,
            };
            let Some(path) = entry.get(b"path".as_slice()).and_then(Value::as_list) else {
                continue;
            };
            let parts: Vec<String> = path
                .iter()
                .filter_map(Value::as_bytes)
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .collect();
            if parts.is_empty() {
                continue;
            }
            files.push((parts.join("/"), length));
        }
        files
    } else {
        Vec::new()
    };

    Some(TorrentInfo {
        name,
        files,
        private,
    })
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn value(&mut self) -> Result<Value, &'static str> {
        match self.data.get(self.pos).copied() {
            Some(b'i') => self.int(),
            Some(b'l') => self.list(),
            Some(b'd') => self.dict(),
            Some(b'0'..=b'9') => self.bytes(),
            Some(_) => Err("invalid value type"),
            None => Err("unexpected end of input"),
        }
    }

    fn int(&mut self) -> Result<Value, &'static str> {
        self.pos += 1;
        let start = self.pos;
        while let Some(b) = self.data.get(self.pos) {
            if *b == b'e' {
                break;
            }
            if !b.is_ascii_digit() && *b != b'-' {
                return Err("invalid integer");
            }
            self.pos += 1;
        }
        let end = self.pos;
        if self.data.get(end) != Some(&b'e') {
            return Err("unterminated integer");
        }
        self.pos += 1;
        let text = std::str::from_utf8(&self.data[start..end]).map_err(|_| "invalid integer")?;
        let n = text.parse::<i64>().map_err(|_| "invalid integer")?;
        Ok(Value::Int(n))
    }

    fn bytes(&mut self) -> Result<Value, &'static str> {
        let start = self.pos;
        while let Some(b) = self.data.get(self.pos) {
            if *b == b':' {
                break;
            }
            if !b.is_ascii_digit() {
                return Err("invalid length");
            }
            self.pos += 1;
        }
        let colon = self.pos;
        if self.data.get(colon) != Some(&b':') {
            return Err("unterminated length");
        }
        let text = std::str::from_utf8(&self.data[start..colon]).map_err(|_| "invalid length")?;
        let len = text.parse::<usize>().map_err(|_| "invalid length")?;
        self.pos = colon + 1;
        let end = self.pos.checked_add(len).ok_or("length overflow")?;
        if end > self.data.len() {
            return Err("length exceeds input");
        }
        let bytes = self.data[self.pos..end].to_vec();
        self.pos = end;
        Ok(Value::Bytes(bytes))
    }

    fn list(&mut self) -> Result<Value, &'static str> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            match self.data.get(self.pos) {
                Some(b'e') => {
                    self.pos += 1;
                    return Ok(Value::List(items));
                }
                Some(_) => items.push(self.value()?),
                None => return Err("unterminated list"),
            }
        }
    }

    fn dict(&mut self) -> Result<Value, &'static str> {
        self.pos += 1;
        let mut entries = HashMap::new();
        loop {
            match self.data.get(self.pos) {
                Some(b'e') => {
                    self.pos += 1;
                    return Ok(Value::Dict(entries));
                }
                Some(_) => {
                    let key = match self.value()? {
                        Value::Bytes(b) => b,
                        _ => return Err("non-bytes dictionary key"),
                    };
                    let value = self.value()?;
                    entries.insert(key, value);
                }
                None => return Err("unterminated dictionary"),
            }
        }
    }
}

/// Parse a complete bencode document. Fails on trailing garbage.
fn parse(data: &[u8]) -> Result<Value, &'static str> {
    let mut parser = Parser { data, pos: 0 };
    let value = parser.value()?;
    if parser.pos != data.len() {
        return Err("trailing data");
    }
    Ok(value)
}

#[cfg(test)]
pub(crate) mod test_helpers {
    /// `len:value` bencode byte string.
    pub(crate) fn bstr(s: &str) -> Vec<u8> {
        format!("{}:{s}", s.len()).into_bytes()
    }

    /// `i<value>e` bencode integer.
    pub(crate) fn bint(n: i64) -> Vec<u8> {
        format!("i{n}e").into_bytes()
    }

    /// `l<items>e` bencode list.
    pub(crate) fn blist(items: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![b'l'];
        for item in items {
            out.extend_from_slice(item);
        }
        out.push(b'e');
        out
    }

    /// `d<key><value>...e` bencode dictionary.
    pub(crate) fn bdict(pairs: &[(&[u8], Vec<u8>)]) -> Vec<u8> {
        let mut out = vec![b'd'];
        for (key, value) in pairs {
            out.extend_from_slice(&format!("{}:", key.len()).into_bytes());
            out.extend_from_slice(key);
            out.extend_from_slice(value);
        }
        out.push(b'e');
        out
    }

    /// Single-file torrent metainfo with an optional `private` flag.
    pub(crate) fn torrent_metainfo_single_file(name: &str, length: i64, private: bool) -> Vec<u8> {
        let info = bdict(&[
            (b"length", bint(length)),
            (b"name", bstr(name)),
            (b"private", bint(i64::from(private))),
        ]);
        bdict(&[(b"info", info)])
    }

    /// Multi-file torrent metainfo. Each entry is a single path component.
    pub(crate) fn torrent_metainfo_multi_file(files: &[(&str, i64)]) -> Vec<u8> {
        let list: Vec<Vec<u8>> = files
            .iter()
            .map(|(name, length)| {
                bdict(&[(b"length", bint(*length)), (b"path", blist(&[bstr(name)]))])
            })
            .collect();
        let info = bdict(&[(b"name", bstr("multi")), (b"files", blist(&list))]);
        bdict(&[(b"info", info)])
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::*;
    use super::*;

    #[test]
    fn parses_single_file_metainfo() {
        let data = torrent_metainfo_single_file("ubuntu.iso", 12345, true);
        let info = torrent_info(&data).unwrap();
        assert_eq!(info.name, "ubuntu.iso");
        assert_eq!(info.files, vec![("ubuntu.iso".to_string(), 12345)]);
        assert!(info.private);
    }

    #[test]
    fn parses_multi_file_metainfo() {
        let data = torrent_metainfo_multi_file(&[("a.txt", 10), ("b.txt", 20)]);
        let info = torrent_info(&data).unwrap();
        assert_eq!(info.name, "multi");
        assert_eq!(
            info.files,
            vec![("a.txt".to_string(), 10), ("b.txt".to_string(), 20)]
        );
        assert!(!info.private);
    }

    #[test]
    fn rejects_garbage() {
        assert!(torrent_info(b"hello").is_none());
        assert!(torrent_info(b"d4:infox").is_none());
        assert!(torrent_info(&[]).is_none());
    }

    #[test]
    fn trailing_data_is_rejected() {
        let mut data = torrent_metainfo_single_file("x", 1, false);
        data.extend_from_slice(b"junk");
        assert!(torrent_info(&data).is_none());
    }
}
