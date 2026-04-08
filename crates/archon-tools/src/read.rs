use anyhow::Result;
use archon_core::Tool;
use async_trait::async_trait;
use serde_json::json;
use tokio::fs;

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Returns file content with line numbers (cat -n format). \
         You can optionally specify an offset (line number to start from, 1-based) and a limit \
         (number of lines to read)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute or relative path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-based). Defaults to 1."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read. Defaults to reading all lines."
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let file_path = input["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        let offset = input["offset"].as_u64().unwrap_or(1).max(1) as usize;
        let limit = input["limit"].as_u64().map(|l| l as usize);

        let content = fs::read_to_string(file_path).await.map_err(|e| {
            anyhow::anyhow!("Failed to read file '{file_path}': {e}")
        })?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let start = (offset - 1).min(total_lines);
        let end = match limit {
            Some(l) => (start + l).min(total_lines),
            None => total_lines,
        };

        let mut result = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;
            result.push_str(&format!("{line_num:>6}\t{line}\n"));
        }

        if result.is_empty() {
            result = format!("(file '{file_path}' is empty or offset is beyond end of file)\n");
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::Tool;
    use serde_json::json;
    use std::io::Write;

    #[tokio::test]
    async fn test_read_full_file() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "line1").unwrap();
        writeln!(f, "line2").unwrap();
        writeln!(f, "line3").unwrap();
        let path = f.path().to_str().unwrap();

        let tool = ReadTool;
        let result = tool.execute(json!({"file_path": path})).await.unwrap();
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(result.contains("line3"));
    }

    #[tokio::test]
    async fn test_read_with_offset_and_limit() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        for i in 1..=10 {
            writeln!(f, "line{i}").unwrap();
        }
        let path = f.path().to_str().unwrap();

        let tool = ReadTool;
        let result = tool
            .execute(json!({"file_path": path, "offset": 3, "limit": 2}))
            .await
            .unwrap();
        assert!(result.contains("line3"));
        assert!(result.contains("line4"));
        assert!(!result.contains("line5"));
        assert!(!result.contains("line2"));
    }

    #[tokio::test]
    async fn test_read_empty_file() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap();

        let tool = ReadTool;
        let result = tool.execute(json!({"file_path": path})).await.unwrap();
        assert!(result.contains("empty"));
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let tool = ReadTool;
        let result = tool.execute(json!({"file_path": "/nonexistent/file.txt"})).await;
        assert!(result.is_err());
    }
}
