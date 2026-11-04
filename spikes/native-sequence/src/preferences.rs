//! User navigation preferences. Review payloads remain in the preparation/workspace store.
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceTab {
    Overview,
    Changes,
    Findings,
}
impl WorkspaceTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Changes => "Changes",
            Self::Findings => "Findings",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentReview {
    pub preparation_id: String,
    pub repository_path: String,
    pub title: String,
    pub owner: String,
    pub repository: String,
    pub number: u64,
    pub pending_work: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NavigationPreferences {
    pub dark: bool,
    pub inspector_width: f32,
    pub active_tab: Option<WorkspaceTab>,
    pub repositories: Vec<String>,
    pub recent: Vec<RecentReview>,
}
impl Default for NavigationPreferences {
    fn default() -> Self {
        Self {
            dark: true,
            inspector_width: 420.,
            active_tab: None,
            repositories: Vec::new(),
            recent: Vec::new(),
        }
    }
}

impl NavigationPreferences {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp = path.with_extension(format!("{}.{}.tmp", std::process::id(), suffix));
        std::fs::write(&temp, bytes)?;
        std::fs::rename(temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::{NavigationPreferences, RecentReview, WorkspaceTab};

    #[test]
    fn navigation_preferences_persist_without_review_content() {
        let root = std::env::temp_dir().join(format!(
            "bodh-prefs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("prefs.json");
        let prefs = NavigationPreferences {
            dark: false,
            inspector_width: 512.,
            active_tab: Some(WorkspaceTab::Changes),
            repositories: vec!["/repo".into()],
            recent: vec![RecentReview {
                preparation_id: "opaque-1".into(),
                repository_path: "/repo".into(),
                title: "A change".into(),
                owner: "acme".into(),
                repository: "app".into(),
                number: 24,
                pending_work: 0,
            }],
        };
        prefs.save(&path).unwrap();
        let restored = NavigationPreferences::load(&path);
        assert!(!restored.dark);
        assert_eq!(restored.inspector_width, 512.);
        assert_eq!(restored.active_tab, Some(WorkspaceTab::Changes));
        assert_eq!(restored.repositories, ["/repo"]);
        assert_eq!(restored.recent[0].preparation_id, "opaque-1");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(value.get("review_result").is_none());
        assert!(value.get("drafts").is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
