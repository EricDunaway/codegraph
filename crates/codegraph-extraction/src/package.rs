//! Package name extraction (M1, M2, M3)
//!
//! Extracts package names from various manifest files (package.json, Cargo.toml, etc.)
//! and falls back to directory names when no manifest is found.

use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Result of package name extraction
#[derive(Debug, Clone)]
pub struct PackageInfo {
    /// The package name
    pub name: String,
    /// Where the name was found
    pub source: PackageSource,
}

/// Where the package name was found
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageSource {
    /// From package.json
    PackageJson,
    /// From Cargo.toml
    CargoToml,
    /// From pubspec.yaml (Dart/Flutter)
    PubspecYaml,
    /// From go.mod
    GoMod,
    /// From pyproject.toml
    PyprojectToml,
    /// From setup.py (legacy Python)
    SetupPy,
    /// From composer.json (PHP)
    ComposerJson,
    /// Fallback to directory name
    DirectoryName,
}

/// Extract package name for a file from its project root
///
/// Searches for manifest files in the project root and extracts the package name.
/// If no manifest is found, uses the directory name as a fallback.
///
/// # Arguments
/// * `project_root` - Path to the project root directory
/// * `file_path` - Relative path to the file (used for directory fallback)
///
/// # Returns
/// The package info if found, None if extraction fails
pub fn extract_package_name(project_root: &Path, _file_path: &str) -> Option<PackageInfo> {
    // Try package.json (JavaScript/TypeScript)
    if let Some(info) = try_package_json(project_root) {
        return Some(info);
    }

    // Try Cargo.toml (Rust)
    if let Some(info) = try_cargo_toml(project_root) {
        return Some(info);
    }

    // Try pubspec.yaml (Dart/Flutter)
    if let Some(info) = try_pubspec_yaml(project_root) {
        return Some(info);
    }

    // Try go.mod (Go)
    if let Some(info) = try_go_mod(project_root) {
        return Some(info);
    }

    // Try pyproject.toml (Python)
    if let Some(info) = try_pyproject_toml(project_root) {
        return Some(info);
    }

    // Try composer.json (PHP)
    if let Some(info) = try_composer_json(project_root) {
        return Some(info);
    }

    // Fallback to directory name (M2)
    fallback_to_directory(project_root)
}

/// Extract package name from nearest manifest, searching up the directory tree
///
/// This handles monorepo scenarios where a file might be in a sub-package.
pub fn extract_package_name_nearest(file_path: &Path) -> Option<PackageInfo> {
    let mut current = file_path.parent();

    while let Some(dir) = current {
        // Try to extract from this directory
        if let Some(info) = extract_package_name(dir, "") {
            if info.source != PackageSource::DirectoryName {
                return Some(info);
            }
        }
        current = dir.parent();
    }

    // Use directory name of the file as final fallback
    file_path.parent().and_then(fallback_to_directory)
}

/// Extract package name for a file within a project, handling workspaces (M3)
///
/// For monorepos/workspaces, this finds the nearest sub-package manifest
/// rather than the workspace root manifest.
///
/// # Arguments
/// * `project_root` - Root of the project/workspace
/// * `file_path` - Relative path to the file within the project
pub fn extract_package_for_file(project_root: &Path, file_path: &str) -> Option<PackageInfo> {
    // Construct absolute path to file's directory
    let full_path = project_root.join(file_path);
    let file_dir = full_path.parent()?;

    // Start from the file's directory and walk up to project root
    let mut current_dir = file_dir.to_path_buf();

    loop {
        // Don't go beyond project root
        if !current_dir.starts_with(project_root) {
            break;
        }

        // Try to find a manifest in this directory
        if let Some(info) = extract_package_name(&current_dir, "") {
            if info.source != PackageSource::DirectoryName {
                return Some(info);
            }
        }

        // If we've reached project root, stop
        if current_dir == project_root {
            break;
        }

        // Move up one directory
        if let Some(parent) = current_dir.parent() {
            current_dir = parent.to_path_buf();
        } else {
            break;
        }
    }

    // If nothing found in subdirectories, check project root again
    if let Some(info) = extract_package_name(project_root, file_path) {
        if info.source != PackageSource::DirectoryName {
            return Some(info);
        }
    }

    // Final fallback to directory name
    fallback_to_directory(project_root)
}

