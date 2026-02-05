//! Test file detection and test function discovery (E6a, E6)
//!
//! Detects test files based on naming conventions and finds test function
//! names that reference specific symbols.

use codegraph_types::Language;
use regex::Regex;

/// Check if a file path represents a test file based on naming conventions
///
/// Conventions vary by language:
/// - TypeScript/JavaScript: `*.test.ts`, `*.spec.ts`, `__tests__/*`
/// - Rust: `*_test.rs`, `tests/*.rs`
/// - Python: `test_*.py`, `*_test.py`
/// - Go: `*_test.go`
/// - Java: `*Test.java`, `*Tests.java`
/// - C#: `*Tests.cs`, `*Test.cs`
pub fn is_test_file(path: &str, language: Language) -> bool {
    let path_lower = path.to_lowercase();

    match language {
        Language::TypeScript | Language::JavaScript | Language::Tsx | Language::Jsx => {
            // *.test.ts, *.spec.ts, *.test.tsx, *.spec.tsx
            // __tests__/*, __mocks__/*
            path_lower.contains(".test.") ||
            path_lower.contains(".spec.") ||
            path_lower.contains("__tests__") ||
            path_lower.contains("__mocks__")
        }
        Language::Rust => {
            // *_test.rs, tests/*.rs
            path_lower.ends_with("_test.rs") ||
            path_lower.contains("/tests/") ||
            path_lower.starts_with("tests/")
        }
        Language::Python => {
            // test_*.py, *_test.py, tests/*.py, conftest.py
            let filename = path.rsplit('/').next().unwrap_or(path);
            let filename_lower = filename.to_lowercase();
            filename_lower.starts_with("test_") ||
            filename_lower.ends_with("_test.py") ||
            filename_lower == "conftest.py" ||
            path_lower.contains("/tests/") ||
            path_lower.starts_with("tests/")
        }
        Language::Go => {
            // *_test.go
            path_lower.ends_with("_test.go")
        }
        Language::Java => {
            // *Test.java, *Tests.java, *IT.java (integration tests)
            path_lower.ends_with("test.java") ||
            path_lower.ends_with("tests.java") ||
            path_lower.ends_with("it.java") ||
            path_lower.contains("/test/")
        }
        Language::CSharp => {
            // *Test.cs, *Tests.cs, *.Tests/*.cs
            path_lower.ends_with("test.cs") ||
            path_lower.ends_with("tests.cs") ||
            path_lower.contains(".tests/") ||
            path_lower.contains(".test/")
        }
        Language::Kotlin => {
            // *Test.kt, *Tests.kt
            path_lower.ends_with("test.kt") ||
            path_lower.ends_with("tests.kt") ||
            path_lower.contains("/test/")
        }
        Language::Swift => {
            // *Tests.swift, *Test.swift
            path_lower.ends_with("tests.swift") ||
            path_lower.ends_with("test.swift") ||
            path_lower.contains("/tests/") ||
            path_lower.contains("tests/")
        }
        Language::Php => {
            // *Test.php, tests/*.php
            path_lower.ends_with("test.php") ||
            path_lower.contains("/tests/") ||
            path_lower.starts_with("tests/")
        }
        Language::Ruby => {
            // *_test.rb, *_spec.rb, test_*.rb, spec/*.rb
            let filename = path.rsplit('/').next().unwrap_or(path);
            let filename_lower = filename.to_lowercase();
            filename_lower.ends_with("_test.rb") ||
            filename_lower.ends_with("_spec.rb") ||
            filename_lower.starts_with("test_") ||
            path_lower.contains("/test/") ||
            path_lower.contains("/spec/") ||
            path_lower.starts_with("test/") ||
            path_lower.starts_with("spec/")
        }
        Language::Dart => {
            // *_test.dart, test/*.dart
            path_lower.ends_with("_test.dart") ||
            path_lower.contains("/test/") ||
            path_lower.starts_with("test/")
        }
        _ => {
            // Generic fallbacks
            path_lower.contains("/tests/") ||
            path_lower.contains("/test/") ||
            path_lower.contains("_test.") ||
            path_lower.contains(".test.")
        }
    }
}

