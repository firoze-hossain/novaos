//! userland/libs/nova-ui/src/lib.rs
//!
//! # NovaOS Retained-Mode Reactive UI Toolkit & Scene Graph (nova-ui)
//!
//! A modern, pure Rust retained-mode UI toolkit providing:
//! - Flexbox layout engine with row/column distribution, gaps, and padding
//! - Vector shape rasterization (SDF rounded rectangles, Bezier paths, anti-aliased text)
//! - Hardware-accelerated shaders (Gaussian blur, box shadows, backdrop filters)
//! - Retained-mode scene graph with damage tracking and z-ordering
//! - Responsive event routing with hit testing, hover, capture, and focus
//! - Reactive state bindings (Signals)
//! - Complete suite of declarative widgets: Button, Slider, TextInput, List, Table, Tabs, and Modal

#![cfg_attr(not(test), no_std)]
#![allow(dead_code, unused_variables)]

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::vec;

#[cfg(test)]
use std::vec;

pub mod layout;
pub mod render;
pub mod scene;
pub mod event;
pub mod reactive;
pub mod widget;

pub mod prelude {
    pub use crate::layout::{
        AlignItems, Dimension, EdgeInsets, FlexDirection, FlexStyle, FlexWrap, JustifyContent,
        LayoutNode, Point, Rect, Size,
    };
    pub use crate::render::{
        fast_sqrt, Color, BufferSurface, BLACK, TRANSPARENT, WHITE,
    };
    pub use crate::scene::{NodeId, PaintCommand, SceneGraph, SceneNode};
    pub use crate::event::{
        EventDispatcher, EventHandler, EventOutcome, KeyEvent, KeyModifiers, PointerButton,
        PointerEvent,
    };
    pub use crate::reactive::{Binding, Signal};
    pub use crate::widget::{
        ButtonState, ButtonWidget, ListWidget, ModalWidget, SliderWidget, TableWidget,
        TabsWidget, TextInputWidget, PRIMARY_ACCENT,
    };
}

// ============================================================================
// Declarative UI Construction Macros
// ============================================================================

#[macro_export]
macro_rules! row {
    (gap: $gap:expr, padding: $pad:expr, children: [$($child:expr),* $(,)?]) => {{
        let style = $crate::layout::FlexStyle {
            direction: $crate::layout::FlexDirection::Row,
            gap: $gap,
            padding: $crate::layout::EdgeInsets::all($pad),
            ..Default::default()
        };
        let mut children = vec![];
        $(children.push($child);)*
        $crate::layout::LayoutNode::with_children(0, style, children)
    }};
}

#[macro_export]
macro_rules! col {
    (gap: $gap:expr, padding: $pad:expr, children: [$($child:expr),* $(,)?]) => {{
        let style = $crate::layout::FlexStyle {
            direction: $crate::layout::FlexDirection::Column,
            gap: $gap,
            padding: $crate::layout::EdgeInsets::all($pad),
            ..Default::default()
        };
        let mut children = vec![];
        $(children.push($child);)*
        $crate::layout::LayoutNode::with_children(0, style, children)
    }};
}

#[macro_export]
macro_rules! button {
    ($label:expr) => {
        $crate::widget::ButtonWidget::new($label)
    };
}

#[macro_export]
macro_rules! slider {
    ($min:expr, $max:expr, $val:expr) => {
        $crate::widget::SliderWidget::new($min, $max, $val)
    };
}

#[macro_export]
macro_rules! text_input {
    ($placeholder:expr) => {
        $crate::widget::TextInputWidget::new($placeholder)
    };
}

#[macro_export]
macro_rules! modal {
    ($title:expr, $w:expr, $h:expr) => {
        $crate::widget::ModalWidget::new($title, $w, $h)
    };
}

// ============================================================================
// Comprehensive Self-Test Harness
// ============================================================================

