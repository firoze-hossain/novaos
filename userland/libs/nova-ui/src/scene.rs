//! userland/libs/nova-ui/src/scene.rs
//!
//! Retained-Mode Scene Graph for NovaOS UI.
//! Manages hierarchical node trees, z-ordering, damage tracking, and dirty flag propagation.

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(test)]
use std::{string::String, vec::Vec};

use crate::layout::{Point, Rect};
use crate::render::{BufferSurface, Color};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

#[derive(Clone, Debug)]
pub enum PaintCommand {
    FillRect(Rect, Color),
    FillRoundedRect(Rect, i32, Color),
    StrokeRoundedRect(Rect, i32, i32, Color),
    BoxShadow(Rect, i32, i32, i32, i32, Color),
    Text(String, Point, Color),
    BackdropFilter(Rect, i32, Color),
    BezierQuadratic(Point, Point, Point, Color),
    BezierCubic(Point, Point, Point, Point, Color),
}

#[derive(Clone, Debug)]
pub struct SceneNode {
    pub id: NodeId,
    pub name: String,
    pub bounds: Rect,
    pub clip: Option<Rect>,
    pub z_index: i32,
    pub visible: bool,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub dirty_layout: bool,
    pub dirty_paint: bool,
    pub commands: Vec<PaintCommand>,
}

impl SceneNode {
    pub fn new(id: NodeId, name: &str) -> Self {
        Self {
            id,
            name: String::from(name),
            bounds: Rect::zero(),
            clip: None,
            z_index: 0,
            visible: true,
            parent: None,
            children: Vec::new(),
            dirty_layout: true,
            dirty_paint: true,
            commands: Vec::new(),
        }
    }
}

pub struct SceneGraph {
    nodes: Vec<SceneNode>,
    pub root: NodeId,
    damage_region: Rect,
}

impl SceneGraph {
    pub fn new() -> Self {
        let mut sg = Self {
            nodes: Vec::new(),
            root: NodeId(0),
            damage_region: Rect::zero(),
        };
        let root = sg.create_node("root");
        sg.root = root;
        sg
    }

