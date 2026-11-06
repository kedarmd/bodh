//! User navigation preferences. Review payloads remain in the preparation/workspace store.
use bodh::Revision;
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceTab {
    Overview,
    #[serde(alias = "Discussions")]
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
    #[serde(default)]
    pub selected_file: Option<String>,
    #[serde(default)]
    pub source_revision: Option<Revision>,
    #[serde(default)]
    pub split_diff: bool,
    #[serde(default)]
    pub file_scroll_y: f32,
    #[serde(default)]
    pub diff_scroll_y: f32,
    #[serde(default)]
    pub source_scroll_y: f32,
    #[serde(default)]
    pub description_scroll_y: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NavigationPreferences {
    pub dark: bool,
    pub inspector_width: f32,
    pub active_tab: Option<WorkspaceTab>,
    pub repositories: Vec<String>,
    pub recent: Vec<RecentReview>,
}

impl NavigationPreferences {
    /// Returns the newest saved preparation for a repository PR, if present.
    pub fn recent_review_for_pr(
        &self,
        repository_path: &str,
        number: u64,
    ) -> Option<&RecentReview> {
        self.recent
            .iter()
            .find(|review| review.repository_path == repository_path && review.number == number)
    }
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
    use bodh::Revision;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn repository_pr_lookup_returns_the_most_recent_saved_workspace() {
        let prefs = NavigationPreferences {
            recent: vec![
                RecentReview {
                    preparation_id: "newest".into(),
                    repository_path: "/repo".into(),
                    title: "A change".into(),
                    owner: "acme".into(),
                    repository: "app".into(),
                    number: 24,
                    pending_work: 0,
                    selected_file: Some("src/current.rs".into()),
                    source_revision: Some(Revision::Base),
                    split_diff: true,
                    file_scroll_y: 120.,
                    diff_scroll_y: 240.,
                    source_scroll_y: 360.,
                    description_scroll_y: 480.,
                },
                RecentReview {
                    preparation_id: "other-repository".into(),
                    repository_path: "/other".into(),
                    title: "A change".into(),
                    owner: "acme".into(),
                    repository: "app".into(),
                    number: 24,
                    pending_work: 0,
                    selected_file: None,
                    source_revision: None,
                    split_diff: false,
                    file_scroll_y: 0.,
                    diff_scroll_y: 0.,
                    source_scroll_y: 0.,
                    description_scroll_y: 0.,
                },
                RecentReview {
                    preparation_id: "older".into(),
                    repository_path: "/repo".into(),
                    title: "A change".into(),
                    owner: "acme".into(),
                    repository: "app".into(),
                    number: 24,
                    pending_work: 0,
                    selected_file: None,
                    source_revision: None,
                    split_diff: false,
                    file_scroll_y: 0.,
                    diff_scroll_y: 0.,
                    source_scroll_y: 0.,
                    description_scroll_y: 0.,
                },
            ],
            ..NavigationPreferences::default()
        };

        assert_eq!(
            prefs
                .recent_review_for_pr("/repo", 24)
                .map(|review| review.preparation_id.as_str()),
            Some("newest")
        );
        assert!(prefs.recent_review_for_pr("/repo", 25).is_none());
        assert!(prefs.recent_review_for_pr("/missing", 24).is_none());
    }

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
                selected_file: Some("src/lib.rs".into()),
                source_revision: Some(Revision::Base),
                split_diff: true,
                file_scroll_y: 120.,
                diff_scroll_y: 240.,
                source_scroll_y: 360.,
                description_scroll_y: 480.,
            }],
        };
        prefs.save(&path).unwrap();
        let restored = NavigationPreferences::load(&path);
        assert!(!restored.dark);
        assert_eq!(restored.inspector_width, 512.);
        assert_eq!(restored.active_tab, Some(WorkspaceTab::Changes));
        assert_eq!(restored.repositories, ["/repo"]);
        assert_eq!(restored.recent[0].preparation_id, "opaque-1");
        assert_eq!(
            restored.recent[0].selected_file.as_deref(),
            Some("src/lib.rs")
        );
        assert_eq!(restored.recent[0].source_revision, Some(Revision::Base));
        assert!(restored.recent[0].split_diff);
        assert_eq!(restored.recent[0].file_scroll_y, 120.);
        assert_eq!(restored.recent[0].diff_scroll_y, 240.);
        assert_eq!(restored.recent[0].source_scroll_y, 360.);
        assert_eq!(restored.recent[0].description_scroll_y, 480.);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(value.get("review_result").is_none());
        assert!(value.get("drafts").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn old_navigation_preferences_load_with_default_review_positions() {
        let path = std::env::temp_dir().join(format!(
            "bodh-old-prefs-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, r#"{"dark":true,"inspector_width":420,"active_tab":"Changes","repositories":[],"recent":[{"preparation_id":"p1","repository_path":"/repo","title":"T","owner":"a","repository":"b","number":1,"pending_work":0}]}"#).unwrap();
        let restored = NavigationPreferences::load(&path);
        assert_eq!(restored.recent.len(), 1);
        assert_eq!(restored.recent[0].selected_file, None);
        assert!(!restored.recent[0].split_diff);
        assert_eq!(restored.recent[0].source_revision, None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn retention_cleanup_does_not_overwrite_concurrent_atomic_preference_saves() {
        let root = std::env::temp_dir().join(format!(
            "bodh-prefs-retention-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let prefs_path = root.join("prefs.json");
        let state_path = root.join("state.sqlite");
        drop(bodh::PreparationStore::open(&state_path).unwrap());
        NavigationPreferences::default().save(&prefs_path).unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let writer_barrier = barrier.clone();
        let writer_path = prefs_path.clone();
        let writer = std::thread::spawn(move || {
            writer_barrier.wait();
            for width in 400..420 {
                let prefs = NavigationPreferences {
                    inspector_width: width as f32,
                    ..NavigationPreferences::default()
                };
                prefs.save(&writer_path).unwrap();
            }
        });
        barrier.wait();
        for _ in 0..20 {
            bodh::enforce_retention(&state_path, bodh::RetentionPolicy::default(), &[]).unwrap();
        }
        writer.join().unwrap();
        let restored = NavigationPreferences::load(&prefs_path);
        assert_eq!(restored.inspector_width, 419.);
        let _ = std::fs::remove_dir_all(root);
    }
}
