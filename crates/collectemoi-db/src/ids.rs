//! Validation helpers for identifiers that may come from HTTP path/query input.
//! Never concatenate these into SQL — always bind as parameters after validation.

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum IdError {
    #[error("invalid id: empty")]
    Empty,
    #[error("invalid id: path characters not allowed")]
    PathChars,
    #[error("invalid id: must be a UUID")]
    NotUuid,
    #[error("invalid id: too long")]
    TooLong,
}

/// Reject path traversal / separators before any filesystem join.
pub fn reject_path_chars(id: &str) -> Result<(), IdError> {
    if id.is_empty() {
        return Err(IdError::Empty);
    }
    if id.len() > 128 {
        return Err(IdError::TooLong);
    }
    if id.contains("..")
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
        || id.contains('\n')
        || id.contains('\r')
    {
        return Err(IdError::PathChars);
    }
    Ok(())
}

/// Blob / upload / device path ids must be UUIDs (canonical lowercase hyphenated).
pub fn parse_uuid_id(id: &str) -> Result<Uuid, IdError> {
    reject_path_chars(id)?;
    Uuid::parse_str(id.trim()).map_err(|_| IdError::NotUuid)
}

/// Sanitize a filename for Content-Disposition: strip path segments and quotes.
pub fn safe_filename(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .chars()
        .map(|c| match c {
            '"' | '\n' | '\r' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>();
    if base.is_empty() || base == "." || base == ".." {
        "download.bin".into()
    } else {
        base.chars().take(200).collect()
    }
}

/// Escape `%` / `_` for use inside a SQL `LIKE` pattern bound as a parameter.
pub fn like_contains(needle: &str) -> String {
    let mut out = String::from('%');
    for c in needle.chars() {
        match c {
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('%');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal() {
        assert!(reject_path_chars("../etc/passwd").is_err());
        assert!(reject_path_chars("a/b").is_err());
        assert!(parse_uuid_id("not-a-uuid").is_err());
        assert!(parse_uuid_id("00000000-0000-4000-8000-000000000001").is_ok());
    }

    #[test]
    fn filename_sanitizes() {
        assert_eq!(safe_filename("../../evil\"x.txt"), "evil_x.txt");
        assert_eq!(safe_filename(""), "download.bin");
    }
}
