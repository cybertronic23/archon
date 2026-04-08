use std::collections::HashSet;
use std::sync::Mutex;

use archon_core::permission::{PermissionHandler, PermissionRequest, PermissionVerdict};

/// Interactive permission handler that prompts the user via stdin.
pub struct InteractivePermissionHandler {
    always_allow: Mutex<HashSet<String>>,
}

impl InteractivePermissionHandler {
    pub fn new() -> Self {
        Self {
            always_allow: Mutex::new(HashSet::new()),
        }
    }
}

#[async_trait::async_trait]
impl PermissionHandler for InteractivePermissionHandler {
    async fn check(&self, request: &PermissionRequest<'_>) -> PermissionVerdict {
        // Check always-allow set
        {
            let set = self.always_allow.lock().unwrap();
            if set.contains(request.tool_name) {
                return PermissionVerdict::Allow;
            }
        }

        let tool_name = request.tool_name.to_string();
        let risk = request.risk_level;
        let input_summary = {
            let pretty = serde_json::to_string_pretty(request.input)
                .unwrap_or_else(|_| request.input.to_string());
            if pretty.len() > 500 {
                format!("{}...", &pretty[..500])
            } else {
                pretty
            }
        };

        // Use spawn_blocking to read stdin without blocking the async runtime
        let answer = tokio::task::spawn_blocking(move || {
            use std::io::{self, BufRead, Write};

            eprintln!("\n[Permission required] tool={tool_name} risk={risk:?}");
            eprintln!("{input_summary}");
            eprint!("Allow? [y]es / [n]o / [a]lways: ");
            io::stderr().flush().ok();

            let mut line = String::new();
            io::stdin().lock().read_line(&mut line).ok();
            line.trim().to_lowercase()
        })
        .await
        .unwrap_or_default();

        match answer.as_str() {
            "y" | "yes" => PermissionVerdict::Allow,
            "a" | "always" => {
                let mut set = self.always_allow.lock().unwrap();
                set.insert(request.tool_name.to_string());
                PermissionVerdict::Allow
            }
            _ => PermissionVerdict::Deny,
        }
    }
}
