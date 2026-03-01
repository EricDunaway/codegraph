//! Thrown error extraction (E8)
//!
//! Extracts thrown error types from function bodies using regex patterns.
//! Supports multiple languages with different throw/error patterns.

use codegraph_types::Language;
use regex::Regex;
use std::collections::HashSet;

/// Extract thrown error types from source code within a line range
///
/// For TypeScript/JavaScript: extracts from `throw new ErrorType(...)`
/// For Rust: extracts from `Err(ErrorType::...)` and `return Err(...)`
/// For Python: extracts from `raise ErrorType(...)`
pub fn extract_thrown_errors(
    source: &str,
    start_line: u32,
    end_line: u32,
    language: Language,
) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();

    // Convert to 0-indexed
    let start_idx = (start_line as usize).saturating_sub(1);
    let end_idx = (end_line as usize).min(lines.len());

    if start_idx >= lines.len() {
        return Vec::new();
    }

    let code_block = lines[start_idx..end_idx].join("\n");
    extract_errors_from_code(&code_block, language)
}

/// Extract error types from a code block
fn extract_errors_from_code(code: &str, language: Language) -> Vec<String> {
    let mut errors: HashSet<String> = HashSet::new();

    match language {
        Language::TypeScript | Language::JavaScript | Language::Tsx | Language::Jsx => {
            // Pattern: throw new ErrorType(...) or throw ErrorType(...)
            let re = Regex::new(r"throw\s+(?:new\s+)?([A-Z][a-zA-Z0-9_]*)\s*\(").unwrap();
            for cap in re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    errors.insert(error_type.as_str().to_string());
                }
            }
        }
        Language::Rust => {
            // Pattern: Err(ErrorType::Variant(...)) or Err(ErrorType(...))
            let re = Regex::new(r"Err\(\s*([A-Z][a-zA-Z0-9_]*(?:::[A-Z][a-zA-Z0-9_]*)?)").unwrap();
            for cap in re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    errors.insert(error_type.as_str().to_string());
                }
            }
            // Also catch ? operator with map_err
            let map_err_re = Regex::new(r"map_err\s*\(\s*\|[^|]*\|\s*([A-Z][a-zA-Z0-9_]*(?:::[A-Z][a-zA-Z0-9_]*)?)").unwrap();
            for cap in map_err_re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    errors.insert(error_type.as_str().to_string());
                }
            }
        }
        Language::Python => {
            // Pattern: raise ErrorType(...) or raise ErrorType
            let re = Regex::new(r"raise\s+([A-Z][a-zA-Z0-9_]*)(?:\s*\(|$|\s)").unwrap();
            for cap in re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    errors.insert(error_type.as_str().to_string());
                }
            }
        }
        Language::Java | Language::CSharp | Language::Kotlin => {
            // Pattern: throw new ExceptionType(...)
            let re = Regex::new(r"throw\s+new\s+([A-Z][a-zA-Z0-9_]*)\s*\(").unwrap();
            for cap in re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    errors.insert(error_type.as_str().to_string());
                }
            }
        }
        Language::Go => {
            // Pattern: errors.New(...) or fmt.Errorf(...) or custom error types
            // Also: return nil, ErrorType{...}
            let errors_new_re = Regex::new(r"errors\.New\s*\(").unwrap();
            if errors_new_re.is_match(code) {
                errors.insert("error".to_string()); // Generic Go error
            }
            let fmt_errorf_re = Regex::new(r"fmt\.Errorf\s*\(").unwrap();
            if fmt_errorf_re.is_match(code) {
                errors.insert("error".to_string());
            }
            // Custom error types: return ErrorType{...}
            let custom_re = Regex::new(r"return\s+(?:nil,\s*)?&?([A-Z][a-zA-Z0-9_]*)\s*\{").unwrap();
            for cap in custom_re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    let name = error_type.as_str();
                    if name.contains("Error") || name.contains("Err") {
                        errors.insert(name.to_string());
                    }
                }
            }
        }
        Language::Swift => {
            // Pattern: throw ErrorType.case or throw ErrorType(...)
            let re = Regex::new(r"throw\s+([A-Z][a-zA-Z0-9_]*(?:\.[a-zA-Z0-9_]+)?)").unwrap();
            for cap in re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    errors.insert(error_type.as_str().to_string());
                }
            }
        }
        Language::Php => {
            // Pattern: throw new ExceptionType(...)
            let re = Regex::new(r"throw\s+new\s+\\?([A-Z][a-zA-Z0-9_\\]*)\s*\(").unwrap();
            for cap in re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    errors.insert(error_type.as_str().to_string());
                }
            }
        }
        Language::Ruby => {
            // Pattern: raise ErrorType, "message" or raise ErrorType.new(...)
            let re = Regex::new(r"raise\s+([A-Z][a-zA-Z0-9_:]*)").unwrap();
            for cap in re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    errors.insert(error_type.as_str().to_string());
                }
            }
        }
        Language::Dart => {
            // Pattern: throw ErrorType(...) or throw Exception(...)
            let re = Regex::new(r"throw\s+([A-Z][a-zA-Z0-9_]*)\s*\(").unwrap();
            for cap in re.captures_iter(code) {
                if let Some(error_type) = cap.get(1) {
                    errors.insert(error_type.as_str().to_string());
                }
            }
        }
        _ => {
            // Unsupported language - return empty
        }
    }

    let mut result: Vec<String> = errors.into_iter().collect();
    result.sort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_thrown_errors_typescript() {
        let source = r#"
function validate(input: string): void {
    if (!input) {
        throw new ValidationError("Input required");
    }
    if (input.length > 100) {
        throw new LengthError("Too long");
    }
}
"#;

        let errors = extract_thrown_errors(source, 1, 10, Language::TypeScript);

        assert_eq!(errors.len(), 2);
        assert!(errors.contains(&"ValidationError".to_string()));
        assert!(errors.contains(&"LengthError".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_typescript_without_new() {
        let source = r#"
function test() {
    throw Error("message");
}
"#;

        let errors = extract_thrown_errors(source, 1, 5, Language::TypeScript);
        assert!(errors.contains(&"Error".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_rust() {
        let source = r#"
fn validate(input: &str) -> Result<(), AppError> {
    if input.is_empty() {
        return Err(AppError::ValidationError("Input required".into()));
    }
    Ok(())
}
"#;

        let errors = extract_thrown_errors(source, 1, 8, Language::Rust);

        assert!(errors.contains(&"AppError::ValidationError".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_rust_simple() {
        let source = r#"
fn test() -> Result<(), MyError> {
    Err(MyError("failed"))
}
"#;

        let errors = extract_thrown_errors(source, 1, 5, Language::Rust);
        assert!(errors.contains(&"MyError".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_python() {
        let source = r#"
def validate(input):
    if not input:
        raise ValueError("Input required")
    if len(input) > 100:
        raise ValidationError("Too long")
"#;

        let errors = extract_thrown_errors(source, 1, 7, Language::Python);

        assert_eq!(errors.len(), 2);
        assert!(errors.contains(&"ValueError".to_string()));
        assert!(errors.contains(&"ValidationError".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_java() {
        let source = r#"
public void validate(String input) {
    if (input == null) {
        throw new NullPointerException("Input cannot be null");
    }
    if (input.isEmpty()) {
        throw new IllegalArgumentException("Input cannot be empty");
    }
}
"#;

        let errors = extract_thrown_errors(source, 1, 10, Language::Java);

        assert_eq!(errors.len(), 2);
        assert!(errors.contains(&"NullPointerException".to_string()));
        assert!(errors.contains(&"IllegalArgumentException".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_go() {
        let source = r#"
func validate(input string) error {
    if input == "" {
        return errors.New("input required")
    }
    return nil
}
"#;

        let errors = extract_thrown_errors(source, 1, 8, Language::Go);
        assert!(errors.contains(&"error".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_swift() {
        let source = r#"
func validate(input: String) throws {
    guard !input.isEmpty else {
        throw ValidationError.emptyInput
    }
}
"#;

        let errors = extract_thrown_errors(source, 1, 7, Language::Swift);
        assert!(errors.contains(&"ValidationError.emptyInput".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_no_errors() {
        let source = r#"
function add(a: number, b: number): number {
    return a + b;
}
"#;

        let errors = extract_thrown_errors(source, 1, 5, Language::TypeScript);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_extract_thrown_errors_deduplicates() {
        let source = r#"
function test() {
    if (a) throw new MyError("a");
    if (b) throw new MyError("b");
    if (c) throw new MyError("c");
}
"#;

        let errors = extract_thrown_errors(source, 1, 7, Language::TypeScript);
        assert_eq!(errors.len(), 1);
        assert!(errors.contains(&"MyError".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_invalid_range() {
        let source = "function test() {}";
        let errors = extract_thrown_errors(source, 100, 200, Language::TypeScript);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_extract_thrown_errors_php() {
        let source = r#"
function validate($input) {
    if (!$input) {
        throw new \InvalidArgumentException("Input required");
    }
}
"#;

        let errors = extract_thrown_errors(source, 1, 7, Language::Php);
        assert!(errors.contains(&"InvalidArgumentException".to_string()));
    }

    #[test]
    fn test_extract_thrown_errors_ruby() {
        let source = r#"
def validate(input)
  raise ArgumentError, "Input required" if input.nil?
  raise ValidationError.new("Too long") if input.length > 100
end
"#;

        let errors = extract_thrown_errors(source, 1, 6, Language::Ruby);
        assert!(errors.contains(&"ArgumentError".to_string()));
        assert!(errors.contains(&"ValidationError".to_string()));
    }
}
