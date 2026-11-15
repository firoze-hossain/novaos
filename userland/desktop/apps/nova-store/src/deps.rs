//! userland/desktop/apps/nova-store/src/deps.rs
//!
//! Dependency Resolution Engine and Reverse Dependency Graph for NovaOS Packages.

#[cfg(not(test))]
use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

#[cfg(test)]
use std::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::package::AppPackage;

/// Dependency resolver and package tree analyzer.
pub struct DependencyResolver;

impl DependencyResolver {
    /// Computes installation sequence (topological order) for a package and its dependencies.
    pub fn resolve_install_order(
        target_id: &str,
        available: &BTreeMap<String, AppPackage>,
    ) -> Result<Vec<String>, String> {
        let mut visited = Vec::new();
        let mut visiting = Vec::new();
        let mut order = Vec::new();

        Self::dfs_resolve(
            target_id,
            available,
            &mut visited,
            &mut visiting,
            &mut order,
        )?;

        Ok(order)
    }

    fn dfs_resolve(
        current_id: &str,
        available: &BTreeMap<String, AppPackage>,
        visited: &mut Vec<String>,
        visiting: &mut Vec<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if visiting.contains(&current_id.to_string()) {
            return Err(format!("Circular dependency detected involving: {}", current_id));
        }
        if visited.contains(&current_id.to_string()) {
            return Ok(());
        }

        visiting.push(current_id.to_string());

        let pkg = available
            .get(current_id)
            .ok_or_else(|| format!("Unsatisfied dependency: package '{}' not found", current_id))?;

        for dep in &pkg.dependencies {
            Self::dfs_resolve(dep, available, visited, visiting, order)?;
        }

        visiting.retain(|id| id != current_id);
        visited.push(current_id.to_string());
        order.push(current_id.to_string());

        Ok(())
    }

    /// Checks if a package can be safely uninstalled without breaking other installed packages.
    pub fn check_safe_to_remove(
        target_id: &str,
        installed: &BTreeMap<String, AppPackage>,
    ) -> Result<(), Vec<String>> {
        let mut blockers = Vec::new();

        for (id, pkg) in installed {
            if id == target_id {
                continue;
            }
            if pkg.dependencies.contains(&target_id.to_string()) {
                blockers.push(pkg.name.clone());
            }
        }

        if blockers.is_empty() {
            Ok(())
        } else {
            Err(blockers)
        }
    }
}