/// Find test function/block names that reference a specific symbol
///
/// Returns a list of test names that likely test the given symbol.
/// For TypeScript/JavaScript, this looks for `describe('SymbolName', ...)` or
/// `it('should ... SymbolName ...', ...)` patterns.
pub fn find_test_names_for_symbol(source: &str, symbol_name: &str, language: Language) -> Vec<String> {
    let mut tests: Vec<String> = Vec::new();

    match language {
        Language::TypeScript | Language::JavaScript | Language::Tsx | Language::Jsx => {
            // Look for describe blocks mentioning the symbol
            let describe_re = Regex::new(r#"describe\s*\(\s*['"`]([^'"`]+)['"`]"#).unwrap();
            for cap in describe_re.captures_iter(source) {
                if let Some(name) = cap.get(1) {
                    let name_str = name.as_str();
                    // Check if this describe block is for our symbol
                    if name_str.contains(symbol_name) {
                        // Find all it/test blocks within the general vicinity
                        // This is a simplification - full implementation would parse AST
                        let it_re = Regex::new(r#"(?:it|test)\s*\(\s*['"`]([^'"`]+)['"`]"#).unwrap();
                        for it_cap in it_re.captures_iter(source) {
                            if let Some(test_name) = it_cap.get(1) {
                                tests.push(test_name.as_str().to_string());
                            }
                        }
                    }
                }
            }

            // Also look for standalone test/it blocks that mention the symbol
            if tests.is_empty() {
                let it_re = Regex::new(r#"(?:it|test)\s*\(\s*['"`]([^'"`]+)['"`]"#).unwrap();
                for cap in it_re.captures_iter(source) {
                    if let Some(test_name) = cap.get(1) {
                        let name_str = test_name.as_str();
                        if name_str.to_lowercase().contains(&symbol_name.to_lowercase()) {
                            tests.push(name_str.to_string());
                        }
                    }
                }
            }
        }
        Language::Rust => {
            // Look for #[test] fn test_symbol_name() or fn test_*_symbol_name()
            let test_fn_re = Regex::new(r#"#\[test\]\s*(?:#\[[\w\(\)]+\]\s*)*fn\s+(\w+)"#).unwrap();
            for cap in test_fn_re.captures_iter(source) {
                if let Some(name) = cap.get(1) {
                    let name_str = name.as_str();
                    if name_str.to_lowercase().contains(&symbol_name.to_lowercase()) {
                        tests.push(name_str.to_string());
                    }
                }
            }
        }
        Language::Python => {
            // Look for def test_*() or class Test*
            let test_fn_re = Regex::new(r#"def\s+(test_\w+)\s*\("#).unwrap();
            for cap in test_fn_re.captures_iter(source) {
                if let Some(name) = cap.get(1) {
                    let name_str = name.as_str();
                    if name_str.to_lowercase().contains(&symbol_name.to_lowercase()) {
                        tests.push(name_str.to_string());
                    }
                }
            }

            // Also check pytest parametrize markers
            let class_re = Regex::new(r#"class\s+(Test\w*)"#).unwrap();
            for cap in class_re.captures_iter(source) {
                if let Some(name) = cap.get(1) {
                    let name_str = name.as_str();
                    if name_str.to_lowercase().contains(&symbol_name.to_lowercase()) {
                        tests.push(name_str.to_string());
                    }
                }
            }
        }
        Language::Go => {
            // Look for func Test*(t *testing.T)
            let test_fn_re = Regex::new(r#"func\s+(Test\w+)\s*\("#).unwrap();
            for cap in test_fn_re.captures_iter(source) {
                if let Some(name) = cap.get(1) {
                    let name_str = name.as_str();
                    if name_str.to_lowercase().contains(&symbol_name.to_lowercase()) {
                        tests.push(name_str.to_string());
                    }
                }
            }
        }
        Language::Java | Language::Kotlin => {
            // Look for @Test annotation followed by method
            let test_fn_re = Regex::new(r#"@Test\s*(?:\([^)]*\))?\s*(?:public\s+|private\s+|protected\s+)?(?:fun|void|static\s+void)\s+(\w+)"#).unwrap();
            for cap in test_fn_re.captures_iter(source) {
                if let Some(name) = cap.get(1) {
                    let name_str = name.as_str();
                    if name_str.to_lowercase().contains(&symbol_name.to_lowercase()) {
                        tests.push(name_str.to_string());
                    }
                }
            }
        }
        _ => {
            // Generic: look for functions starting with test_ or ending with _test
            let generic_re = Regex::new(r#"(?:fn|func|def|function)\s+((?:test_\w+|\w+_test))\s*\("#).unwrap();
            for cap in generic_re.captures_iter(source) {
                if let Some(name) = cap.get(1) {
                    let name_str = name.as_str();
                    if name_str.to_lowercase().contains(&symbol_name.to_lowercase()) {
                        tests.push(name_str.to_string());
                    }
                }
            }
        }
    }

    // Deduplicate and sort
    tests.sort();
    tests.dedup();
    tests
}

/// Represents an import extracted from source code
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportInfo {
    /// The imported name(s) (e.g., "UserService" from `import { UserService }`)
    pub names: Vec<String>,
    /// The import path/module (e.g., "../services/user" or "lodash")
    pub path: String,
}

/// Extract imports from source code
///
/// Returns a list of import statements with their paths and imported names.
pub fn extract_imports(source: &str, language: Language) -> Vec<ImportInfo> {
    let mut imports = Vec::new();

    match language {
        Language::TypeScript | Language::JavaScript | Language::Tsx | Language::Jsx => {
            // ES6 imports: import { A, B } from 'path'
            // Also: import A from 'path', import * as A from 'path'
            let named_re = Regex::new(r#"import\s*\{\s*([^}]+)\s*\}\s*from\s*['"]([^'"]+)['"]"#).unwrap();
            for cap in named_re.captures_iter(source) {
                if let (Some(names), Some(path)) = (cap.get(1), cap.get(2)) {
                    let imported_names: Vec<String> = names.as_str()
                        .split(',')
                        .map(|s| s.split(" as ").next().unwrap_or(s).trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    imports.push(ImportInfo {
                        names: imported_names,
                        path: path.as_str().to_string(),
                    });
                }
            }

            // Default imports: import A from 'path'
            let default_re = Regex::new(r#"import\s+(\w+)\s+from\s*['"]([^'"]+)['"]"#).unwrap();
            for cap in default_re.captures_iter(source) {
                if let (Some(name), Some(path)) = (cap.get(1), cap.get(2)) {
                    imports.push(ImportInfo {
                        names: vec![name.as_str().to_string()],
                        path: path.as_str().to_string(),
                    });
                }
            }

            // Require statements: const A = require('path')
            let require_re = Regex::new(r#"(?:const|let|var)\s+(?:\{([^}]+)\}|(\w+))\s*=\s*require\s*\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap();
            for cap in require_re.captures_iter(source) {
                let path = cap.get(3).map(|m| m.as_str().to_string()).unwrap_or_default();
                let names = if let Some(destructured) = cap.get(1) {
                    destructured.as_str()
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                } else if let Some(name) = cap.get(2) {
                    vec![name.as_str().to_string()]
                } else {
                    vec![]
                };
                if !path.is_empty() {
                    imports.push(ImportInfo { names, path });
                }
            }
        }
        Language::Python => {
            // from module import A, B
            let from_re = Regex::new(r#"from\s+([\w.]+)\s+import\s+([^#\n]+)"#).unwrap();
            for cap in from_re.captures_iter(source) {
                if let (Some(module), Some(names)) = (cap.get(1), cap.get(2)) {
                    let imported_names: Vec<String> = names.as_str()
                        .split(',')
                        .map(|s| s.split(" as ").next().unwrap_or(s).trim().to_string())
                        .filter(|s| !s.is_empty() && s != "*")
                        .collect();
                    imports.push(ImportInfo {
                        names: imported_names,
                        path: module.as_str().to_string(),
                    });
                }
            }

            // import module
            let import_re = Regex::new(r#"^import\s+([\w.]+)"#).unwrap();
            for cap in import_re.captures_iter(source) {
                if let Some(module) = cap.get(1) {
                    let name = module.as_str().split('.').last().unwrap_or(module.as_str());
                    imports.push(ImportInfo {
                        names: vec![name.to_string()],
                        path: module.as_str().to_string(),
                    });
                }
            }
        }
        Language::Rust => {
            // use crate::module::{Item1, Item2};
            let braced_re = Regex::new(r#"use\s+([\w:]+)::\{([^}]+)\}"#).unwrap();
            for cap in braced_re.captures_iter(source) {
                if let (Some(path), Some(items)) = (cap.get(1), cap.get(2)) {
                    let names: Vec<String> = items.as_str()
                        .split(',')
                        .map(|s| s.split(" as ").next().unwrap_or(s).trim().to_string())
                        .filter(|s| !s.is_empty() && s != "self")
                        .collect();
                    imports.push(ImportInfo {
                        names,
                        path: path.as_str().to_string(),
                    });
                }
            }

            // use crate::module::Item;
            // use super::Item;
            let simple_re = Regex::new(r#"use\s+([\w:]+);"#).unwrap();
            for cap in simple_re.captures_iter(source) {
                if let Some(full_path) = cap.get(1) {
                    let path_str = full_path.as_str();
                    // Skip if it matches the braced pattern (already handled)
                    if path_str.contains('{') {
                        continue;
                    }
                    let last = path_str.split("::").last().unwrap_or("");
                    if !last.is_empty() {
                        imports.push(ImportInfo {
                            names: vec![last.to_string()],
                            path: path_str.to_string(),
                        });
                    }
                }
            }
        }
        Language::Go => {
            // import "path" or import alias "path"
            let import_re = Regex::new(r#"import\s+(?:(\w+)\s+)?["']([^"']+)["']"#).unwrap();
            for cap in import_re.captures_iter(source) {
                let path = cap.get(2).map(|m| m.as_str()).unwrap_or("");
                let name = cap.get(1)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string());
                imports.push(ImportInfo {
                    names: vec![name],
                    path: path.to_string(),
                });
            }

            // import block
            let block_re = Regex::new(r#"import\s*\(([^)]+)\)"#).unwrap();
            for cap in block_re.captures_iter(source) {
                if let Some(block) = cap.get(1) {
                    let line_re = Regex::new(r#"(?:(\w+)\s+)?["']([^"']+)["']"#).unwrap();
                    for line_cap in line_re.captures_iter(block.as_str()) {
                        let path = line_cap.get(2).map(|m| m.as_str()).unwrap_or("");
                        let name = line_cap.get(1)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string());
                        imports.push(ImportInfo {
                            names: vec![name],
                            path: path.to_string(),
                        });
                    }
                }
            }
        }
        _ => {
            // Generic fallback for other languages - try common patterns
        }
    }

    imports
}

/// Method used to associate tests with a symbol
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociationMethod {
    /// Matched via file naming convention (e.g., user.test.ts tests user.ts)
    Convention,
    /// Matched via import analysis
    Import,
    /// No association found
    None,
}

/// Result of test association lookup
#[derive(Debug, Clone)]
pub struct TestAssociation {
    /// The test names found
    pub test_names: Vec<String>,
    /// How the association was determined
    pub method: AssociationMethod,
}

/// Check if a test file path likely tests a source file path by convention
fn test_file_matches_source_by_convention(test_file: &str, source_file: &str) -> bool {
    // Extract base names without extension
    let test_base = test_file
        .rsplit('/')
        .next()
        .unwrap_or(test_file)
        .split('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
        .replace("_test", "")
        .replace(".test", "")
        .replace("_spec", "")
        .replace(".spec", "")
        .replace("test_", "")
        .replace("tests", "");

    let source_base = source_file
        .rsplit('/')
        .next()
        .unwrap_or(source_file)
        .split('.')
        .next()
        .unwrap_or("")
        .to_lowercase();

    !test_base.is_empty() && !source_base.is_empty() &&
    (test_base == source_base || test_base.contains(&source_base) || source_base.contains(&test_base))
}

/// Associate tests from a test file with a source file via import analysis
///
/// Returns test names from the test source that import from the specified source file.
pub fn associate_tests_via_imports(
    test_source: &str,
    source_file_path: &str,
    language: Language,
) -> Vec<String> {
    let imports = extract_imports(test_source, language);

    // Check if any import matches the source file path
    let source_base = source_file_path
        .rsplit('/')
        .next()
        .unwrap_or(source_file_path)
        .split('.')
        .next()
        .unwrap_or("");

    let has_matching_import = imports.iter().any(|import| {
        let import_base = import.path
            .rsplit('/')
            .next()
            .unwrap_or(&import.path)
            .split('.')
            .next()
            .unwrap_or("");

        import_base.to_lowercase() == source_base.to_lowercase() ||
        import.path.to_lowercase().contains(&source_base.to_lowercase())
    });

    if has_matching_import {
        // Extract all test names from this file
        // For describe/it blocks, find all tests
        let it_re = Regex::new(r#"(?:it|test)\s*\(\s*['"`]([^'"`]+)['"`]"#).unwrap();
        let mut tests: Vec<String> = it_re.captures_iter(test_source)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
            .collect();
        tests.sort();
        tests.dedup();
        tests
    } else {
        Vec::new()
    }
}

/// Find tests for a symbol using hybrid approach (convention first, then imports)
///
/// E6: Uses naming conventions as primary method, falls back to import analysis.
pub fn find_tests_for_symbol(
    symbol_name: &str,
    source_file: &str,
    test_file: &str,
    test_source: &str,
    language: Language,
) -> TestAssociation {
    // First try convention-based matching
    if test_file_matches_source_by_convention(test_file, source_file) {
        let tests = find_test_names_for_symbol(test_source, symbol_name, language);
        if !tests.is_empty() {
            return TestAssociation {
                test_names: tests,
                method: AssociationMethod::Convention,
            };
        }
    }

    // Fall back to import-based matching
    let tests = associate_tests_via_imports(test_source, source_file, language);
    if !tests.is_empty() {
        return TestAssociation {
            test_names: tests,
            method: AssociationMethod::Import,
        };
    }

    TestAssociation {
        test_names: Vec::new(),
        method: AssociationMethod::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === is_test_file tests ===

    #[test]
    fn test_detect_test_file_typescript() {
        assert!(is_test_file("src/utils.test.ts", Language::TypeScript));
        assert!(is_test_file("src/utils.spec.ts", Language::TypeScript));
        assert!(is_test_file("__tests__/utils.ts", Language::TypeScript));
        assert!(!is_test_file("src/utils.ts", Language::TypeScript));
    }

    #[test]
    fn test_detect_test_file_javascript() {
        assert!(is_test_file("src/utils.test.js", Language::JavaScript));
        assert!(is_test_file("src/utils.spec.js", Language::JavaScript));
        assert!(is_test_file("__tests__/utils.js", Language::JavaScript));
        assert!(!is_test_file("src/utils.js", Language::JavaScript));
    }

    #[test]
    fn test_detect_test_file_tsx() {
        assert!(is_test_file("src/Component.test.tsx", Language::Tsx));
        assert!(is_test_file("src/Component.spec.tsx", Language::Tsx));
        assert!(!is_test_file("src/Component.tsx", Language::Tsx));
    }

    #[test]
    fn test_detect_test_file_rust() {
        assert!(is_test_file("src/utils_test.rs", Language::Rust));
        assert!(is_test_file("tests/integration.rs", Language::Rust));
        assert!(!is_test_file("src/utils.rs", Language::Rust));
    }

    #[test]
    fn test_detect_test_file_python() {
        assert!(is_test_file("test_utils.py", Language::Python));
        assert!(is_test_file("tests/test_api.py", Language::Python));
        assert!(is_test_file("utils_test.py", Language::Python));
        assert!(is_test_file("conftest.py", Language::Python));
        assert!(!is_test_file("utils.py", Language::Python));
    }

    #[test]
    fn test_detect_test_file_go() {
        assert!(is_test_file("utils_test.go", Language::Go));
        assert!(is_test_file("main_test.go", Language::Go));
        assert!(!is_test_file("main.go", Language::Go));
    }

    #[test]
    fn test_detect_test_file_java() {
        assert!(is_test_file("UserServiceTest.java", Language::Java));
        assert!(is_test_file("UserServiceTests.java", Language::Java));
        assert!(is_test_file("src/test/java/Service.java", Language::Java));
        assert!(!is_test_file("UserService.java", Language::Java));
    }

    #[test]
    fn test_detect_test_file_csharp() {
        assert!(is_test_file("UserServiceTest.cs", Language::CSharp));
        assert!(is_test_file("UserServiceTests.cs", Language::CSharp));
        assert!(is_test_file("MyApp.Tests/ServiceTests.cs", Language::CSharp));
        assert!(!is_test_file("UserService.cs", Language::CSharp));
    }

    #[test]
    fn test_detect_test_file_kotlin() {
        assert!(is_test_file("UserServiceTest.kt", Language::Kotlin));
        assert!(is_test_file("src/test/kotlin/Service.kt", Language::Kotlin));
        assert!(!is_test_file("UserService.kt", Language::Kotlin));
    }

    #[test]
    fn test_detect_test_file_swift() {
        assert!(is_test_file("UserServiceTests.swift", Language::Swift));
        assert!(is_test_file("tests/ServiceTests.swift", Language::Swift));
        assert!(!is_test_file("UserService.swift", Language::Swift));
    }

    #[test]
    fn test_detect_test_file_php() {
        assert!(is_test_file("UserServiceTest.php", Language::Php));
        assert!(is_test_file("tests/ServiceTest.php", Language::Php));
        assert!(!is_test_file("UserService.php", Language::Php));
    }

    #[test]
    fn test_detect_test_file_ruby() {
        assert!(is_test_file("user_service_test.rb", Language::Ruby));
        assert!(is_test_file("user_service_spec.rb", Language::Ruby));
        assert!(is_test_file("spec/service_spec.rb", Language::Ruby));
        assert!(is_test_file("test/service_test.rb", Language::Ruby));
        assert!(!is_test_file("user_service.rb", Language::Ruby));
    }

    #[test]
    fn test_detect_test_file_dart() {
        assert!(is_test_file("widget_test.dart", Language::Dart));
        assert!(is_test_file("test/widget_test.dart", Language::Dart));
        assert!(!is_test_file("widget.dart", Language::Dart));
    }

    // === find_test_names_for_symbol tests ===

    #[test]
    fn test_find_test_functions_for_symbol_typescript() {
        let test_source = r#"
describe('UserService', () => {
    it('should create user', () => {});
    it('should delete user', () => {});
});
"#;

        let tests = find_test_names_for_symbol(test_source, "UserService", Language::TypeScript);
        assert_eq!(tests.len(), 2);
        assert!(tests.contains(&"should create user".to_string()));
        assert!(tests.contains(&"should delete user".to_string()));
    }

    #[test]
    fn test_find_test_functions_for_symbol_typescript_test() {
        let test_source = r#"
describe('Api', () => {
    test('creates user correctly', () => {});
    test('validates input', () => {});
});
"#;

        let tests = find_test_names_for_symbol(test_source, "Api", Language::TypeScript);
        assert!(tests.contains(&"creates user correctly".to_string()));
    }

    #[test]
    fn test_find_test_functions_for_symbol_rust() {
        let test_source = r#"
#[test]
fn test_user_creation() {
    let user = User::new();
}

#[test]
fn test_user_validation() {
    let user = User::validate();
}

#[test]
fn test_something_else() {
    // unrelated test
}
"#;

        let tests = find_test_names_for_symbol(test_source, "user", Language::Rust);
        assert_eq!(tests.len(), 2);
        assert!(tests.contains(&"test_user_creation".to_string()));
        assert!(tests.contains(&"test_user_validation".to_string()));
    }

    #[test]
    fn test_find_test_functions_for_symbol_python() {
        let test_source = r#"
def test_create_user():
    user = User()

def test_delete_user():
    pass

def test_something_else():
    pass
"#;

        let tests = find_test_names_for_symbol(test_source, "user", Language::Python);
        assert_eq!(tests.len(), 2);
        assert!(tests.contains(&"test_create_user".to_string()));
        assert!(tests.contains(&"test_delete_user".to_string()));
    }

    #[test]
    fn test_find_test_functions_for_symbol_python_class() {
        let test_source = r#"
class TestUserService:
    def test_create(self):
        pass
"#;

        let tests = find_test_names_for_symbol(test_source, "UserService", Language::Python);
        assert!(tests.contains(&"TestUserService".to_string()));
    }

    #[test]
    fn test_find_test_functions_for_symbol_go() {
        let test_source = r#"
func TestUserCreate(t *testing.T) {
    user := NewUser()
}

func TestUserDelete(t *testing.T) {
    // delete test
}

func TestSomethingElse(t *testing.T) {
    // unrelated
}
"#;

        let tests = find_test_names_for_symbol(test_source, "User", Language::Go);
        assert_eq!(tests.len(), 2);
        assert!(tests.contains(&"TestUserCreate".to_string()));
        assert!(tests.contains(&"TestUserDelete".to_string()));
    }

    #[test]
    fn test_find_test_no_matches() {
        let test_source = r#"
describe('Api', () => {
    it('should work', () => {});
});
"#;

        let tests = find_test_names_for_symbol(test_source, "User", Language::TypeScript);
        assert!(tests.is_empty());
    }

    #[test]
    fn test_find_test_deduplicates() {
        let test_source = r#"
describe('UserService', () => {
    it('should create user', () => {});
});
describe('UserService', () => {
    it('should create user', () => {});
});
"#;

        let tests = find_test_names_for_symbol(test_source, "UserService", Language::TypeScript);
        // Should deduplicate
        assert_eq!(tests.len(), 1);
    }

    // === extract_imports tests ===

    #[test]
    fn test_extract_imports_typescript_named() {
        let source = r#"
import { UserService, AuthService } from '../services/user';
import { Config } from './config';
"#;

        let imports = extract_imports(source, Language::TypeScript);
        assert_eq!(imports.len(), 2);
        assert!(imports[0].names.contains(&"UserService".to_string()));
        assert!(imports[0].names.contains(&"AuthService".to_string()));
        assert_eq!(imports[0].path, "../services/user");
    }

    #[test]
    fn test_extract_imports_typescript_default() {
        let source = r#"
import React from 'react';
import lodash from 'lodash';
"#;

        let imports = extract_imports(source, Language::TypeScript);
        assert!(imports.iter().any(|i| i.names.contains(&"React".to_string())));
        assert!(imports.iter().any(|i| i.path == "react"));
    }

    #[test]
    fn test_extract_imports_python() {
        let source = r#"
from services.user import UserService, AuthService
import logging
"#;

        let imports = extract_imports(source, Language::Python);
        assert!(imports.iter().any(|i| i.names.contains(&"UserService".to_string())));
        assert!(imports.iter().any(|i| i.path == "services.user"));
    }

    #[test]
    fn test_extract_imports_rust() {
        let source = r#"
use crate::services::user::{UserService, AuthService};
use super::config::Config;
use std::collections::HashMap;
"#;

        let imports = extract_imports(source, Language::Rust);
        assert!(imports.iter().any(|i| i.names.contains(&"UserService".to_string())));
        assert!(imports.iter().any(|i| i.names.contains(&"HashMap".to_string())));
    }

    #[test]
    fn test_extract_imports_go() {
        let source = r#"
import (
    "fmt"
    "github.com/user/project/services"
    svc "github.com/user/project/other"
)
"#;

        let imports = extract_imports(source, Language::Go);
        assert!(imports.iter().any(|i| i.path == "fmt"));
        assert!(imports.iter().any(|i| i.path == "github.com/user/project/services"));
    }

    // === associate_tests_via_imports tests ===

    #[test]
    fn test_associate_via_import() {
        let test_source = r#"
import { UserService } from '../services/user';

describe('UserService', () => {
    it('should work', () => {});
    it('should also work', () => {});
});
"#;

        let associations = associate_tests_via_imports(test_source, "services/user.ts", Language::TypeScript);
        assert!(associations.contains(&"should work".to_string()));
        assert!(associations.contains(&"should also work".to_string()));
    }

    #[test]
    fn test_associate_via_import_no_match() {
        let test_source = r#"
import { OtherService } from '../services/other';

describe('OtherService', () => {
    it('should work', () => {});
});
"#;

        let associations = associate_tests_via_imports(test_source, "services/user.ts", Language::TypeScript);
        assert!(associations.is_empty());
    }

    // === find_tests_for_symbol (hybrid) tests ===

    #[test]
    fn test_hybrid_association_prefers_convention() {
        let symbol_name = "UserService";
        let source_file = "src/services/user.ts";
        let test_file = "src/services/user.test.ts";
        let test_source = r#"
import { UserService } from './user';

describe('UserService', () => {
    it('should create user', () => {});
});
"#;

        let result = find_tests_for_symbol(symbol_name, source_file, test_file, test_source, Language::TypeScript);
        assert_eq!(result.method, AssociationMethod::Convention);
        assert!(result.test_names.contains(&"should create user".to_string()));
    }

    #[test]
    fn test_hybrid_association_falls_back_to_import() {
        let symbol_name = "Logger"; // Not in describe block
        let source_file = "src/utils/logger.ts";
        let test_file = "src/__tests__/integration.test.ts"; // Doesn't match by convention
        let test_source = r#"
import { Logger } from '../utils/logger';

describe('Integration', () => {
    it('should log correctly', () => {});
});
"#;

        let result = find_tests_for_symbol(symbol_name, source_file, test_file, test_source, Language::TypeScript);
        assert_eq!(result.method, AssociationMethod::Import);
        assert!(result.test_names.contains(&"should log correctly".to_string()));
    }

    #[test]
    fn test_hybrid_association_no_match() {
        let symbol_name = "SomeService";
        let source_file = "src/services/some.ts";
        let test_file = "src/__tests__/other.test.ts";
        let test_source = r#"
import { OtherService } from '../services/other';

describe('OtherService', () => {
    it('should work', () => {});
});
"#;

        let result = find_tests_for_symbol(symbol_name, source_file, test_file, test_source, Language::TypeScript);
        assert_eq!(result.method, AssociationMethod::None);
        assert!(result.test_names.is_empty());
    }

    #[test]
    fn test_test_file_matches_source_by_convention() {
        assert!(test_file_matches_source_by_convention("user.test.ts", "user.ts"));
        assert!(test_file_matches_source_by_convention("user_test.py", "user.py"));
        assert!(test_file_matches_source_by_convention("test_user.py", "user.py"));
        assert!(test_file_matches_source_by_convention("UserServiceTest.java", "UserService.java"));
        assert!(!test_file_matches_source_by_convention("other.test.ts", "user.ts"));
    }
}