// --- Private extraction functions ---

#[derive(Deserialize)]
struct PackageJson {
    name: Option<String>,
    #[serde(default)]
    workspaces: Option<serde_json::Value>,
}

fn try_package_json(project_root: &Path) -> Option<PackageInfo> {
    let package_json = project_root.join("package.json");
    let content = fs::read_to_string(&package_json).ok()?;
    let parsed: PackageJson = serde_json::from_str(&content).ok()?;

    parsed.name.map(|name| PackageInfo {
        name,
        source: PackageSource::PackageJson,
    })
}

fn try_cargo_toml(project_root: &Path) -> Option<PackageInfo> {
    let cargo_toml = project_root.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml).ok()?;

    // Simple TOML parsing for [package] name = "..."
    // Note: In production, consider using the `toml` crate
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("name") && trimmed.contains('=') {
            // Extract value after =
            if let Some(value_part) = trimmed.split('=').nth(1) {
                let value = value_part.trim().trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    return Some(PackageInfo {
                        name: value.to_string(),
                        source: PackageSource::CargoToml,
                    });
                }
            }
        }
    }

    None
}

fn try_pubspec_yaml(project_root: &Path) -> Option<PackageInfo> {
    let pubspec = project_root.join("pubspec.yaml");
    let content = fs::read_to_string(&pubspec).ok()?;

    // Simple YAML parsing for name: ...
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("name:") {
            let value = trimmed.strip_prefix("name:")?.trim();
            if !value.is_empty() {
                return Some(PackageInfo {
                    name: value.to_string(),
                    source: PackageSource::PubspecYaml,
                });
            }
        }
    }

    None
}

fn try_go_mod(project_root: &Path) -> Option<PackageInfo> {
    let go_mod = project_root.join("go.mod");
    let content = fs::read_to_string(&go_mod).ok()?;

    // Parse module declaration: module github.com/user/repo
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("module ") {
            let module_path = trimmed.strip_prefix("module ")?.trim();
            // Use last segment as package name
            let name = module_path.rsplit('/').next().unwrap_or(module_path);
            return Some(PackageInfo {
                name: name.to_string(),
                source: PackageSource::GoMod,
            });
        }
    }

    None
}

fn try_pyproject_toml(project_root: &Path) -> Option<PackageInfo> {
    let pyproject = project_root.join("pyproject.toml");
    let content = fs::read_to_string(&pyproject).ok()?;

    // Look for name under [project] or [tool.poetry]
    let mut in_project = false;
    let mut in_poetry = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "[project]" {
            in_project = true;
            in_poetry = false;
        } else if trimmed == "[tool.poetry]" {
            in_poetry = true;
            in_project = false;
        } else if trimmed.starts_with('[') {
            in_project = false;
            in_poetry = false;
        } else if (in_project || in_poetry) && trimmed.starts_with("name") && trimmed.contains('=') {
            if let Some(value_part) = trimmed.split('=').nth(1) {
                let value = value_part.trim().trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    return Some(PackageInfo {
                        name: value.to_string(),
                        source: PackageSource::PyprojectToml,
                    });
                }
            }
        }
    }

    None
}

fn try_composer_json(project_root: &Path) -> Option<PackageInfo> {
    let composer = project_root.join("composer.json");
    let content = fs::read_to_string(&composer).ok()?;

    #[derive(Deserialize)]
    struct ComposerJson {
        name: Option<String>,
    }

    let parsed: ComposerJson = serde_json::from_str(&content).ok()?;
    parsed.name.map(|name| PackageInfo {
        name,
        source: PackageSource::ComposerJson,
    })
}

