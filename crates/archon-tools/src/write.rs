use anyhow::Result;
use archon_core::Tool;
use async_trait::async_trait;
use serde_json::json;
use tokio::fs;

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. \
         Parent directories are created automatically."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let file_path = input["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        let content = input["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: content"))?;

        // Create parent directories if they don't exist
        if let Some(parent) = std::path::Path::new(file_path).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    anyhow::anyhow!("Failed to create parent directories for '{file_path}': {e}")
                })?;
            }
        }

        let bytes = content.len();
        fs::write(file_path, content).await.map_err(|e| {
            anyhow::anyhow!("Failed to write file '{file_path}': {e}")
        })?;

        Ok(format!("Successfully wrote {bytes} bytes to '{file_path}'."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::Tool;
    use serde_json::json;

    #[tokio::test]
    async fn test_write_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_str().unwrap();

        let tool = WriteTool;
        let result = tool
            .execute(json!({"file_path": path_str, "content": "hello world"}))
            .await
            .unwrap();
        assert!(result.contains("Successfully wrote"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_write_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/c/test.txt");
        let path_str = path.to_str().unwrap();

        let tool = WriteTool;
        tool.execute(json!({"file_path": path_str, "content": "nested"}))
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "nested");
    }

    #[tokio::test]
    async fn test_write_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_str().unwrap();
        std::fs::write(&path, "old content").unwrap();

        let tool = WriteTool;
        tool.execute(json!({"file_path": path_str, "content": "new content"}))
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");
    }

    #[tokio::test]
    async fn test_write_byte_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_str().unwrap();

        let tool = WriteTool;
        let result = tool
            .execute(json!({"file_path": path_str, "content": "12345"}))
            .await
            .unwrap();
        assert!(result.contains("5 bytes"));
    }
}
