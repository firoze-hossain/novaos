//! NovaOS Starter GUI Application Template
use novaos_sdk::prelude::*;

fn main() {
    println!("Initializing NovaOS Starter GUI App...");

    let mut app = App::builder("com.novaos.template.gui", "Nova App Template")
        .window_size(640, 480)
        .theme(ThemeMode::Dark)
        .accent(AccentPalette::FluentBlue)
        .build();

    let main_panel = app.add_view("main_panel", Rect::new(20, 20, 600, 440));
    let mut click_count = State::new(0);
    click_count.subscribe(main_panel);

    println!("App running. Window ready.");
}
