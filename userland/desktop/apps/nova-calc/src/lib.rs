//! userland/desktop/apps/nova-calc/src/lib.rs
//!
//! # NovaOS Calculator Application Library (nova-calc)
//!
//! Unified Multi-Mode Calculator System for NovaOS Desktop Experience.
//! Synthesizes Apple Calculator (macOS), Windows 11 Fluent Calculator, and GNOME Calculator (Ubuntu).

pub mod model;
pub mod ui;

use model::{
    evaluate_expression, CalcMode, CalculatorState, ProgBase,
};
use ui::{CalcUiRenderer, CALC_HEIGHT, CALC_WIDTH};
use nova_ui::prelude::BufferSurface;

/// Executes the comprehensive autonomous qualification suite for NovaOS Calculator.
pub fn run_nova_calc_selftest() -> bool {
    let mut state = CalculatorState::new();

    // 1. Basic Arithmetic: 125 + 75 = 200
    state.input_digit('1');
    state.input_digit('2');
    state.input_digit('5');
    state.apply_binary_op('+');
    state.input_digit('7');
    state.input_digit('5');
    state.equals();
    if state.display != "200" {
        eprintln!("[FAIL] 125 + 75 should equal 200, got {}", state.display);
        return false;
    }

    // 2. Subtraction and Negation: 200 - 350 = -150
    state.apply_binary_op('-');
    state.input_digit('3');
    state.input_digit('5');
    state.input_digit('0');
    state.equals();
    if state.display != "-150" {
        eprintln!("[FAIL] 200 - 350 should equal -150, got {}", state.display);
        return false;
    }

    // Toggle Sign: -150 -> 150
    state.toggle_sign();
    if state.display != "150" {
        eprintln!("[FAIL] Toggle sign on -150 should equal 150, got {}", state.display);
        return false;
    }

    // 3. Multiplication and Decimals: 150 * 2.5 = 375
    state.apply_binary_op('*');
    state.input_digit('2');
    state.input_decimal();
    state.input_digit('5');
    state.equals();
    if state.display != "375" {
        eprintln!("[FAIL] 150 * 2.5 should equal 375, got {}", state.display);
        return false;
    }

    // 4. Division by Zero Protection
    state.apply_binary_op('/');
    state.input_digit('0');
    state.equals();
    if !state.is_error || !state.display.contains("Divide by 0") {
        eprintln!("[FAIL] Division by zero should report error, got {}", state.display);
        return false;
    }

    // 5. Reset and Unary Functions
    state.reset();
    state.input_digit('1');
    state.input_digit('6');
    state.apply_unary_op("sqrt");
    if state.display != "4" {
        eprintln!("[FAIL] sqrt(16) should equal 4, got {}", state.display);
        return false;
    }

    state.apply_unary_op("x²");
    if state.display != "16" {
        eprintln!("[FAIL] sqr(4) should equal 16, got {}", state.display);
        return false;
    }

    // Reciprocal: 1/16 = 0.0625
    state.apply_unary_op("1/x");
    if state.display != "0.0625" {
        eprintln!("[FAIL] 1/16 should equal 0.0625, got {}", state.display);
        return false;
    }

    // 6. Scientific Mode Functions
    state.mode = CalcMode::Scientific;
    state.reset();
    state.input_digit('5');
    state.apply_unary_op("fact");
    if state.display != "120" {
        eprintln!("[FAIL] 5! should equal 120, got {}", state.display);
        return false;
    }

    state.reset();
    state.insert_constant("pi");
    state.apply_unary_op("sin");
    let sin_pi = state.current_val();
    if sin_pi.abs() > 1e-6 {
        eprintln!("[FAIL] sin(pi) should be near 0, got {}", sin_pi);
        return false;
    }

    // 7. Programmer Mode: Multi-base conversions and bitwise ops
    state.mode = CalcMode::Programmer;
    state.set_prog_base(ProgBase::Hex);
    state.reset();
    state.input_digit('F');
    state.input_digit('F');
    if state.current_int() != 255 {
        eprintln!("[FAIL] HEX FF should equal DEC 255, got {}", state.current_int());
        return false;
    }

    // Bitwise AND: 0xFF & 0x0F = 0xF
    state.apply_binary_op('&');
    state.input_digit('0');
    state.input_digit('F');
    state.equals();
    if state.current_int() != 15 {
        eprintln!("[FAIL] 0xFF & 0x0F should equal 15, got {}", state.current_int());
        return false;
    }

    // 8. Memory Management
    state.mode = CalcMode::Standard;
    state.reset();
    state.input_digit('4');
    state.input_digit('2');
    state.memory_store();
    if !state.has_memory || (state.memory - 42.0).abs() > 1e-6 {
        eprintln!("[FAIL] Memory store 42 failed.");
        return false;
    }

    state.input_digit('8');
    state.memory_add();
    if (state.memory - 50.0).abs() > 1e-6 {
        eprintln!("[FAIL] Memory add 8 failed, expected 50.");
        return false;
    }

    state.reset();
    state.memory_recall();
    if state.display != "50" {
        eprintln!("[FAIL] Memory recall expected 50, got {}", state.display);
        return false;
    }

    state.memory_clear();
    if state.has_memory {
        eprintln!("[FAIL] Memory clear failed to reset flag.");
        return false;
    }

    // 9. History Journal
    if state.history.is_empty() {
        eprintln!("[FAIL] Expected history journal entries from previous calculations.");
        return false;
    }
    state.clear_history();
    if !state.history.is_empty() {
        eprintln!("[FAIL] Clear history failed.");
        return false;
    }

    // 10. Expression Parser Evaluation
    let expr_test = evaluate_expression("12 + 6 * 4 / (3 - 1)");
    match expr_test {
        Ok(v) => {
            // 12 + 24 / 2 = 12 + 12 = 24
            if (v - 24.0).abs() > 1e-6 {
                eprintln!("[FAIL] Expression evaluate '12 + 6 * 4 / (3 - 1)' expected 24, got {}", v);
                return false;
            }
        }
        Err(e) => {
            eprintln!("[FAIL] Expression parser error: {}", e);
            return false;
        }
    }

    // 11. BufferSurface UI Rasterization
    let mut surf = BufferSurface::new(CALC_WIDTH, CALC_HEIGHT);
    CalcUiRenderer::render(&mut surf, &state);

    let non_zero_pixels = surf.pixels.iter().filter(|&&p| p != 0).count();
    if non_zero_pixels < 1000 {
        eprintln!("[FAIL] BufferSurface rendering generated insufficient pixels: {}", non_zero_pixels);
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::model::{format_num, format_with_commas};

    #[test]
    fn test_calculator_state_arithmetic() {
        let mut calc = CalculatorState::new();
        calc.input_digit('7');
        calc.apply_binary_op('*');
        calc.input_digit('8');
        calc.equals();
        assert_eq!(calc.display, "56");

        calc.apply_binary_op('/');
        calc.input_digit('2');
        calc.equals();
        assert_eq!(calc.display, "28");
    }

    #[test]
    fn test_calculator_formatting() {
        assert_eq!(format_with_commas("1000000"), "1,000,000");
        assert_eq!(format_with_commas("1234.5678"), "1,234.5678");
        assert_eq!(format_with_commas("-9876543"), "-9,876,543");
        assert_eq!(format_num(42.0), "42");
        assert_eq!(format_num(3.1415), "3.1415");
    }

    #[test]
    fn test_calculator_expression_parser() {
        assert_eq!(evaluate_expression("2 + 3 * 4").unwrap(), 14.0);
        assert_eq!(evaluate_expression("(2 + 3) * 4").unwrap(), 20.0);
        assert_eq!(evaluate_expression("2 ^ 3").unwrap(), 8.0);
        assert_eq!(evaluate_expression("sqrt(81)").unwrap(), 9.0);
        assert_eq!(evaluate_expression("abs(-42)").unwrap(), 42.0);
    }

    #[test]
    fn test_calculator_programmer_mode() {
        let mut calc = CalculatorState::new();
        calc.mode = CalcMode::Programmer;
        calc.set_prog_base(ProgBase::Hex);
        calc.input_digit('A');
        calc.input_digit('0');
        assert_eq!(calc.current_int(), 160);

        calc.set_prog_base(ProgBase::Dec);
        assert_eq!(calc.display, "160");

        calc.set_prog_base(ProgBase::Bin);
        assert_eq!(calc.display, "10100000");
    }

    #[test]
    fn test_calculator_selftest_suite() {
        assert!(run_nova_calc_selftest());
    }
}
