use crate::error::ParseError;

/// Preprocess Nous source to insert virtual INDENT (⇥) and DEDENT (⇤) tokens.
///
/// Nous uses significant indentation (2 spaces per level).
/// This preprocessor converts indentation into explicit tokens
/// so the PEG grammar can work without whitespace sensitivity.
pub fn preprocess_indentation(source: &str) -> Result<String, ParseError> {
    let mut result = String::with_capacity(source.len() * 2);
    let mut indent_stack: Vec<usize> = vec![0];
    let mut line_num = 0;

    for line in source.lines() {
        line_num += 1;

        // Skip empty lines and comment-only lines
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") {
            result.push('\n');
            continue;
        }

        // Count leading spaces
        let spaces = line.len() - line.trim_start().len();

        // Validate indentation is a multiple of 2
        if spaces % 2 != 0 {
            return Err(ParseError::Indentation {
                line: line_num,
                message: format!(
                    "indentation must be a multiple of 2 spaces, got {spaces}"
                ),
            });
        }

        let current_level = *indent_stack.last().unwrap();

        if spaces > current_level {
            // Indent: push new level and emit INDENT token
            indent_stack.push(spaces);
            result.push_str("⇥ ");
            result.push_str(trimmed);
            result.push('\n');
        } else if spaces < current_level {
            // Dedent: pop levels until we match, emit DEDENT for each
            while *indent_stack.last().unwrap() > spaces {
                indent_stack.pop();
                result.push_str("⇤ ");
            }
            if *indent_stack.last().unwrap() != spaces {
                return Err(ParseError::Indentation {
                    line: line_num,
                    message: format!(
                        "dedent to level {spaces} does not match any previous indentation"
                    ),
                });
            }
            result.push_str(trimmed);
            result.push('\n');
        } else {
            // Same level
            result.push_str(trimmed);
            result.push('\n');
        }
    }

    // Close any remaining indentation levels
    while indent_stack.len() > 1 {
        indent_stack.pop();
        result.push_str("⇤ ");
    }
    result.push('\n');

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flat_code() {
        let source = "ns banking\nuse banking.types\n";
        let result = preprocess_indentation(source).unwrap();
        assert!(!result.contains('⇥'));
        assert!(!result.contains('⇤'));
    }

    #[test]
    fn test_single_indent() {
        let source = "entity Account\n  id : Text\n  balance : Int\n";
        let result = preprocess_indentation(source).unwrap();
        assert!(result.contains('⇥'));
        assert!(result.contains('⇤'));
    }

    #[test]
    fn test_nested_indent() {
        let source = "fn foo() -> Int\n  require x > 0\n  let y = 1\n    let z = 2\n";
        let result = preprocess_indentation(source).unwrap();
        let indent_count = result.matches('⇥').count();
        let dedent_count = result.matches('⇤').count();
        assert_eq!(indent_count, dedent_count);
    }

    #[test]
    fn test_odd_spaces_rejected() {
        let source = "entity Account\n   id : Text\n";
        let result = preprocess_indentation(source);
        assert!(result.is_err());
    }

    #[test]
    fn test_comments_skipped() {
        let source = "-- this is a comment\nns banking\n";
        let result = preprocess_indentation(source).unwrap();
        assert!(result.contains("ns banking"));
    }
}
