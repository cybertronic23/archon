use anyhow::Result;
use archon_core::Tool;
use async_trait::async_trait;
use serde_json::json;
use tokio::fs;

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Perform an exact string replacement in a file. The old_string must appear exactly once \
         in the file (unless replace_all is true). Provide the file_path, old_string to find, \
         and new_string to replace it with."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact string to find in the file"
                },
                "new_string": {
                    "type": "string",
                    "description": "The string to replace old_string with"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "If true, replace all occurrences. Default: false"
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let file_path = input["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        let old_string = input["old_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: old_string"))?;

        let new_string = input["new_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: new_string"))?;

        let replace_all = input["replace_all"].as_bool().unwrap_or(false);

        let content = fs::read_to_string(file_path).await.map_err(|e| {
            anyhow::anyhow!("Failed to read file '{file_path}': {e}")
        })?;

        if old_string == new_string {
            anyhow::bail!("old_string and new_string are identical — no change needed");
        }

        let count = content.matches(old_string).count();

        if count == 0 {
            anyhow::bail!(
                "old_string not found in '{file_path}'. Make sure the string matches exactly \
                 (including whitespace and indentation)."
            );
        }

        if !replace_all && count > 1 {
            anyhow::bail!(
                "old_string appears {count} times in '{file_path}'. Provide more surrounding \
                 context to make it unique, or set replace_all to true."
            );
        }

        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        fs::write(file_path, &new_content).await.map_err(|e| {
            anyhow::anyhow!("Failed to write file '{file_path}': {e}")
        })?;

        let replacements = if replace_all {
            format!("{count} replacement(s)")
        } else {
            "1 replacement".to_string()
        };
        Ok(format!(
            "Successfully edited '{file_path}' ({replacements})."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::Tool;
    use serde_json::json;

    #[tokio::test]
    async fn test_single_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world").unwrap();
        let path_str = path.to_str().unwrap();

        let tool = EditTool;
        let result = tool
            .execute(json!({
                "file_path": path_str,
                "old_string": "hello",
                "new_string": "goodbye"
            }))
            .await
            .unwrap();
        assert!(result.contains("Successfully edited"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "goodbye world");
    }

    #[tokio::test]
    async fn test_multiple_matches_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "aaa bbb aaa").unwrap();
        let path_str = path.to_str().unwrap();

        let tool = EditTool;
        let result = tool
            .execute(json!({
                "file_path": path_str,
                "old_string": "aaa",
                "new_string": "ccc"
            }))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("appears 2 times"));
    }

    #[tokio::test]
    async fn test_replace_all() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "aaa bbb aaa").unwrap();
        let path_str = path.to_str().unwrap();

        let tool = EditTool;
        let result = tool
            .execute(json!({
                "file_path": path_str,
                "old_string": "aaa",
                "new_string": "ccc",
                "replace_all": true
            }))
            .await
            .unwrap();
        assert!(result.contains("2 replacement(s)"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ccc bbb ccc");
    }

    #[tokio::test]
    async fn test_identical_strings_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello").unwrap();
        let path_str = path.to_str().unwrap();

        let tool = EditTool;
        let result = tool
            .execute(json!({
                "file_path": path_str,
                "old_string": "hello",
                "new_string": "hello"
            }))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("identical"));
    }

    #[tokio::test]
    async fn test_not_found_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world").unwrap();
        let path_str = path.to_str().unwrap();

        let tool = EditTool;
        let result = tool
            .execute(json!({
                "file_path": path_str,
                "old_string": "xyz",
                "new_string": "abc"
            }))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
