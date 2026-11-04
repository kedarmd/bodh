//! Explicit known IDs and acquisition completeness; validation never fetches discussions.
use crate::ContractError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
pub const MAX_OBSERVED_THREADS: usize = 16_384;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionFailure {
    #[serde(deserialize_with = "crate::bounded_text")]
    pub connection: String,
    #[serde(deserialize_with = "crate::bounded_text")]
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionObservation {
    #[serde(deserialize_with = "thread_ids")]
    pub thread_ids: Vec<String>,
    pub complete: bool,
    #[serde(deserialize_with = "crate::bounded_list")]
    pub failures: Vec<DiscussionFailure>,
}
impl Default for DiscussionObservation {
    fn default() -> Self {
        Self {
            thread_ids: Vec::new(),
            complete: false,
            failures: vec![DiscussionFailure {
                connection: "threads".into(),
                reason: "Discussion context has not been acquired".into(),
            }],
        }
    }
}
fn thread_ids<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let ids = <Vec<String> as Deserialize>::deserialize(d)?;
    if ids.len() > MAX_OBSERVED_THREADS
        || ids
            .iter()
            .any(|id| id.chars().count() > crate::MAX_STRING_CHARS)
    {
        return Err(serde::de::Error::custom(
            "discussion IDs exceed supported bounds",
        ));
    }
    Ok(ids)
}
impl DiscussionObservation {
    pub(crate) fn validate(&self) -> Result<HashSet<&str>, ContractError> {
        let fail = |pointer: &str| {
            ContractError::new(
                "PREPARED_CONTEXT_INVALID",
                "Discussion observation needs unique real IDs and explicit consistent completeness details",
                pointer,
            )
        };
        if self.thread_ids.len() > MAX_OBSERVED_THREADS
            || self.failures.len() > crate::MAX_COLLECTION_ITEMS
            || self.complete != self.failures.is_empty()
        {
            return Err(fail("/discussions"));
        }
        let mut ids = HashSet::new();
        for (i, id) in self.thread_ids.iter().enumerate() {
            if id.trim().is_empty()
                || id.chars().count() > crate::MAX_STRING_CHARS
                || !ids.insert(id.as_str())
            {
                return Err(fail(&format!("/discussions/thread_ids/{i}")));
            }
        }
        for (i, failure) in self.failures.iter().enumerate() {
            if [&failure.connection, &failure.reason]
                .into_iter()
                .any(|s| s.trim().is_empty() || s.chars().count() > crate::MAX_STRING_CHARS)
            {
                return Err(fail(&format!("/discussions/failures/{i}")));
            }
        }
        Ok(ids)
    }
}