fn fallback_to_directory(project_root: &Path) -> Option<PackageInfo> {
    let dir_name = project_root.file_name()?.to_str()?;
    Some(PackageInfo {
        name: dir_name.to_string(),
        source: PackageSource::DirectoryName,
    })
}

/// Check if a package.json indicates a monorepo workspace root
pub fn is_workspace_root(project_root: &Path) -> bool {
    let package_json = project_root.join("package.json");
    if let Ok(content) = fs::read_to_string(&package_json) {
        if let Ok(parsed) = serde_json::from_str::<PackageJson>(&content) {
            return parsed.workspaces.is_some();
        }
    }

    // Also check for Cargo workspace
    let cargo_toml = project_root.join("Cargo.toml");
    if let Ok(content) = fs::read_to_string(&cargo_toml) {
        return content.contains("[workspace]");
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_extract_package_from_package_json() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("package.json"), r#"{"name": "@myorg/utils"}"#).unwrap();

        let result = extract_package_name(temp.path(), "src/index.ts");
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "@myorg/utils");
        assert_eq!(info.source, PackageSource::PackageJson);
    }

    #[test]
    fn test_extract_package_from_cargo_toml() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("Cargo.toml"), r#"
[package]
name = "my-crate"
version = "0.1.0"
"#).unwrap();

        let result = extract_package_name(temp.path(), "src/lib.rs");
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "my-crate");
        assert_eq!(info.source, PackageSource::CargoToml);
    }

    #[test]
    fn test_extract_package_from_pubspec_yaml() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("pubspec.yaml"), r#"
name: my_app
version: 1.0.0
"#).unwrap();

        let result = extract_package_name(temp.path(), "lib/main.dart");
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "my_app");
        assert_eq!(info.source, PackageSource::PubspecYaml);
    }

    #[test]
    fn test_extract_package_from_go_mod() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("go.mod"), r#"
module github.com/user/myproject

go 1.21
"#).unwrap();

        let result = extract_package_name(temp.path(), "main.go");
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "myproject");
        assert_eq!(info.source, PackageSource::GoMod);
    }

    #[test]
    fn test_extract_package_from_pyproject_toml() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("pyproject.toml"), r#"
[project]
name = "my-python-app"
version = "0.1.0"
"#).unwrap();

        let result = extract_package_name(temp.path(), "src/main.py");
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "my-python-app");
        assert_eq!(info.source, PackageSource::PyprojectToml);
    }

    #[test]
    fn test_extract_package_from_pyproject_toml_poetry() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("pyproject.toml"), r#"
[tool.poetry]
name = "poetry-app"
version = "0.1.0"
"#).unwrap();

        let result = extract_package_name(temp.path(), "src/main.py");
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "poetry-app");
        assert_eq!(info.source, PackageSource::PyprojectToml);
    }

    #[test]
    fn test_extract_package_from_composer_json() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("composer.json"), r#"{"name": "vendor/my-php-lib"}"#).unwrap();

        let result = extract_package_name(temp.path(), "src/index.php");
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "vendor/my-php-lib");
        assert_eq!(info.source, PackageSource::ComposerJson);
    }

    #[test]
    fn test_fallback_to_directory_name() {
        let temp = TempDir::new().unwrap();
        // No manifest files

        let result = extract_package_name(temp.path(), "src/index.ts");
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.source, PackageSource::DirectoryName);
        // Name should be temp dir name (something like tmpabc123)
        assert!(!info.name.is_empty());
    }

    #[test]
    fn test_extract_package_name_nearest() {
        let temp = TempDir::new().unwrap();

        // Create nested structure: root/sub/file.ts
        let sub_dir = temp.path().join("sub");
        fs::create_dir_all(&sub_dir).unwrap();
        fs::write(temp.path().join("package.json"), r#"{"name": "root-package"}"#).unwrap();

        let file_path = sub_dir.join("file.ts");
        fs::write(&file_path, "// code").unwrap();

        let result = extract_package_name_nearest(&file_path);
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "root-package");
    }

    #[test]
    fn test_is_workspace_root_npm() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("package.json"), r#"
{
    "name": "monorepo",
    "workspaces": ["packages/*"]
}
"#).unwrap();

        assert!(is_workspace_root(temp.path()));
    }

    #[test]
    fn test_is_workspace_root_cargo() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("Cargo.toml"), r#"