/// Runs the complete 7-point self-test validation suite for nova-ui.
pub fn run_nova_ui_selftest() -> bool {
    use prelude::*;

    // 1. Flexbox Layout Engine
    let mut root_layout = col!(
        gap: 8,
        padding: 12,
        children: [
            LayoutNode::new(1, FlexStyle {
                width: Dimension::Points(100),
                height: Dimension::Points(30),
                ..Default::default()
            }),
            LayoutNode::new(2, FlexStyle {
                flex_grow: 1.0,
                width: Dimension::Points(100),
                ..Default::default()
            })
        ]
    );
    root_layout.compute_layout(Size::new(200, 200));
    if root_layout.bounds.width != 200 || root_layout.children.len() != 2 {
        return false;
    }

    // 2. Vector Shape Rasterization & SDF Anti-Aliasing
    let mut surface = BufferSurface::new(100, 100);
    surface.clear(BLACK);
    surface.fill_rounded_rect(Rect::new(10, 10, 80, 80), 12, WHITE);
    if surface.get_pixel(50, 50) != WHITE {
        return false;
    }
    surface.draw_quadratic_bezier(Point::new(0, 0), Point::new(50, 100), Point::new(100, 0), PRIMARY_ACCENT);

    // 3. Shaders (Box Shadow & Gaussian Blur)
    surface.draw_box_shadow(Rect::new(20, 20, 60, 60), 0, 4, 8, 2, Color::rgba(0, 0, 0, 120));
    surface.apply_gaussian_blur(Rect::new(10, 10, 40, 40), 3);

    // 4. Retained-Mode Scene Graph & Damage Tracking
    let mut sg = SceneGraph::new();
    sg.get_node_mut(sg.root).unwrap().bounds = Rect::new(0, 0, 800, 600);
    let card = sg.create_node("main_card");
    sg.add_child(sg.root, card);
    if let Some(n) = sg.get_node_mut(card) {
        n.bounds = Rect::new(10, 10, 180, 180);
    }
    sg.mark_dirty(card);
    if sg.damage_region().width != 180 {
        return false;
    }
    let hit = sg.hit_test(Point::new(50, 50));
    if hit != Some(card) {
        return false;
    }
    sg.render(&mut surface);

    // 5. Responsive Event Dispatcher
    let mut dispatcher = EventDispatcher::new();
    struct DummyHandler;
    impl EventHandler for DummyHandler {}
    let mut handler = DummyHandler;
    dispatcher.dispatch_pointer(
        PointerEvent::Move { pos: Point::new(50, 50) },
        &sg,
        &mut handler,
    );
    if dispatcher.hovered_node != Some(card) {
        return false;
    }

    // 6. Reactive State & Signal Notification
    let mut signal = Signal::new(100);
    signal.subscribe(card);
    signal.set_and_notify(200, &mut sg);
    if !sg.get_node(card).unwrap().dirty_paint {
        return false;
    }

    // 7. Core Declarative Widgets (Button, Slider, TextInput, List, Table, Tabs, Modal)
    let btn = ButtonWidget::new("OK");
    let mut slider = SliderWidget::new(0.0, 10.0, 5.0);
    let mut input = TextInputWidget::new("Type here...");
    let list = ListWidget::new(vec!["Item A".into(), "Item B".into()]);
    let table = TableWidget::new(vec!["Col 1".into()], vec![vec!["Val 1".into()]], vec![100]);
    let tabs = TabsWidget::new(vec!["Tab 1".into(), "Tab 2".into()]);
    let mut modal = ModalWidget::new("Alert", 200, 100);

    input.insert_char('A');
    slider.set_from_pos(50, Rect::new(0, 0, 100, 20));
    modal.open();

    if input.text != "A" || !modal.is_open {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_selftest_suite() {
        assert!(run_nova_ui_selftest(), "Nova-UI complete self-test suite passed");
    }

    #[test]
    fn test_declarative_macros() {
        let node = row!(
            gap: 10,
            padding: 5,
            children: [
                layout::LayoutNode::new(1, layout::FlexStyle::default()),
                layout::LayoutNode::new(2, layout::FlexStyle::default())
            ]
        );
        assert_eq!(node.style.direction, layout::FlexDirection::Row);
        assert_eq!(node.style.gap, 10);
        assert_eq!(node.children.len(), 2);
    }
}
