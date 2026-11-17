//! userland/server/novad/src/graph.rs
//!
//! Dependency Graph Resolution & Parallel Startup Engine for novad:
//! - Directed Acyclic Graph (DAG) construction
//! - Cycle detection & breaking for ordering constraints (After=, Before=)
//! - Parallel execution waves generation (deterministic concurrent startup)
//! - Graceful shutdown sequencing (exact reverse topological order)
//! - Requirement & failure propagation (Requires=, Wants=, BindsTo=, Conflicts=)

use std::collections::{HashMap, HashSet, VecDeque};
use crate::unit::{UnitFile, UnitState};

#[derive(Clone, Debug, Default)]
pub struct DependencyGraph {
    /// Mapping from unit name to its UnitFile definition.
    pub units: HashMap<String, UnitFile>,
    /// Forward ordering: A -> [B] means A must start AFTER B has started.
    pub after_deps: HashMap<String, HashSet<String>>,
    /// Forward ordering: A -> [B] means A must start BEFORE B starts.
    pub before_deps: HashMap<String, HashSet<String>>,
    /// Hard requirements: A requires B to be active.
    pub requires: HashMap<String, HashSet<String>>,
    /// Soft requirements: A wants B to be active.
    pub wants: HashMap<String, HashSet<String>>,
    /// Bound lifecycles: if B dies, A must be stopped.
    pub binds_to: HashMap<String, HashSet<String>>,
    /// Mutual exclusions: A cannot run concurrently with B.
    pub conflicts: HashMap<String, HashSet<String>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        DependencyGraph::default()
    }

    /// Registers a unit into the dependency graph.
    pub fn add_unit(&mut self, name: &str, unit_file: UnitFile) {
        self.units.insert(name.to_string(), unit_file);
    }

    /// Builds all relationship indices from registered units.
    pub fn compile(&mut self) {
        self.after_deps.clear();
        self.before_deps.clear();
        self.requires.clear();
        self.wants.clear();
        self.binds_to.clear();
        self.conflicts.clear();

        // 1. First pass: load explicit unit section relationships
        for (name, unit) in &self.units {
            // Requires
            for req in &unit.unit.requires {
                self.requires.entry(name.clone()).or_default().insert(req.clone());
                // Implicit After if Requires is present and no explicit Before
                self.after_deps.entry(name.clone()).or_default().insert(req.clone());
            }

            // Wants
            for want in &unit.unit.wants {
                self.wants.entry(name.clone()).or_default().insert(want.clone());
            }

            // BindsTo
            for b in &unit.unit.binds_to {
                self.binds_to.entry(name.clone()).or_default().insert(b.clone());
                self.requires.entry(name.clone()).or_default().insert(b.clone());
                self.after_deps.entry(name.clone()).or_default().insert(b.clone());
            }

            // Conflicts
            for c in &unit.unit.conflicts {
                self.conflicts.entry(name.clone()).or_default().insert(c.clone());
                self.conflicts.entry(c.clone()).or_default().insert(name.clone());
            }

            // After
            for a in &unit.unit.after {
                self.after_deps.entry(name.clone()).or_default().insert(a.clone());
            }

            // Before
            for b in &unit.unit.before {
                self.before_deps.entry(name.clone()).or_default().insert(b.clone());
                // Symmetric mapping: Before=b means b has After=name
                self.after_deps.entry(b.clone()).or_default().insert(name.clone());
            }
        }

        // 2. Second pass: link sockets to services
        for (name, unit) in &self.units {
            if let Some(sock) = &unit.socket {
                let target_svc = sock.service.clone().unwrap_or_else(|| {
                    if name.ends_with(".socket") {
                        format!("{}.service", &name[..name.len() - 7])
                    } else {
                        format!("{}.service", name)
                    }
                });
                // Service requires and runs after socket
                self.after_deps.entry(target_svc.clone()).or_default().insert(name.clone());
                self.wants.entry(target_svc.clone()).or_default().insert(name.clone());
            }
        }

        // 3. Third pass: process [Install] WantedBy / RequiredBy
        for (name, unit) in &self.units {
            for target in &unit.install.wanted_by {
                self.wants.entry(target.clone()).or_default().insert(name.clone());
            }
            for target in &unit.install.required_by {
                self.requires.entry(target.clone()).or_default().insert(name.clone());
                self.after_deps.entry(target.clone()).or_default().insert(name.clone());
            }
        }

        // 4. Cycle detection and resolution
        self.resolve_ordering_cycles();
    }

    /// Detects and breaks ordering cycles in `after_deps` using DFS.
    fn resolve_ordering_cycles(&mut self) {
        let all_units: Vec<String> = self.units.keys().cloned().collect();
        for start in &all_units {
            let mut visited = HashSet::new();
            let mut stack = Vec::new();
            self.dfs_cycle_break(start, &mut visited, &mut stack);
        }
    }

    fn dfs_cycle_break(
        &mut self,
        current: &str,
        visited: &mut HashSet<String>,
        stack: &mut Vec<String>,
    ) {
        visited.insert(current.to_string());
        stack.push(current.to_string());

        if let Some(neighbors) = self.after_deps.get(current).cloned() {
            for neighbor in neighbors {
                if let Some(pos) = stack.iter().position(|x| x == &neighbor) {
                    // Cycle detected! Break edge current -> neighbor
                    let cycle = stack[pos..].join(" -> ");
                    eprintln!("[novad::graph] Warning: Detected ordering cycle: {} -> {}. Breaking edge.", cycle, neighbor);
                    if let Some(set) = self.after_deps.get_mut(current) {
                        set.remove(&neighbor);
                    }
                } else if !visited.contains(&neighbor) {
                    self.dfs_cycle_break(&neighbor, visited, stack);
                }
            }
        }

        stack.pop();
    }

    /// Recursively collects all transitive dependencies for a given target unit.
    pub fn collect_transitive_deps(&self, target: &str) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(target.to_string());
        result.insert(target.to_string());

        while let Some(u) = queue.pop_front() {
            if let Some(reqs) = self.requires.get(&u) {
                for r in reqs {
                    if self.units.contains_key(r) && result.insert(r.clone()) {
                        queue.push_back(r.clone());
                    }
                }
            }
            if let Some(wants) = self.wants.get(&u) {
                for w in wants {
                    if self.units.contains_key(w) && result.insert(w.clone()) {
                        queue.push_back(w.clone());
                    }
                }
            }
        }

        result
    }

    /// Computes parallel startup execution waves (batches) using topological layering.
    /// Units in the same wave have no dependencies between each other and can be executed concurrently.
    pub fn compute_parallel_startup_waves(&self, unit_subset: &HashSet<String>) -> Vec<Vec<String>> {
        // Build in-degree map based strictly on After= edges within unit_subset
        let mut in_degrees: HashMap<String, usize> = HashMap::new();
        let mut dependents: HashMap<String, HashSet<String>> = HashMap::new();

        for u in unit_subset {
            in_degrees.insert(u.clone(), 0);
        }

        for u in unit_subset {
            if let Some(prereqs) = self.after_deps.get(u) {
                for p in prereqs {
                    if unit_subset.contains(p) && p != u {
                        *in_degrees.entry(u.clone()).or_insert(0) += 1;
                        dependents.entry(p.clone()).or_default().insert(u.clone());
                    }
                }
            }
        }

        let mut waves: Vec<Vec<String>> = Vec::new();
        let mut ready: Vec<String> = in_degrees
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(u, _)| u.clone())
            .collect();
        ready.sort(); // Deterministic ordering within wave

        let mut processed_count = 0;
        let total_units = unit_subset.len();

        while !ready.is_empty() {
            let current_wave = ready;
            ready = Vec::new();

            for u in &current_wave {
                processed_count += 1;
                if let Some(next_units) = dependents.get(u) {
                    for next in next_units {
                        if let Some(deg) = in_degrees.get_mut(next) {
                            if *deg > 0 {
                                *deg -= 1;
                                if *deg == 0 {
                                    ready.push(next.clone());
                                }
                            }
                        }
                    }
                }
            }

            ready.sort();
            waves.push(current_wave);
        }

        // If some units had unresolvable circular dependencies, append remaining as a final wave
        if processed_count < total_units {
            let mut remaining: Vec<String> = in_degrees
                .iter()
                .filter(|(_, &deg)| deg > 0)
                .map(|(u, _)| u.clone())
                .collect();
            remaining.sort();
            if !remaining.is_empty() {
                waves.push(remaining);
            }
        }

        waves
    }

    /// Computes graceful shutdown sequence (exact reverse topological ordering).
    pub fn compute_shutdown_waves(&self, active_units: &HashSet<String>) -> Vec<Vec<String>> {
        let startup_waves = self.compute_parallel_startup_waves(active_units);
        let mut shutdown_waves = startup_waves;
        shutdown_waves.reverse();
        shutdown_waves
    }

    /// Checks if a unit's hard dependencies (Requires=) are satisfied, and identifies conflicts.
    pub fn evaluate_dependencies(
        &self,
        unit: &str,
        states: &HashMap<String, UnitState>,
    ) -> Result<Vec<String>, String> {
        // Check conflicts
        let mut to_stop = Vec::new();
        if let Some(conflicts) = self.conflicts.get(unit) {
            for c in conflicts {
                if let Some(&st) = states.get(c) {
                    if st == UnitState::Active || st == UnitState::Activating {
                        to_stop.push(c.clone());
                    }
                }
            }
        }

        // Check hard requirements
        if let Some(reqs) = self.requires.get(unit) {
            for r in reqs {
                if let Some(&st) = states.get(r) {
                    if st == UnitState::Failed {
                        return Err(format!(
                            "Unit '{}' cannot start because required dependency '{}' failed",
                            unit, r
                        ));
                    }
                }
            }
        }

        Ok(to_stop)
    }
}