[workspace]
members = ["crates/*"]
"#).unwrap();

        assert!(is_workspace_root(temp.path()));
    }

    #[test]
    fn test_is_not_workspace_root() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("package.json"), r#"{"name": "single-package"}"#).unwrap();

        assert!(!is_workspace_root(temp.path()));
    }

    #[test]
    fn test_package_json_priority_over_cargo() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("package.json"), r#"{"name": "js-package"}"#).unwrap();
        fs::write(temp.path().join("Cargo.toml"), r#"
[package]
name = "rust-crate"
"#).unwrap();

        // package.json should take priority (checked first)
        let result = extract_package_name(temp.path(), "index.js");
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "js-package");
        assert_eq!(info.source, PackageSource::PackageJson);
    }

    // === Monorepo/Workspace tests (M3) ===

    #[test]
    fn test_monorepo_preserves_package_name() {
        let temp = TempDir::new().unwrap();

        // Root workspace
        fs::write(temp.path().join("package.json"), r#"
{
    "name": "monorepo-root",
    "workspaces": ["packages/*"]
}
"#).unwrap();

        // Sub-package
        fs::create_dir_all(temp.path().join("packages/utils")).unwrap();
        fs::write(temp.path().join("packages/utils/package.json"), r#"
{
    "name": "@myorg/utils"
}
"#).unwrap();

        // M3: Should preserve sub-package name, not root
        let result = extract_package_for_file(
            temp.path(),
            "packages/utils/src/index.ts"
        );
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "@myorg/utils");
        assert_eq!(info.source, PackageSource::PackageJson);
    }

    #[test]
    fn test_cargo_workspace_detection() {
        let temp = TempDir::new().unwrap();

        // Root workspace
        fs::write(temp.path().join("Cargo.toml"), r#"
[workspace]
members = ["crates/*"]
"#).unwrap();

        // Sub-crate
        fs::create_dir_all(temp.path().join("crates/utils/src")).unwrap();
        fs::write(temp.path().join("crates/utils/Cargo.toml"), r#"
[package]
name = "my-utils"
"#).unwrap();

        let result = extract_package_for_file(
            temp.path(),
            "crates/utils/src/lib.rs"
        );
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "my-utils");
        assert_eq!(info.source, PackageSource::CargoToml);
    }

    #[test]
    fn test_nested_workspaces() {
        let temp = TempDir::new().unwrap();

        // Root workspace
        fs::write(temp.path().join("package.json"), r#"{"name": "root", "workspaces": ["packages/*"]}"#).unwrap();

        // First level sub-package
        fs::create_dir_all(temp.path().join("packages/a")).unwrap();
        fs::write(temp.path().join("packages/a/package.json"), r#"{"name": "package-a"}"#).unwrap();

        // Deeply nested file without its own package.json
        fs::create_dir_all(temp.path().join("packages/a/src/deeply/nested")).unwrap();

        let result = extract_package_for_file(
            temp.path(),
            "packages/a/src/deeply/nested/file.ts"
        );
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.name, "package-a");
    }

    #[test]
    fn test_monorepo_root_file() {
        let temp = TempDir::new().unwrap();

        // Root workspace with a file at root level
        fs::write(temp.path().join("package.json"), r#"{"name": "monorepo-root", "workspaces": ["packages/*"]}"#).unwrap();

        let result = extract_package_for_file(
            temp.path(),
            "scripts/setup.js"
        );
        assert!(result.is_some());
        let info = result.unwrap();
        // File at root level should get root package name
        assert_eq!(info.name, "monorepo-root");
    }
}
