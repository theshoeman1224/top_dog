pub mod activation;
pub mod config;
pub mod monitor;
pub mod plugin;
pub mod view;

/// Strip terminal control sequences, including OSC payloads and C1 controls.
pub fn display_text(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {
                    for x in chars.by_ref() {
                        if ('@'..='~').contains(&x) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(x) = chars.next() {
                        if x == '\x07' {
                            break;
                        } else if x == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn removes_terminal_control_sequences() {
        assert_eq!(
            super::display_text("a\x1b[31mb\x1b]52;c;secret\x07c\n\u{009b}"),
            "abc"
        );
    }
}
