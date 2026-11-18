//! sdk/novaos-sdk/src/emulator.rs
//!
//! Host-side application simulator and emulator runner.
//! Enables rapid local testing of NovaOS applications without full QEMU boots.

use nova_ui::prelude::*;

/// Simulated display buffer representing the application window.
pub struct SimulatedWindow {
    pub width: i32,
    pub height: i32,
    pub surface: BufferSurface,
    pub title: String,
    pub frame_count: u64,
}

impl SimulatedWindow {
    pub fn new(title: impl Into<String>, width: i32, height: i32) -> Self {
        let mut surface = BufferSurface::new(width.max(1) as usize, height.max(1) as usize);
        surface.clear(Color::rgb(24, 24, 28));
        Self {
            title: title.into(),
            width,
            height,
            surface,
            frame_count: 0,
        }
    }

    pub fn render_frame(&mut self, scene: &mut SceneGraph) {
        scene.render(&mut self.surface);
        self.frame_count += 1;
    }

    pub fn pixel_at(&self, x: i32, y: i32) -> Color {
        self.surface.get_pixel(x, y)
    }

    /// Exports the simulated window buffer as a Netpbm PPM image string (for pixel-diff testing).
    pub fn export_ppm(&self) -> String {
        let mut out = format!("P3\n{} {}\n255\n", self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let px = self.surface.get_pixel(x, y);
                out.push_str(&format!("{} {} {} ", px.r, px.g, px.b));
            }
            out.push('\n');
        }
        out
    }
}

/// Simulated user input injector for automated integration testing.
pub struct InputSimulator;

impl InputSimulator {
    pub fn click(dispatcher: &mut EventDispatcher, scene: &SceneGraph, handler: &mut dyn EventHandler, x: i32, y: i32) {
        dispatcher.dispatch_pointer(
            PointerEvent::Down {
                pos: Point::new(x, y),
                button: PointerButton::Left,
            },
            scene,
            handler,
        );
        dispatcher.dispatch_pointer(
            PointerEvent::Up {
                pos: Point::new(x, y),
                button: PointerButton::Left,
            },
            scene,
            handler,
        );
    }

    pub fn hover(dispatcher: &mut EventDispatcher, scene: &SceneGraph, handler: &mut dyn EventHandler, x: i32, y: i32) {
        dispatcher.dispatch_pointer(
            PointerEvent::Move {
                pos: Point::new(x, y),
            },
            scene,
            handler,
        );
    }
}
