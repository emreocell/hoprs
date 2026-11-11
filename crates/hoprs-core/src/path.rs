use std::fmt;

/// Error produced when building a [`RemotePath`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("path is not absolute")]
    NotAbsolute,
    #[error("path escapes the root directory")]
    EscapesRoot,
}

/// An absolute, normalized POSIX path on a remote host.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RemotePath(String);

impl RemotePath {
    /// Parses an absolute POSIX path, collapsing `//`, `.`, `..` and the trailing slash.
    pub fn new(s: &str) -> Result<Self, PathError> {
        if !s.starts_with('/') {
            return Err(PathError::NotAbsolute);
        }
        let mut parts: Vec<&str> = Vec::new();
        for comp in s.split('/') {
            match comp {
                "" | "." => {}
                ".." => {
                    parts.pop().ok_or(PathError::EscapesRoot)?;
                }
                c => parts.push(c),
            }
        }
        if parts.is_empty() {
            return Ok(Self::root());
        }
        Ok(Self(format!("/{}", parts.join("/"))))
    }

    pub fn root() -> Self {
        Self("/".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Appends a single component, taken verbatim.
    ///
    /// # Panics
    /// Panics if `name` is empty, contains `/`, or is `.` or `..`.
    pub fn join(&self, name: &str) -> Self {
        assert!(
            !name.is_empty() && !name.contains('/') && name != "." && name != "..",
            "RemotePath::join expects a single plain component, got {name:?}"
        );
        if self.0 == "/" {
            Self(format!("/{name}"))
        } else {
            Self(format!("{}/{name}", self.0))
        }
    }

    /// Returns the parent directory, or `None` at the root.
    pub fn parent(&self) -> Option<Self> {
        if self.0 == "/" {
            return None;
        }
        let idx = self.0.rfind('/')?;
        Some(if idx == 0 {
            Self::root()
        } else {
            Self(self.0[..idx].to_owned())
        })
    }

    /// Returns the last component, or `None` at the root.
    pub fn file_name(&self) -> Option<&str> {
        if self.0 == "/" {
            return None;
        }
        self.0.rfind('/').map(|i| &self.0[i + 1..])
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(
            RemotePath::new("//var//www/./site/").unwrap().as_str(),
            "/var/www/site"
        );
        assert_eq!(RemotePath::new("/a/b/../c").unwrap().as_str(), "/a/c");
        assert_eq!(RemotePath::new("/").unwrap().as_str(), "/");
    }
    #[test]
    fn rejects_relative_and_escape() {
        assert_eq!(RemotePath::new("var/www"), Err(PathError::NotAbsolute));
        assert_eq!(RemotePath::new("/.."), Err(PathError::EscapesRoot));
    }
    #[test]
    fn join_parent_file_name() {
        let p = RemotePath::root().join("var").join("relatório final.pdf");
        assert_eq!(p.as_str(), "/var/relatório final.pdf");
        assert_eq!(p.file_name(), Some("relatório final.pdf"));
        assert_eq!(p.parent().unwrap().as_str(), "/var");
        assert_eq!(RemotePath::root().parent(), None);
        assert_eq!(RemotePath::root().join(".env").as_str(), "/.env");
    }
    #[test]
    #[should_panic]
    fn join_rejects_separator() {
        RemotePath::root().join("a/b");
    }
}
