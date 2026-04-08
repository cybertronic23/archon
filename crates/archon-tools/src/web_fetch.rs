use anyhow::Result;
use archon_core::Tool;
use async_trait::async_trait;
use serde_json::json;

pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch and extract content from a web page. Converts HTML to readable markdown format. \
         Supports setting custom headers and timeout."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (must start with http:// or https://)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30)"
                },
                "user_agent": {
                    "type": "string",
                    "description": "Custom User-Agent header (default: Archon/1.0)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let url = input["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: url"))?;

        // Validate URL
        if !url.starts_with("http://") && !url.starts_with("https://") {
            anyhow::bail!("URL must start with http:// or https://");
        }

        let timeout_secs = input["timeout"].as_u64().unwrap_or(30);
        let user_agent = input["user_agent"]
            .as_str()
            .unwrap_or("Archon/1.0 (WebFetch Tool)");

        // Create HTTP client with timeout
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .user_agent(user_agent)
            .build()?;

        // Fetch the URL
        let response = client.get(url).send().await?;
        let status = response.status();

        if !status.is_success() {
            anyhow::bail!("HTTP error {}: {}", status.as_u16(), status.canonical_reason().unwrap_or("Unknown"));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/html");

        let body = response.text().await?;

        // Convert HTML to markdown if content-type is HTML
        if content_type.contains("text/html") {
            let markdown = html_to_markdown(&body, url)?;
            Ok(format!(
                "# Fetched: {}\n\n**Status:** {}\n**Content-Type:** {}\n\n---\n\n{}",
                url, status, content_type, markdown
            ))
        } else {
            // Return text content as-is (truncated if too long)
            let truncated = if body.len() > 100000 {
                format!("{}\n\n...[content truncated, total length: {} bytes]", &body[..100000], body.len())
            } else {
                body
            };

            Ok(format!(
                "# Fetched: {}\n\n**Status:** {}\n**Content-Type:** {}\n\n---\n\n```\n{}\n```",
                url, status, content_type, truncated
            ))
        }
    }
}

/// Simple HTML to Markdown conversion
fn html_to_markdown(html: &str, base_url: &str) -> Result<String> {
    // Use a simple approach: extract text and structure
    // For a production system, you might want to use a crate like `html2md` or `readable`

    let mut output = String::new();
    let mut in_script = false;
    let mut in_style = false;
    let mut in_title = false;

    // Very simple HTML tag stripper that preserves some structure
    let lines: Vec<&str> = html.split('<').collect();

    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            continue; // Skip content before first tag
        }

        if let Some(end_pos) = line.find('>') {
            let tag_content = &line[..end_pos];
            let text = &line[end_pos + 1..];

            let tag_lower = tag_content.to_lowercase();

            // Handle closing tags
            if tag_lower.starts_with("/script") {
                in_script = false;
                continue;
            }
            if tag_lower.starts_with("/style") {
                in_style = false;
                continue;
            }
            if tag_lower.starts_with("/title") {
                in_title = false;
                continue;
            }

            // Handle opening tags
            if tag_lower.starts_with("script") {
                in_script = true;
                continue;
            }
            if tag_lower.starts_with("style") {
                in_style = true;
                continue;
            }
            if tag_lower.starts_with("title") {
                in_title = true;
                if !text.is_empty() {
                    output.push_str("# ");
                    output.push_str(&html_unescape(text));
                    output.push('\n');
                }
                continue;
            }

            // Skip content inside script/style
            if in_script || in_style {
                continue;
            }

            // Handle heading tags
            if tag_lower.starts_with("h1") {
                if !text.is_empty() {
                    output.push_str("# ");
                    output.push_str(&html_unescape(text));
                    output.push('\n');
                }
            } else if tag_lower.starts_with("h2") {
                if !text.is_empty() {
                    output.push_str("## ");
                    output.push_str(&html_unescape(text));
                    output.push('\n');
                }
            } else if tag_lower.starts_with("h3") {
                if !text.is_empty() {
                    output.push_str("### ");
                    output.push_str(&html_unescape(text));
                    output.push('\n');
                }
            } else if tag_lower.starts_with("p") || tag_lower.starts_with("div") || tag_lower.starts_with("br") {
                if !text.is_empty() {
                    output.push_str(&html_unescape(text));
                    output.push('\n');
                }
            } else if tag_lower.starts_with("li") {
                if !text.is_empty() {
                    output.push_str("- ");
                    output.push_str(&html_unescape(text));
                    output.push('\n');
                }
            } else if tag_lower.starts_with("a ") || tag_lower == "a" {
                // Extract href for links
                let href = if let Some(start) = tag_content.find("href=") {
                    let rest = &tag_content[start + 5..];
                    let delim = rest.chars().next().unwrap_or('"');
                    let end = rest[1..].find(delim).unwrap_or(rest.len() - 1);
                    &rest[1..=end]
                } else {
                    ""
                };

                if !text.is_empty() {
                    output.push('[');
                    output.push_str(&html_unescape(text));
                    output.push_str("](");
                    // Resolve relative URLs
                    if href.starts_with("http://") || href.starts_with("https://") {
                        output.push_str(href);
                    } else if !href.starts_with("#") && !href.is_empty() {
                        // Relative URL - resolve against base
                        if base_url.ends_with('/') {
                            output.push_str(base_url);
                            output.push_str(href.trim_start_matches('/'));
                        } else {
                            output.push_str(base_url);
                            output.push('/');
                            output.push_str(href.trim_start_matches('/'));
                        }
                    }
                    output.push_str(") ");
                }
            } else {
                // For other tags, just add the text
                if !text.is_empty() {
                    output.push_str(&html_unescape(text));
                }
            }
        }
    }

    // Clean up output
    let cleaned = output
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(cleaned)
}

