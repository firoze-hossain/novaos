//! sdk/examples/calculator-app/src/main.rs
//!
//! Fluent Calculator GUI application built with `novaos-sdk`.
//! Demonstrates grid layout, reactive calculation state, and custom styled buttons.

use novaos_sdk::prelude::*;

#[derive(Debug, Clone)]
pub struct CalculatorState {
    pub display: String,
    pub accumulator: f64,
    pub pending_op: Option<char>,
    pub clear_on_next_digit: bool,
}

impl CalculatorState {
    pub fn new() -> Self {
        Self {
            display: "0".to_string(),
            accumulator: 0.0,
            pending_op: None,
            clear_on_next_digit: false,
        }
    }

    pub fn input_digit(&mut self, digit: char) {
        if self.clear_on_next_digit || self.display == "0" {
            self.display = digit.to_string();
            self.clear_on_next_digit = false;
        } else {
            self.display.push(digit);
        }
    }

    pub fn input_decimal(&mut self) {
        if self.clear_on_next_digit {
            self.display = "0.".to_string();
            self.clear_on_next_digit = false;
        } else if !self.display.contains('.') {
            self.display.push('.');
        }
    }

    pub fn apply_operator(&mut self, op: char) {
        let val: f64 = self.display.parse().unwrap_or(0.0);
        if let Some(pending) = self.pending_op {
            self.accumulator = match pending {
                '+' => self.accumulator + val,
                '-' => self.accumulator - val,
                '*' | '×' => self.accumulator * val,
                '/' | '÷' => {
                    if val != 0.0 {
                        self.accumulator / val
                    } else {
                        0.0
                    }
                }
                _ => val,
            };
        } else {
            self.accumulator = val;
        }
        self.pending_op = Some(op);
        self.clear_on_next_digit = true;
        self.display = format!("{}", self.accumulator);
    }

    pub fn equals(&mut self) {
        if let Some(op) = self.pending_op {
            let val: f64 = self.display.parse().unwrap_or(0.0);
            self.accumulator = match op {
                '+' => self.accumulator + val,
                '-' => self.accumulator - val,
                '*' | '×' => self.accumulator * val,
                '/' | '÷' => {
                    if val != 0.0 {
                        self.accumulator / val
                    } else {
                        0.0
                    }
                }
                _ => val,
            };
            self.display = format!("{}", self.accumulator);
            self.pending_op = None;
            self.clear_on_next_digit = true;
        }
    }

    pub fn clear(&mut self) {
        self.display = "0".to_string();
        self.accumulator = 0.0;
        self.pending_op = None;
        self.clear_on_next_digit = false;
    }
}

impl Default for CalculatorState {
    fn default() -> Self {
        Self::new()
    }
}

fn main() {
    println!("Launching NovaOS Calculator...");

    let mut app = App::builder("com.novaos.calculator", "Calculator")
        .window_size(360, 480)
        .theme(ThemeMode::Dark)
        .accent(AccentPalette::FluentBlue)
        .resizable(false)
        .build();

    let mut calc_state = CalculatorState::new();

    // Verify basic math operations
    calc_state.input_digit('4');
    calc_state.input_digit('2');
    calc_state.apply_operator('+');
    calc_state.input_digit('8');
    calc_state.equals();

    println!("Calculator initial calculation: 42 + 8 = {}", calc_state.display);
    assert_eq!(calc_state.display, "50");

    let screen_view = app.add_view("calc_display", Rect::new(16, 20, 328, 80));
    let mut display_signal = State::new(calc_state.display.clone());
    display_signal.subscribe(screen_view);

    app.step_frame();
    println!("NovaOS Calculator window initialized successfully (360x480).");
}
