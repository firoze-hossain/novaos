//! userland/libs/nova-ui/src/event.rs
//!
//! Responsive Event Routing & Dispatcher for NovaOS UI.
//! Handles pointer interactions, hit testing, active capture, keyboard focus, and event bubbling.

#[cfg(not(test))]
extern crate alloc;

#[cfg(test)]
use std::vec::Vec;

use crate::layout::Point;
use crate::scene::{NodeId, SceneGraph};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerEvent {
    Down { pos: Point, button: PointerButton },
    Up { pos: Point, button: PointerButton },
    Move { pos: Point },
    Scroll { dx: i32, dy: i32 },
    Click { pos: Point, button: PointerButton },
    Enter(NodeId),
    Leave(NodeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    Down { key_code: u32, modifiers: KeyModifiers },
    Up { key_code: u32, modifiers: KeyModifiers },
    Char { ch: char, modifiers: KeyModifiers },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventOutcome {
    Handled,
    Ignored,
}

pub trait EventHandler {
    fn handle_pointer(&mut self, node: NodeId, event: PointerEvent) -> EventOutcome {
        let _ = (node, event);
        EventOutcome::Ignored
    }

    fn handle_key(&mut self, node: NodeId, event: KeyEvent) -> EventOutcome {
        let _ = (node, event);
        EventOutcome::Ignored
    }
}

pub struct EventDispatcher {
    pub hovered_node: Option<NodeId>,
    pub focused_node: Option<NodeId>,
    pub captured_node: Option<NodeId>,
    last_down_node: Option<NodeId>,
    last_down_pos: Point,
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self {
            hovered_node: None,
            focused_node: None,
            captured_node: None,
            last_down_node: None,
            last_down_pos: Point::default(),
        }
    }

    pub fn set_focus(&mut self, node: Option<NodeId>) {
        self.focused_node = node;
    }

    pub fn capture_pointer(&mut self, node: NodeId) {
        self.captured_node = Some(node);
    }

    pub fn release_pointer(&mut self) {
        self.captured_node = None;
    }

    /// Dispatches pointer events against the scene graph to the registered event handler.
    pub fn dispatch_pointer(
        &mut self,
        event: PointerEvent,
        scene: &SceneGraph,
        handler: &mut dyn EventHandler,
    ) -> EventOutcome {
        match event {
            PointerEvent::Move { pos } => {
                // Determine target node: if captured, use captured; otherwise hit-test
                let hit = if let Some(cap) = self.captured_node {
                    Some(cap)
                } else {
                    scene.hit_test(pos)
                };

                // Check for hover transitions
                if hit != self.hovered_node {
                    if let Some(old) = self.hovered_node {
                        handler.handle_pointer(old, PointerEvent::Leave(old));
                    }
                    if let Some(new_node) = hit {
                        handler.handle_pointer(new_node, PointerEvent::Enter(new_node));
                    }
                    self.hovered_node = hit;
                }

                if let Some(target) = hit {
                    handler.handle_pointer(target, PointerEvent::Move { pos })
                } else {
                    EventOutcome::Ignored
                }
            }

            PointerEvent::Down { pos, button } => {
                let hit = scene.hit_test(pos);
                self.focused_node = hit;
                self.last_down_node = hit;
                self.last_down_pos = pos;

                if button == PointerButton::Left {
                    if let Some(node) = hit {
                        self.captured_node = Some(node);
                    }
                }

                if let Some(target) = hit {
                    handler.handle_pointer(target, PointerEvent::Down { pos, button })
                } else {
                    EventOutcome::Ignored
                }
            }

            PointerEvent::Up { pos, button } => {
                let target = self.captured_node.or_else(|| scene.hit_test(pos));
                self.captured_node = None;

                let mut outcome = EventOutcome::Ignored;
                if let Some(node) = target {
                    outcome = handler.handle_pointer(node, PointerEvent::Up { pos, button });
                    // Check if click should be generated (down & up in same node)
                    if self.last_down_node == Some(node) {
                        let click_outcome = handler.handle_pointer(node, PointerEvent::Click { pos, button });
                        if click_outcome == EventOutcome::Handled {
                            outcome = EventOutcome::Handled;
                        }
                    }
                }
                self.last_down_node = None;
                outcome
            }

            PointerEvent::Click { pos, button } => {
                if let Some(target) = scene.hit_test(pos) {
                    handler.handle_pointer(target, PointerEvent::Click { pos, button })
                } else {
                    EventOutcome::Ignored
                }
            }

            PointerEvent::Scroll { dx, dy } => {
                let target = self.hovered_node.or(self.focused_node).or(Some(scene.root));
                if let Some(node) = target {
                    handler.handle_pointer(node, PointerEvent::Scroll { dx, dy })
                } else {
                    EventOutcome::Ignored
                }
            }

            _ => EventOutcome::Ignored,
        }
    }

    /// Dispatches key events to the currently focused node.
    pub fn dispatch_key(&mut self, event: KeyEvent, handler: &mut dyn EventHandler) -> EventOutcome {
        if let Some(target) = self.focused_node {
            handler.handle_key(target, event)
        } else {
            EventOutcome::Ignored
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;

    struct MockHandler {
        clicked: Vec<NodeId>,
        entered: Vec<NodeId>,
        keys: Vec<char>,
    }

    impl EventHandler for MockHandler {
        fn handle_pointer(&mut self, node: NodeId, event: PointerEvent) -> EventOutcome {
            match event {
                PointerEvent::Click { .. } => {
                    self.clicked.push(node);
                    EventOutcome::Handled
                }
                PointerEvent::Enter(n) => {
                    self.entered.push(n);
                    EventOutcome::Handled
                }
                _ => EventOutcome::Ignored,
            }
        }

        fn handle_key(&mut self, _node: NodeId, event: KeyEvent) -> EventOutcome {
            if let KeyEvent::Char { ch, .. } = event {
                self.keys.push(ch);
                EventOutcome::Handled
            } else {
                EventOutcome::Ignored
            }
        }
    }

    #[test]
    fn test_click_and_hover_dispatch() {
        let mut sg = SceneGraph::new();
        sg.get_node_mut(sg.root).unwrap().bounds = Rect::new(0, 0, 800, 600);

        let btn = sg.create_node("button");
        sg.get_node_mut(btn).unwrap().bounds = Rect::new(50, 50, 100, 40);
        sg.add_child(sg.root, btn);

        let mut dispatcher = EventDispatcher::new();
        let mut handler = MockHandler {
            clicked: Vec::new(),
            entered: Vec::new(),
            keys: Vec::new(),
        };

        // Pointer move into button
        dispatcher.dispatch_pointer(PointerEvent::Move { pos: Point::new(60, 60) }, &sg, &mut handler);
        assert_eq!(dispatcher.hovered_node, Some(btn));
        assert_eq!(handler.entered, vec![btn]);

        // Pointer Down + Up = Click
        dispatcher.dispatch_pointer(PointerEvent::Down { pos: Point::new(60, 60), button: PointerButton::Left }, &sg, &mut handler);
        dispatcher.dispatch_pointer(PointerEvent::Up { pos: Point::new(60, 60), button: PointerButton::Left }, &sg, &mut handler);
        assert_eq!(handler.clicked, vec![btn]);
        assert_eq!(dispatcher.focused_node, Some(btn));

        // Key dispatch to focused button
        dispatcher.dispatch_key(KeyEvent::Char { ch: 'X', modifiers: KeyModifiers::default() }, &mut handler);
        assert_eq!(handler.keys, vec!['X']);
    }
}
