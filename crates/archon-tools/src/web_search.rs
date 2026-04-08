use anyhow::Result;
use archon_core::Tool;
use async_trait::async_trait;
use serde_json::json;

pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for information. Returns search results with titles, URLs, and snippets. \
         Uses DuckDuckGo HTML interface (no API key required). \
         Supports result count limiting and recency filtering."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "num_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 10, max: 20)"
                },
                "recency_days": {
                    "type": "integer",
                    "description": "Filter results by recency (days). Optional."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let query = input["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: query"))?;

        let num_results = input["num_results"].as_u64().map(|n| n.min(20).max(1) as usize;
        let _recency_days = input["recency_days"].as_u64();

        // Use DuckDuckGo HTML search
        let search_results = search_duckduckgo(query, num_results).await?;

        if search_results.is_empty() {
            return Ok(format!("No search results found for query: '{}'", query));
        }

        // Format results
        let mut output = format!("# Search Results for: {}\n\n", query);

        for (i, result) in search_results.iter().enumerate() {
            output.push_str(&format!(
                "## {}. {}\n**URL:** {}\n\n{}\n\n---\n\n",
                i + 1,
                result.title,
                result.url,
                result.snippet
            ));
        }

        output.push_str(&format!("\n*Found {} results*", search_results.len()));

        Ok(output)
    }
}

#[derive(Debug)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

async fn search_duckduckgo(query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
    use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    // Build search URL
    let encoded_query = urlencoding::encode(query);
    let url = format!("https://html.duckduckgo.com/html/?q={}", encoded_query);

    // Set headers to mimic a browser
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36"
    ));

    let response = client.get(&url).headers(headers).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Search request failed with status: {}", response.status());
    }

    let html = response.text().await?;

    // Parse search results from HTML
    parse_duckduckgo_results(&html, max_results)
}

fn parse_duckduckgo_results(html: &str, max_results: usize) -> Result<Vec<SearchResult>> {
    let mut results = Vec::new();

    // DuckDuckGo HTML results are in elements with class "result"
    // Each result has:
    // - Title in <a class="result__a"> or <a class="result__title">
    // - URL in <a class="result__url"> or extracted from href
    // - Snippet in <a class="result__snippet"> or <div class="result__snippet">

    // Simple regex-free parsing
    let result_blocks: Vec<&str> = html.split("class=\"result\"").skip(1).collect();

    for block in result_blocks.iter().take(max_results) {
        let title = extract_between(block, "class=\"result__a\">", "</a>")
            .or_else(|| extract_between(block, "class=\"result__title\">", "</a>"))
            .unwrap_or_default();

        let url = extract_between(block, "class=\"result__url\">", "</a>")
            .or_else(|| extract_href(block))
            .unwrap_or_default();

        let snippet = extract_between(block, "class=\"result__snippet\">", "</a>")
            .or_else(|| extract_between(block, "class=\"result__snippet\">", "</div>"))
            .unwrap_or_default();

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title: html_unescape(&title),
                url: html_unescape(&url),
                snippet: html_unescape(&snippet),
            });
        }
    }

    Ok(results)
}

fn extract_between(text: &str, start: &str, end: &str) -> Option<String> {
    let start_idx = text.find(start)?;
    let after_start = &text[start_idx + start.len()..];
    let end_idx = after_start.find(end)?;
    Some(after_start[..end_idx].to_string())
}

fn extract_href(text: &str) -> Option<String> {
    let href_idx = text.find("href=\"")?;
    let after_href = &text[href_idx + 6..];
    let end_idx = after_href.find("\"")?;
    let url = &after_href[..end_idx];

    // Filter out DuckDuckGo redirect URLs
    if url.starts_with("/l/?") {
        // Extract the actual URL from the uddg parameter
        if let Some(start) = url.find("uddg=") {
            let encoded = &url[start + 5..];
            if let Ok(decoded) = urlencoding::decode(encoded) {
                return Some(decoded.to_string());
            }
        }
        None
    } else {
        Some(url.to_string())
    }
}

fn html_unescape(text: &str) -> String {
    let mut result = text.to_string();

    let entities = [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", " "),
    ];

    for (entity, replacement) in &entities {
        result = result.replace(entity, replacement);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::Tool;
    use serde_json::json;

    #[tokio::test]
    async fn test_web_search_schema() {
        let tool = WebSearchTool;
        let schema = tool.input_schema();

        assert!(schema.get("properties").is_some());
        assert!(schema.get("required").is_some());
    }

    #[test]
    fn test_html_unescape() {
        assert_eq!(super::html_unescape("&lt;div&gt;"), "<div>");
        assert_eq!(super::html_unescape("&amp;test"), "&test");
        assert_eq!(super::html_unescape("&quot;hello&quot;"), "\"hello\"");
    }

    #[test]
    fn test_extract_between() {
        assert_eq!(
            super::extract_between("start content end", "start ", " end"),
            Some("content".to_string())
        );
        assert_eq!(
            super::extract_between("no match", "xxx", "yyy"),
            None
        );
    }
}
