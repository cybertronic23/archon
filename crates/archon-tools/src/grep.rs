use std::path::Path;

use anyhow::Result;
use archon_core::Tool;
use async_trait::async_trait;
use regex::Regex;
use serde_json::json;

const MAX_RESULTS: usize = 100;
const MAX_FILE_SIZE: u64 = 1_048_576; // 1 MB

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".next",
    "__pycache__",
    ".mypy_cache",
    "dist",
    "build",
];

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents using a regex pattern. Recursively searches files in a directory. \
         Skips binary files, hidden directories (.git, etc.), and files larger than 1 MB. \
         Output format: file_path:line_number:matching_line. Limited to 100 results."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "The file or directory to search in. Defaults to current directory."
                },
                "include": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. \"*.rs\", \"*.{ts,tsx}\")"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let pattern_str = input["pattern"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

        let base_path = input["path"].as_str().unwrap_or(".");
        let include = input["include"].as_str();

        let regex = Regex::new(pattern_str)
            .map_err(|e| anyhow::anyhow!("Invalid regex pattern '{pattern_str}': {e}"))?;

        let include_glob = match include {
            Some(pat) => {
                let full = if base_path == "." {
                    format!("**/{pat}")
                } else {
                    format!("{}/**/{pat}", base_path.trim_end_matches('/'))
                };
                Some(
                    glob::Pattern::new(&full)
                        .map_err(|e| anyhow::anyhow!("Invalid include pattern '{pat}': {e}"))?,
                )
            }
            None => None,
        };

        let mut results = Vec::new();
        let path = Path::new(base_path);

        if path.is_file() {
            search_file(path, &regex, &mut results);
        } else {
            walk_dir(path, &regex, include_glob.as_ref(), &mut results);
        }

        if results.is_empty() {
            Ok(format!("No matches found for pattern '{pattern_str}'."))
        } else {
            let truncated = results.len() >= MAX_RESULTS;
            let output = results.join("\n");
            if truncated {
                Ok(format!(
                    "{output}\n\n(results truncated at {MAX_RESULTS} matches)"
                ))
            } else {
                let count = results.len();
                Ok(format!("{output}\n\n({count} matches)"))
            }
        }
    }
}

fn walk_dir(
    dir: &Path,
    regex: &Regex,
    include: Option<&glob::Pattern>,
    results: &mut Vec<String>,
) {
    if results.len() >= MAX_RESULTS {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        if results.len() >= MAX_RESULTS {
            return;
        }

        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if path.is_dir() {
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            walk_dir(&path, regex, include, results);
        } else if path.is_file() {
            // Check include filter
            if let Some(pat) = include {
                if !pat.matches_path(&path) {
                    continue;
                }
            }

            // Skip large files
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() > MAX_FILE_SIZE {
                    continue;
                }
            }

            search_file(&path, regex, results);
        }
    }
}

fn search_file(path: &Path, regex: &Regex, results: &mut Vec<String>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // skip binary / unreadable files
    };

    let display_path = path.display();
    for (line_num, line) in content.lines().enumerate() {
        if results.len() >= MAX_RESULTS {
            return;
        }
        if regex.is_match(line) {
            results.push(format!("{}:{}:{}", display_path, line_num + 1, line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::Tool;
    use serde_json::json;

    #[tokio::test]
    async fn test_grep_basic_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello world\nfoo bar\nhello again\n").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(json!({"pattern": "hello", "path": path_str}))
            .await
            .unwrap();
        assert!(result.contains("hello world"));
        assert!(result.contains("hello again"));
        assert!(result.contains("2 matches"));
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello world\n").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(json!({"pattern": "xyz", "path": path_str}))
            .await
            .unwrap();
        assert!(result.contains("No matches found"));
    }

    #[tokio::test]
    async fn test_grep_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "aaa\nbbb\nccc\nbbb\n").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(json!({"pattern": "bbb", "path": path_str}))
            .await
            .unwrap();
        assert!(result.contains(":2:bbb"));
        assert!(result.contains(":4:bbb"));
    }

    #[tokio::test]
    async fn test_grep_skips_hidden_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let hidden = dir.path().join(".hidden");
        std::fs::create_dir(&hidden).unwrap();
        std::fs::write(hidden.join("secret.txt"), "findme\n").unwrap();
        std::fs::write(dir.path().join("visible.txt"), "nothing here\n").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(json!({"pattern": "findme", "path": path_str}))
            .await
            .unwrap();
        assert!(result.contains("No matches found"));
    }

    #[tokio::test]
    async fn test_grep_include_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "match_me\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "match_me\n").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let tool = GrepTool;
        let result = tool
            .execute(json!({"pattern": "match_me", "path": path_str, "include": "*.rs"}))
            .await
            .unwrap();
        assert!(result.contains("a.rs"));
        assert!(!result.contains("b.txt"));
    }

    #[tokio::test]
    async fn test_grep_invalid_regex() {
        let tool = GrepTool;
        let result = tool.execute(json!({"pattern": "[invalid"})).await;
        assert!(result.is_err());
    }
}
