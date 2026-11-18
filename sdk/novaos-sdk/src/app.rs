//! sdk/novaos-sdk/src/app.rs
//!
//! Application lifecycle, window management, and main loop.

use crate::emulator::SimulatedWindow;
use nova_ui::prelude::*;

/// Configuration metadata for a NovaOS application.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bundle_id: String,
    pub display_name: String,
    pub width: i32,
    pub height: i32,
    pub theme_mode: ThemeMode,
    pub accent: AccentPalette,
    pub resizable: bool,
}

impl AppConfig {
    pub fn new(bundle_id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            display_name: display_name.into(),
            width: 800,
            height: 600,
            theme_mode: ThemeMode::Dark,
            accent: AccentPalette::FluentBlue,
            resizable: true,
        }
    }
}

/// Reactive state holder for application data.
#[derive(Debug, Clone)]
pub struct State<T: Clone> {
    val: T,
    subscribers: Vec<NodeId>,
}

impl<T: Clone> State<T> {
    pub fn new(initial: T) -> Self {
        Self {
            val: initial,
            subscribers: Vec::new(),
        }
    }

    pub fn get(&self) -> T {
        self.val.clone()
    }

    pub fn set(&mut self, new_val: T, scene: &mut SceneGraph) {
        self.val = new_val;
        for &node in &self.subscribers {
            scene.mark_dirty(node);
        }
    }

    pub fn subscribe(&mut self, node: NodeId) {
        if !self.subscribers.contains(&node) {
            self.subscribers.push(node);
        }
    }
}

/// The core application runtime managing windowing, scene graph, and events.
pub struct App {
    pub config: AppConfig,
    pub scene: SceneGraph,
    pub theme: ThemeManager,
    pub dispatcher: EventDispatcher,
    pub window: SimulatedWindow,
    pub running: bool,
}

impl App {
    pub fn builder(bundle_id: impl Into<String>, display_name: impl Into<String>) -> AppBuilder {
        AppBuilder::new(bundle_id, display_name)
    }

    pub fn new(config: AppConfig) -> Self {
        let width = config.width;
        let height = config.height;
        let title = config.display_name.clone();

        let mut scene = SceneGraph::new();
        if let Some(root) = scene.get_node_mut(scene.root) {
            root.bounds = Rect::new(0, 0, width, height);
        }

        let theme = ThemeManager::new(config.theme_mode, config.accent);
        let dispatcher = EventDispatcher::new();
        let window = SimulatedWindow::new(title, width, height);

        Self {
            config,
            scene,
            theme,
            dispatcher,
            window,
            running: false,
        }
    }

    /// Renders a single frame to the window display surface.
    pub fn step_frame(&mut self) {
        self.window.render_frame(&mut self.scene);
    }

    /// Dispatches a pointer click event at (x, y).
    pub fn click(&mut self, handler: &mut dyn EventHandler, x: i32, y: i32) {
        crate::emulator::InputSimulator::click(&mut self.dispatcher, &self.scene, handler, x, y);
        self.step_frame();
    }

    /// Adds a child widget to the scene root.
    pub fn add_view(&mut self, name: &'static str, bounds: Rect) -> NodeId {
        let node = self.scene.create_node(name);
        self.scene.add_child(self.scene.root, node);
        if let Some(n) = self.scene.get_node_mut(node) {
            n.bounds = bounds;
        }
        self.scene.mark_dirty(node);
        node
    }

    /// Runs the application event loop (simulated for testing).
    pub fn run_test_loop(&mut self, frames: usize) {
        self.running = true;
        for _ in 0..frames {
            self.step_frame();
        }
        self.running = false;
    }
}

/// Fluent builder for constructing `App` instances.
pub struct AppBuilder {
    config: AppConfig,
}

impl AppBuilder {
    pub fn new(bundle_id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            config: AppConfig::new(bundle_id, display_name),
        }
    }

    pub fn window_size(mut self, width: i32, height: i32) -> Self {
        self.config.width = width;
        self.config.height = height;
        self
    }

    pub fn theme(mut self, mode: ThemeMode) -> Self {
        self.config.theme_mode = mode;
        self
    }

    pub fn accent(mut self, accent: AccentPalette) -> Self {
        self.config.accent = accent;
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.config.resizable = resizable;
        self
    }

    pub fn build(self) -> App {
        App::new(self.config)
    }
}
