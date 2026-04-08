use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Safe,
    Moderate,
    Dangerous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionVerdict {
    Allow,
    Deny,
}

pub struct PermissionRequest<'a> {
    pub tool_name: &'a str,
    pub input: &'a Value,
    pub risk_level: RiskLevel,
}

/// Trait for permission gating before tool execution.
#[async_trait::async_trait]
pub trait PermissionHandler: Send + Sync {
    /// Classify risk level of a tool call. Default: read=Safe, edit=Moderate, bash=Dangerous.
    fn classify(&self, tool_name: &str, _input: &Value) -> RiskLevel {
        match tool_name {
            "read" | "glob" | "grep" => RiskLevel::Safe,
            "edit" => RiskLevel::Moderate,
            "bash" => RiskLevel::Dangerous,
            _ => RiskLevel::Moderate,
        }
    }

    /// Check permission for a non-Safe tool call.
    async fn check(&self, request: &PermissionRequest<'_>) -> PermissionVerdict;
}

/// Permits everything — backward-compatible default.
pub struct AllowAllPermissions;

#[async_trait::async_trait]
impl PermissionHandler for AllowAllPermissions {
    async fn check(&self, _: &PermissionRequest<'_>) -> PermissionVerdict {
        PermissionVerdict::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_classify_safe_tools() {
        let handler = AllowAllPermissions;
        let input = json!({});
        assert_eq!(handler.classify("read", &input), RiskLevel::Safe);
        assert_eq!(handler.classify("glob", &input), RiskLevel::Safe);
        assert_eq!(handler.classify("grep", &input), RiskLevel::Safe);
    }

    #[test]
    fn test_classify_moderate_tools() {
        let handler = AllowAllPermissions;
        let input = json!({});
        assert_eq!(handler.classify("edit", &input), RiskLevel::Moderate);
        assert_eq!(handler.classify("write", &input), RiskLevel::Moderate);
        assert_eq!(handler.classify("unknown_tool", &input), RiskLevel::Moderate);
    }

    #[test]
    fn test_classify_dangerous_tools() {
        let handler = AllowAllPermissions;
        let input = json!({});
        assert_eq!(handler.classify("bash", &input), RiskLevel::Dangerous);
    }

    #[tokio::test]
    async fn test_allow_all_permissions() {
        let handler = AllowAllPermissions;
        let input = json!({"command": "rm -rf /"});
        let request = PermissionRequest {
            tool_name: "bash",
            input: &input,
            risk_level: RiskLevel::Dangerous,
        };
        assert_eq!(handler.check(&request).await, PermissionVerdict::Allow);
    }
}
