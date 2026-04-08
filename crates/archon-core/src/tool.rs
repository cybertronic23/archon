use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

use crate::types::ToolDefinition;

/// Every tool must implement this trait.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name used in tool_use blocks.
    fn name(&self) -> &str;

    /// Human-readable description for the LLM.
    fn description(&self) -> &str;

    /// JSON Schema describing the expected `input` object.
    fn input_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given JSON input and return output text.
    async fn execute(&self, input: serde_json::Value) -> Result<String>;

    /// Build the `ToolDefinition` sent to the API.
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.input_schema(),
        }
    }
}

/// Registry that holds all available tools and dispatches calls.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Return tool definitions for the API request body.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    /// Look up a tool by name (for parallel execution).
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// Look up and execute a tool by name.
    pub async fn execute(&self, name: &str, input: serde_json::Value) -> Result<String> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {name}"))?;
        tool.execute(input).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct MockTool;

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            "mock"
        }
        fn description(&self) -> &str {
            "A mock tool for testing"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({ "type": "object", "properties": {} })
        }
        async fn execute(&self, _input: serde_json::Value) -> Result<String> {
            Ok("mock output".to_string())
        }
    }

    #[test]
    fn test_register_and_get() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool));
        assert!(registry.get("mock").is_some());
        assert_eq!(registry.get("mock").unwrap().name(), "mock");
    }

    #[test]
    fn test_get_nonexistent() {
        let registry = ToolRegistry::new();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_definitions() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool));
        let defs = registry.definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "mock");
        assert_eq!(defs[0].description, "A mock tool for testing");
    }
}