/// Unescape common HTML entities
fn html_unescape(text: &str) -> String {
    let mut result = text.to_string();

    // Common entities
    let entities = [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", " "),
        ("&copy;", "©"),
        ("&reg;", "®"),
        ("&trade;", "™"),
    ];

    for (entity, replacement) in &entities {
        result = result.replace(entity, replacement);
    }

    // Handle numeric entities like &#123;
    let mut final_result = String::new();
    let mut chars = result.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '&' && chars.peek() == Some(&'#') {
            chars.next(); // consume '#'
            let mut num_str = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    num_str.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            if chars.peek() == Some(&';') {
                chars.next(); // consume ';'
            }
            if let Ok(num) = num_str.parse::<u32>() {
                if let Some(ch) = char::from_u32(num) {
                    final_result.push(ch);
                    continue;
                }
            }
            // If parsing failed, push the original text
            final_result.push('&');
            final_result.push('#');
            final_result.push_str(&num_str);
            final_result.push(';');
        } else {
            final_result.push(c);
        }
    }

    final_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::Tool;
    use serde_json::json;

    #[tokio::test]
    async fn test_web_fetch_basic() {
        let tool = WebFetchTool;
        // Test with a well-known site that returns HTML
        let result = tool.execute(json!({
            "url": "https://httpbin.org/html"
        })).await;

        assert!(result.is_ok(), "Fetch should succeed: {:?}", result.err());
        let content = result.unwrap();
        assert!(content.contains("httpbin.org") || content.contains("Herman Melville"),
                "Response should contain expected content: {}", content);
    }

    #[tokio::test]
    async fn test_web_fetch_with_timeout() {
        let tool = WebFetchTool;
        let result = tool.execute(json!({
            "url": "https://httpbin.org/delay/10",
            "timeout": 1
        })).await;

        assert!(result.is_err(), "Should timeout");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("timeout") || err.contains("Timed out"),
                "Error should mention timeout: {}", err);
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_url() {
        let tool = WebFetchTool;
        let result = tool.execute(json!({
            "url": "not-a-valid-url"
        })).await;

        assert!(result.is_err(), "Should fail for invalid URL");
    }

    #[tokio::test]
    async fn test_web_fetch_non_html() {
        let tool = WebFetchTool;
        let result = tool.execute(json!({
            "url": "https://httpbin.org/json"
        })).await;

        assert!(result.is_ok(), "Fetch should succeed: {:?}", result.err());
        let content = result.unwrap();
        assert!(content.contains("{") || content.contains("slideshow"),
                "Response should contain JSON content: {}", content);
    }

    // Tests for html_unescape
    #[test]
    fn test_html_unescape_basic() {
        assert_eq!(html_unescape("&lt;div&gt;"), "<div>");
        assert_eq!(html_unescape("&amp;test"), "&test");
        assert_eq!(html_unescape("&quot;hello&quot;"), "\"hello\"");
    }

    #[test]
    fn test_html_unescape_numeric() {
        assert_eq!(html_unescape("&#60;test&#62;"), "<test>");
        assert_eq!(html_unescape("&#39;quote&#39;"), "'quote'");
    }

    #[test]
    fn test_html_unescape_no_entities() {
        assert_eq!(html_unescape("plain text"), "plain text");
        assert_eq!(html_unescape(""), "");
    }
}
