//! userland/desktop/apps/nova-settings/src/search.rs
//!
//! Real-Time Instant Search Engine for NovaOS Unified System Settings.
//! Provides inverted index scoring, keyword indexing, and deep-link breadcrumb navigation.

#[cfg(not(test))]
use alloc::{
    collections::BTreeMap,
    format,
    string::String,
    vec::Vec,
};

#[cfg(test)]
use std::{
    collections::BTreeMap,
    format,
    string::String,
    vec::Vec,
};

use crate::types::{PanelCategory, SearchResult, SettingItem};

/// Index entry referencing a setting toggle/slider/control.
#[derive(Clone, Debug)]
pub struct IndexedSetting {
    pub key: String,
    pub category: PanelCategory,
    pub subgroup: String,
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
}

/// Real-time search engine indexing all settings toggles.
#[derive(Clone, Debug, Default)]
pub struct SettingsSearchEngine {
    entries: Vec<IndexedSetting>,
}

impl SettingsSearchEngine {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Rebuilds the search index from the current backend settings store.
    pub fn rebuild_index(&mut self, items: &BTreeMap<String, SettingItem>) {
        self.entries.clear();
        for item in items.values() {
            self.entries.push(IndexedSetting {
                key: item.key.clone(),
                category: item.category,
                subgroup: item.subgroup.clone(),
                title: item.title.clone(),
                description: item.description.clone(),
                keywords: item.keywords.clone(),
            });
        }
    }

    /// Executes real-time instant search for the given query string.
    pub fn search(&self, query: &str) -> Vec<SearchResult> {
        let clean_query = query.trim().to_lowercase();
        if clean_query.is_empty() {
            return Vec::new();
        }

        let query_tokens: Vec<&str> = clean_query.split_whitespace().collect();
        let mut results = Vec::new();

        for entry in &self.entries {
            let mut total_score = 0u32;
            let title_lower = entry.title.to_lowercase();
            let desc_lower = entry.description.to_lowercase();
            let subgroup_lower = entry.subgroup.to_lowercase();
            let cat_lower = entry.category.title().to_lowercase();

            for token in &query_tokens {
                let mut token_matched = false;

                // 1. Exact match on title or keywords
                if title_lower == *token {
                    total_score += 120;
                    token_matched = true;
                } else if title_lower.starts_with(token) {
                    total_score += 80;
                    token_matched = true;
                } else if title_lower.contains(token) {
                    total_score += 50;
                    token_matched = true;
                }

                // 2. Keyword exact/prefix match
                for kw in &entry.keywords {
                    let kw_lower = kw.to_lowercase();
                    if kw_lower == *token {
                        total_score += 70;
                        token_matched = true;
                    } else if kw_lower.starts_with(token) {
                        total_score += 45;
                        token_matched = true;
                    } else if kw_lower.contains(token) {
                        total_score += 25;
                        token_matched = true;
                    }
                }

                // 3. Subgroup / Category match
                if subgroup_lower.contains(token) {
                    total_score += 35;
                    token_matched = true;
                }
                if cat_lower.contains(token) {
                    total_score += 25;
                    token_matched = true;
                }

                // 4. Description match
                if desc_lower.contains(token) {
                    total_score += 20;
                    token_matched = true;
                }

                if !token_matched {
                    // All tokens should match somewhere
                    total_score = 0;
                    break;
                }
            }

            if total_score > 0 {
                let breadcrumb = format!(
                    "{} > {}",
                    entry.category.title(),
                    entry.subgroup
                );
                results.push(SearchResult {
                    item_key: entry.key.clone(),
                    category: entry.category,
                    subgroup: entry.subgroup.clone(),
                    title: entry.title.clone(),
                    description: entry.description.clone(),
                    breadcrumb,
                    score: total_score,
                });
            }
        }

        // Sort descending by score
        results.sort_by(|a, b| b.score.cmp(&a.score));
        results
    }
}
