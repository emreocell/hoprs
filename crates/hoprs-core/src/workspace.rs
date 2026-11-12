use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{SiteId, WorkspaceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TabKind {
    Terminal,
    Files,
}

/// A tab restored when a workspace is opened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab {
    pub kind: TabKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSite {
    pub site: SiteId,
    #[serde(default)]
    pub tabs: Vec<SavedTab>,
}

/// A named bundle of sites, opened together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    #[serde(default)]
    pub sites: Vec<WorkspaceSite>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_roundtrip() {
        let ws = Workspace {
            id: WorkspaceId::new(),
            name: "Prod".into(),
            sites: vec![
                WorkspaceSite {
                    site: SiteId::new(),
                    tabs: vec![
                        SavedTab {
                            kind: TabKind::Terminal,
                            remote_dir: None,
                            local_dir: None,
                        },
                        SavedTab {
                            kind: TabKind::Files,
                            remote_dir: Some("/var/www".into()),
                            local_dir: Some(PathBuf::from("site")),
                        },
                    ],
                },
                WorkspaceSite {
                    site: SiteId::new(),
                    tabs: vec![],
                },
            ],
        };
        let text = toml::to_string(&ws).unwrap();
        assert_eq!(toml::from_str::<Workspace>(&text).unwrap(), ws);
    }
}
