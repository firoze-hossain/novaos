//! userland/desktop/apps/nova-edit/src/bracket.rs
//!
//! Bracket Matching and Auto-Closing Delimiters Engine for NovaOS Code Editor.

pub struct BracketMatcher;

impl BracketMatcher {
    /// Returns the auto-closing character corresponding to an opening delimiter.
    pub fn get_auto_close_char(ch: char) -> Option<char> {
        match ch {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            '\'' => Some('\''),
            _ => None,
        }
    }

    /// Finds the matching bracket position in the full document text for a bracket at `pos`.
    pub fn find_matching_bracket(text: &str, pos: usize) -> Option<usize> {
        let bytes = text.as_bytes();
        if pos >= bytes.len() {
            return None;
        }

        let ch = bytes[pos];
        match ch {
            b'(' => Self::scan_forward(bytes, pos, b'(', b')'),
            b'[' => Self::scan_forward(bytes, pos, b'[', b']'),
            b'{' => Self::scan_forward(bytes, pos, b'{', b'}'),
            b')' => Self::scan_backward(bytes, pos, b')', b'('),
            b']' => Self::scan_backward(bytes, pos, b']', b'['),
            b'}' => Self::scan_backward(bytes, pos, b'}', b'{'),
            _ => None,
        }
    }

    fn scan_forward(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
        let mut depth = 0;
        for i in start..bytes.len() {
            if bytes[i] == open {
                depth += 1;
            } else if bytes[i] == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    }

    fn scan_backward(bytes: &[u8], start: usize, close: u8, open: u8) -> Option<usize> {
        let mut depth = 0;
        for i in (0..=start).rev() {
            if bytes[i] == close {
                depth += 1;
            } else if bytes[i] == open {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    }
}
