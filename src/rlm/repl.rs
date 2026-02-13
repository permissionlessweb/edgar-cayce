/// Parsed command from LLM output.
#[derive(Debug)]
pub enum Command {
    /// Python code to execute (from ```repl or ```python blocks).
    RunCode(String),
    /// Final answer — loop terminates.
    Final(String),
    /// Unrecognized output — continue conversation.
    InvalidCommand,
}

impl Command {
    /// Parse an LLM response into a structured command.
    pub fn parse(input: &str) -> Self {
        // Check for FINAL(...) first
        if let Some(answer) = extract_final(input) {
            return Command::Final(answer);
        }

        // Check for code blocks: ```repl, ```python, ```py
        if let Some(code) = extract_code_block(input) {
            return Command::RunCode(code);
        }

        Command::InvalidCommand
    }

    pub fn get_final(&self) -> Option<&str> {
        match self {
            Command::Final(s) => Some(s),
            _ => None,
        }
    }

    pub fn get_code(&self) -> Option<&str> {
        match self {
            Command::RunCode(s) => Some(s),
            _ => None,
        }
    }
}

/// Extract content from FINAL(...) using paren-counting.
fn extract_final(input: &str) -> Option<String> {
    let idx = input.find("FINAL(")?;
    let after = &input[idx + 6..]; // skip "FINAL("
    let mut depth = 1i32;
    let mut end = None;

    for (i, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    let content = match end {
        Some(e) => &after[..e],
        None => after.trim(), // Unclosed — take everything
    };

    // Strip surrounding quotes if present
    let trimmed = content.trim();
    let unquoted = if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    Some(unquoted.to_string())
}

/// Extract code from ```repl, ```python, or ```py blocks.
fn extract_code_block(input: &str) -> Option<String> {
    // Find opening fence
    let patterns = ["```repl", "```python", "```py"];
    let mut best_start = None;

    for pat in &patterns {
        if let Some(idx) = input.find(pat) {
            match best_start {
                None => best_start = Some((idx, pat.len())),
                Some((prev, _)) if idx < prev => best_start = Some((idx, pat.len())),
                _ => {}
            }
        }
    }

    let (start_idx, pat_len) = best_start?;
    let after_tag = &input[start_idx + pat_len..];

    // Skip to next newline (the rest of the opening fence line)
    let code_start = after_tag.find('\n').map(|i| i + 1).unwrap_or(0);
    let code_region = &after_tag[code_start..];

    // Find closing ```
    let end = code_region.find("```").unwrap_or(code_region.len());
    let code = code_region[..end].trim();

    if code.is_empty() {
        None
    } else {
        Some(code.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_final() {
        let input = r#"FINAL(The answer is 42)"#;
        let cmd = Command::parse(input);
        assert_eq!(cmd.get_final(), Some("The answer is 42"));
    }

    #[test]
    fn test_parse_final_with_quotes() {
        let input = r#"FINAL("Hello world")"#;
        let cmd = Command::parse(input);
        assert_eq!(cmd.get_final(), Some("Hello world"));
    }

    #[test]
    fn test_parse_final_nested_parens() {
        let input = r#"FINAL(func(a, b) returns (c))"#;
        let cmd = Command::parse(input);
        assert_eq!(cmd.get_final(), Some("func(a, b) returns (c)"));
    }

    #[test]
    fn test_parse_code_block() {
        let input = "Let me check:\n```repl\nprint('hello')\n```\n";
        let cmd = Command::parse(input);
        assert_eq!(cmd.get_code(), Some("print('hello')"));
    }

    #[test]
    fn test_parse_python_block() {
        let input = "```python\nx = 1 + 2\nprint(x)\n```";
        let cmd = Command::parse(input);
        assert_eq!(cmd.get_code(), Some("x = 1 + 2\nprint(x)"));
    }

    #[test]
    fn test_parse_invalid() {
        let input = "I think we should look at the documents.";
        let cmd = Command::parse(input);
        assert!(matches!(cmd, Command::InvalidCommand));
    }
}
