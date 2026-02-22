use std::collections::HashMap;

use anyhow::{Context, Result, bail};

/// Metadata is a map of keys to lists of values.
/// Values are always stored as a list to handle duplicate keys
/// (e.g. the original A5 format repeats the `PAGE` key).
pub type Metadata = HashMap<String, Vec<String>>;

/// Extension trait for convenient access to metadata values.
pub trait MetadataExt {
    /// Get the first value for a key, or None if the key is absent.
    fn get_str(&self, key: &str) -> Option<&str>;

    /// Parse the first value for a key as a usize address.
    fn get_address(&self, key: &str) -> Result<usize>;

    /// Get all values for a key, or an empty slice if absent.
    fn get_all(&self, key: &str) -> &[String];
}

// Sentinel empty vec to return references to for missing keys.
const EMPTY: &Vec<String> = &Vec::new();

impl MetadataExt for Metadata {
    fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|v| v.first()).map(|s| s.as_str())
    }

    fn get_address(&self, key: &str) -> Result<usize> {
        let val = self
            .get_str(key)
            .with_context(|| format!("missing metadata key '{key}'"))?;
        val.parse::<usize>()
            .with_context(|| format!("invalid address for '{key}': {val}"))
    }

    fn get_all(&self, key: &str) -> &[String] {
        self.get(key).unwrap_or(EMPTY).as_slice()
    }
}

/// Parse a `<KEY:VALUE><KEY:VALUE>...` metadata string into a Metadata map.
pub fn parse_metadata_string(text: &str) -> Metadata {
    let mut map: Metadata = HashMap::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Find opening '<'
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        i += 1; // skip '<'

        // Read key until ':' (no '<', '>', ':' allowed in key)
        let key_start = i;
        while i < len && bytes[i] != b':' && bytes[i] != b'<' && bytes[i] != b'>' {
            i += 1;
        }
        if i >= len || bytes[i] != b':' || i == key_start {
            continue; // no ':' found or empty key
        }
        let key = &text[key_start..i];
        i += 1; // skip ':'

        // Read value until '>'
        let value_start = i;
        while i < len && bytes[i] != b'>' {
            i += 1;
        }
        if i >= len {
            break; // no closing '>'
        }
        let value = &text[value_start..i];
        i += 1; // skip '>'

        map.entry(key.to_string())
            .or_default()
            .push(value.to_string());
    }

    map
}

/// Read a length-prefixed metadata block at the given offset.
/// Returns the parsed Metadata and the number of bytes consumed (4 + block_length).
pub fn read_metadata_block(data: &[u8], offset: usize) -> Result<Metadata> {
    if offset + 4 > data.len() {
        bail!(
            "metadata block at offset {offset}: not enough data for length prefix (file size: {})",
            data.len()
        );
    }
    let block_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
    let start = offset + 4;
    let end = start + block_len;
    if end > data.len() {
        bail!(
            "metadata block at offset {offset}: block length {block_len} exceeds file size {}",
            data.len()
        );
    }
    let text = std::str::from_utf8(&data[start..end])
        .with_context(|| format!("metadata block at offset {offset}: invalid UTF-8"))?;
    Ok(parse_metadata_string(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_metadata() {
        let text = "<FILE_TYPE:note><DEVICE_DPI:226>";
        let meta = parse_metadata_string(text);
        assert_eq!(meta.get_str("FILE_TYPE"), Some("note"));
        assert_eq!(meta.get_str("DEVICE_DPI"), Some("226"));
        assert_eq!(meta.get_str("NONEXISTENT"), None);
    }

    #[test]
    fn parse_duplicate_keys() {
        let text = "<PAGE:1048><PAGE:5672><COVER_0:0>";
        let meta = parse_metadata_string(text);
        assert_eq!(meta.get_all("PAGE"), &["1048", "5672"]);
        assert_eq!(meta.get_str("PAGE"), Some("1048"));
        assert_eq!(meta.get_str("COVER_0"), Some("0"));
    }

    #[test]
    fn parse_empty_and_none_values() {
        let text = "<KEY1:><KEY2:none><KEY3:0>";
        let meta = parse_metadata_string(text);
        assert_eq!(meta.get_str("KEY1"), Some(""));
        assert_eq!(meta.get_str("KEY2"), Some("none"));
        assert_eq!(meta.get_str("KEY3"), Some("0"));
    }

    #[test]
    fn get_address_parsing() {
        let text = "<FILE_FEATURE:24><DIRTY:5>";
        let meta = parse_metadata_string(text);
        assert_eq!(meta.get_address("FILE_FEATURE").unwrap(), 24);
        assert_eq!(meta.get_address("DIRTY").unwrap(), 5);
        assert!(meta.get_address("MISSING").is_err());
    }

    #[test]
    fn get_all_missing_key() {
        let meta = parse_metadata_string("<A:1>");
        let empty: &[String] = &[];
        assert_eq!(meta.get_all("MISSING"), empty);
    }

    #[test]
    fn read_metadata_block_roundtrip() {
        let text = b"<FILE_TYPE:note><DPI:226>";
        let len = text.len() as u32;
        let mut data = Vec::new();
        // some padding before the block
        data.extend_from_slice(&[0u8; 10]);
        data.extend_from_slice(&len.to_le_bytes());
        data.extend_from_slice(text);
        // Don't have any trailing junk, to check we are not going over
        // the limits.

        let meta = read_metadata_block(&data, 10).unwrap();
        assert_eq!(meta.get_str("FILE_TYPE"), Some("note"));
        assert_eq!(meta.get_str("DPI"), Some("226"));
    }

    #[test]
    fn read_metadata_block_truncated() {
        let data = [0u8; 3]; // too short for length prefix
        assert!(read_metadata_block(&data, 0).is_err());
    }

    #[test]
    fn read_metadata_block_of_size_zero() {
        let data = [0u8; 4]; // Just the length is here with a size zero
        let meta = read_metadata_block(&data, 0).unwrap();
        assert_eq!(meta.get_str("DPI"), None);
    }

    #[test]
    fn read_metadata_block_length_exceeds() {
        let mut data = Vec::new();
        data.extend_from_slice(&20u32.to_le_bytes()); // claims 20 bytes
        data.extend_from_slice(&[0u8; 19]); // only 19 bytes of content
        assert!(read_metadata_block(&data, 0).is_err());
    }
}