    pub fn create_node(&mut self, name: &str) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(SceneNode::new(id, name));
        id
    }

    pub fn get_node(&self, id: NodeId) -> Option<&SceneNode> {
        self.nodes.get(id.0)
    }

    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut SceneNode> {
        self.nodes.get_mut(id.0)
    }

    pub fn add_child(&mut self, parent_id: NodeId, child_id: NodeId) {
        if parent_id.0 >= self.nodes.len() || child_id.0 >= self.nodes.len() {
            return;
        }
        self.nodes[child_id.0].parent = Some(parent_id);
        self.nodes[parent_id.0].children.push(child_id);
        self.mark_dirty(child_id);
    }

    pub fn remove_child(&mut self, parent_id: NodeId, child_id: NodeId) {
        if let Some(parent) = self.nodes.get_mut(parent_id.0) {
            parent.children.retain(|&c| c != child_id);
        }
        if let Some(child) = self.nodes.get_mut(child_id.0) {
            child.parent = None;
            self.damage_region = self.damage_region.union(&child.bounds);
        }
    }

    pub fn mark_dirty(&mut self, id: NodeId) {
        if let Some(node) = self.nodes.get_mut(id.0) {
            node.dirty_paint = true;
            node.dirty_layout = true;
            self.damage_region = self.damage_region.union(&node.bounds);
        }
        // Bubble dirty flag up to root
        let mut cur = self.nodes.get(id.0).and_then(|n| n.parent);
        while let Some(pid) = cur {
            if let Some(pnode) = self.nodes.get_mut(pid.0) {
                pnode.dirty_paint = true;
                cur = pnode.parent;
            } else {
                break;
            }
        }
    }

    pub fn damage_region(&self) -> Rect {
        self.damage_region
    }

    pub fn clear_damage(&mut self) {
        self.damage_region = Rect::zero();
    }

    /// Hit-tests the scene graph returning the deepest topmost node under point.
    pub fn hit_test(&self, point: Point) -> Option<NodeId> {
        self.hit_test_recursive(self.root, point)
    }

    fn hit_test_recursive(&self, node_id: NodeId, point: Point) -> Option<NodeId> {
        let node = self.nodes.get(node_id.0)?;
        if !node.visible || !node.bounds.contains(point) {
            return None;
        }
        if let Some(clip) = node.clip {
            if !clip.contains(point) {
                return None;
            }
        }

        // Sort children in reverse z-order (highest z first)
        let mut sorted_children = node.children.clone();
        sorted_children.sort_by_key(|&c| self.nodes[c.0].z_index);
        sorted_children.reverse();

        for child_id in sorted_children {
            if let Some(hit) = self.hit_test_recursive(child_id, point) {
                return Some(hit);
            }
        }

        Some(node_id)
    }

    /// Renders the scene graph to the given buffer surface.
    pub fn render(&mut self, surface: &mut BufferSurface) {
        self.render_node(self.root, surface);
        // Clear dirty flags
        for node in self.nodes.iter_mut() {
            node.dirty_paint = false;
            node.dirty_layout = false;
        }
        self.damage_region = Rect::zero();
    }

    fn render_node(&self, node_id: NodeId, surface: &mut BufferSurface) {
        let node = match self.nodes.get(node_id.0) {
            Some(n) => n,
            None => return,
        };
        if !node.visible {
            return;
        }

        // Execute commands for this node
        for cmd in &node.commands {
            match cmd {
                PaintCommand::FillRect(rect, color) => {
                    surface.fill_rect(*rect, *color);
                }
                PaintCommand::FillRoundedRect(rect, radius, color) => {
                    surface.fill_rounded_rect(*rect, *radius, *color);
                }
                PaintCommand::StrokeRoundedRect(rect, radius, stroke_width, color) => {
                    surface.stroke_rounded_rect(*rect, *radius, *stroke_width, *color);
                }
                PaintCommand::BoxShadow(rect, dx, dy, blur, spread, color) => {
                    surface.draw_box_shadow(*rect, *dx, *dy, *blur, *spread, *color);
                }
                PaintCommand::Text(text, pos, color) => {
                    surface.draw_text(text, pos.x, pos.y, *color);
                }
                PaintCommand::BackdropFilter(rect, blur, tint) => {
                    surface.apply_backdrop_filter(*rect, *blur, *tint);
                }
                PaintCommand::BezierQuadratic(p0, p1, p2, color) => {
                    surface.draw_quadratic_bezier(*p0, *p1, *p2, *color);
                }
                PaintCommand::BezierCubic(p0, p1, p2, p3, color) => {
                    surface.draw_cubic_bezier(*p0, *p1, *p2, *p3, *color);
                }
            }
        }

        // Render children ordered by z-index
        let mut children = node.children.clone();
        children.sort_by_key(|&c| self.nodes[c.0].z_index);
        for child_id in children {
            self.render_node(child_id, surface);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scene_graph_hierarchy() {
        let mut sg = SceneGraph::new();
        let child1 = sg.create_node("panel1");
        let child2 = sg.create_node("panel2");

        sg.add_child(sg.root, child1);
        sg.add_child(child1, child2);

        assert_eq!(sg.get_node(child2).unwrap().parent, Some(child1));
        assert_eq!(sg.get_node(child1).unwrap().parent, Some(sg.root));
        assert_eq!(sg.get_node(child1).unwrap().children, vec![child2]);
    }

    #[test]
    fn test_hit_testing() {
        let mut sg = SceneGraph::new();
        sg.get_node_mut(sg.root).unwrap().bounds = Rect::new(0, 0, 800, 600);

        let btn = sg.create_node("btn");
        if let Some(n) = sg.get_node_mut(btn) {
            n.bounds = Rect::new(50, 50, 100, 40);
            n.z_index = 10;
        }
        sg.add_child(sg.root, btn);

        // Click inside button
        let hit = sg.hit_test(Point::new(60, 60));
        assert_eq!(hit, Some(btn));

        // Click outside button but inside root
        let hit_bg = sg.hit_test(Point::new(10, 10));
        assert_eq!(hit_bg, Some(sg.root));

        // Click outside root
        let hit_miss = sg.hit_test(Point::new(900, 900));
        assert_eq!(hit_miss, None);
    }

    #[test]
    fn test_damage_region_tracking() {
        let mut sg = SceneGraph::new();
        let node = sg.create_node("window");
        sg.get_node_mut(node).unwrap().bounds = Rect::new(20, 20, 100, 100);

        sg.mark_dirty(node);
        assert_eq!(sg.damage_region(), Rect::new(20, 20, 100, 100));

        sg.clear_damage();
        assert_eq!(sg.damage_region(), Rect::zero());
    }
}
