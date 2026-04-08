use anyhow::Result;
use archon_core::Tool;
use async_trait::async_trait;
use serde_json::json;

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Supports patterns like \"**/*.rs\", \"src/**/*.ts\". \
         Returns matching file paths sorted alphabetically, one per line."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against (e.g. \"**/*.rs\", \"src/**/*.ts\")"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in. Defaults to current directory."
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let pattern = input["pattern"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

        let base_path = input["path"].as_str().unwrap_or(".");

        let full_pattern = if base_path == "." {
            pattern.to_string()
        } else {
            format!("{}/{}", base_path.trim_end_matches('/'), pattern)
        };

        let entries = glob::glob(&full_pattern)
            .map_err(|e| anyhow::anyhow!("Invalid glob pattern '{full_pattern}': {e}"))?;

        let mut paths: Vec<String> = Vec::new();
        for entry in entries {
            match entry {
                Ok(path) => paths.push(path.display().to_string()),
                Err(e) => eprintln!("Glob error: {e}"),
            }
        }

        paths.sort();

        if paths.is_empty() {
            Ok(format!("No files matched pattern '{full_pattern}'."))
        } else {
            let count = paths.len();
            let listing = paths.join("\n");
            Ok(format!("{listing}\n\n({count} files matched)"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::Tool;
    use serde_json::json;

    #[tokio::test]
    async fn test_glob_matches_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        std::fs::write(dir.path().join("b.rs"), "").unwrap();
        std::fs::write(dir.path().join("c.txt"), "").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let tool = GlobTool;
        let result = tool
            .execute(json!({"pattern": "*.rs", "path": path_str}))
            .await
            .unwrap();
        assert!(result.contains("a.rs"));
        assert!(result.contains("b.rs"));
        assert!(!result.contains("c.txt"));
        assert!(result.contains("2 files matched"));
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path_str = dir.path().to_str().unwrap();

        let tool = GlobTool;
        let result = tool
            .execute(json!({"pattern": "*.xyz", "path": path_str}))
            .await
            .unwrap();
        assert!(result.contains("No files matched"));
    }

    #[tokio::test]
    async fn test_glob_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("file.rs"), "").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let tool = GlobTool;
        let result = tool
            .execute(json!({"pattern": "**/*.rs", "path": path_str}))
            .await
            .unwrap();
        assert!(result.contains("file.rs"));
    }

    #[tokio::test]
    async fn test_glob_invalid_pattern() {
        let tool = GlobTool;
        let result = tool.execute(json!({"pattern": "[invalid"})).await;
        assert!(result.is_err());
    }
}
