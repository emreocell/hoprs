use std::fmt;

/// Identifier of a configured remote site.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SiteId(pub String);

impl fmt::Display for SiteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
