//! userland/libs/nova-ui/src/reactive.rs
//!
//! Reactive State Management Primitives for NovaOS UI.
//! Provides Signals, State bindings, and automated subscriber dirty-notification.

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

use crate::scene::{NodeId, SceneGraph};

#[derive(Clone, Debug)]
pub struct Signal<T> {
    value: T,
    version: usize,
    subscribers: Vec<NodeId>,
}

impl<T: PartialEq + Clone> Signal<T> {
    pub fn new(initial: T) -> Self {
        Self {
            value: initial,
            version: 0,
            subscribers: Vec::new(),
        }
    }

    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn version(&self) -> usize {
        self.version
    }

    pub fn subscribe(&mut self, node: NodeId) {
        if !self.subscribers.contains(&node) {
            self.subscribers.push(node);
        }
    }

    pub fn unsubscribe(&mut self, node: NodeId) {
        self.subscribers.retain(|&n| n != node);
    }

    pub fn set(&mut self, new_val: T) -> bool {
        if self.value != new_val {
            self.value = new_val;
            self.version += 1;
            true
        } else {
            false
        }
    }

    pub fn set_and_notify(&mut self, new_val: T, scene: &mut SceneGraph) -> bool {
        if self.set(new_val) {
            self.notify(scene);
            true
        } else {
            false
        }
    }

    pub fn notify(&self, scene: &mut SceneGraph) {
        for &sub in &self.subscribers {
            scene.mark_dirty(sub);
        }
    }

    pub fn update<F: FnOnce(&mut T)>(&mut self, f: F) {
        f(&mut self.value);
        self.version += 1;
    }
}

#[derive(Clone, Debug)]
pub struct Binding<T> {
    getter: fn() -> T,
    setter: fn(T),
}

impl<T> Binding<T> {
    pub const fn new(getter: fn() -> T, setter: fn(T)) -> Self {
        Self { getter, setter }
    }

    pub fn get(&self) -> T {
        (self.getter)()
    }

    pub fn set(&self, val: T) {
        (self.setter)(val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_mutation_and_notification() {
        let mut sg = SceneGraph::new();
        let node1 = sg.create_node("counter_display");
        let mut count = Signal::new(0);

        count.subscribe(node1);
        assert_eq!(*count.get(), 0);
        assert_eq!(count.version(), 0);

        // Mutate signal
        let changed = count.set_and_notify(42, &mut sg);
        assert!(changed);
        assert_eq!(*count.get(), 42);
        assert_eq!(count.version(), 1);

        // Node in scene graph should now have dirty_paint set
        assert!(sg.get_node(node1).unwrap().dirty_paint);
    }
}
