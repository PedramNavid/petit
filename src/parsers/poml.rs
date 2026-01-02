//! POML (Pedram's Orchestration Markup Language) parser.
//!
//! POML is a markdown-based format for defining task orchestration workflows.
//! It provides a human-readable alternative to YAML that leverages markdown syntax.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur when parsing POML.
#[derive(Debug, Error)]
pub enum PomlError {
    /// Failed to parse frontmatter.
    #[error("invalid frontmatter: {0}")]
    InvalidFrontmatter(String),

    /// Missing required field.
    #[error("missing required field: {0}")]
    MissingField(String),

    /// Invalid task definition.
    #[error("invalid task definition: {0}")]
    InvalidTask(String),

    /// Invalid configuration.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// General parsing error.
    #[error("parse error: {0}")]
    ParseError(String),
}

/// Represents a parsed POML document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PomlDocument {
    /// Job metadata from frontmatter.
    pub metadata: PomlMetadata,
    /// Job name from H1 heading.
    pub name: String,
    /// Job description (text between H1 and first task).
    pub description: String,
    /// Job-level configuration.
    pub config: HashMap<String, Value>,
    /// List of tasks.
    pub tasks: Vec<PomlTask>,
}

/// Job metadata from frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PomlMetadata {
    /// Job ID.
    pub id: String,
    /// Optional schedule (cron expression).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    /// Optional maximum concurrency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<usize>,
    /// Whether the job is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Optional job dependencies.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

/// Represents a task in POML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PomlTask {
    /// Task ID (derived from heading).
    pub id: String,
    /// Task display name.
    pub name: String,
    /// Task description.
    pub description: String,
    /// Task type (e.g., "command").
    #[serde(rename = "type")]
    pub task_type: String,
    /// Command to execute (for command tasks).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Command arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables.
    #[serde(default)]
    pub environment: HashMap<String, String>,
    /// Task dependencies (other task IDs).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Dependency condition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Additional task configuration.
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

/// POML parser.
pub struct PomlParser;

impl PomlParser {
    /// Parse a POML document from a string.
    pub fn parse(content: &str) -> Result<PomlDocument, PomlError> {
        let (frontmatter, body) = Self::extract_frontmatter(content)?;
        let metadata = Self::parse_frontmatter(&frontmatter)?;

        let lines: Vec<&str> = body.lines().collect();
        let (name, description_start) = Self::extract_job_name(&lines)?;
        let (description, config, tasks) = Self::parse_body(&lines[description_start..])?;

        Ok(PomlDocument {
            metadata,
            name,
            description,
            config,
            tasks,
        })
    }

    /// Extract frontmatter from the document.
    fn extract_frontmatter(content: &str) -> Result<(String, String), PomlError> {
        let content = content.trim_start();

        if !content.starts_with("---") {
            return Err(PomlError::MissingField("frontmatter".to_string()));
        }

        let rest = &content[3..];
        if let Some(end_pos) = rest.find("\n---") {
            let frontmatter = rest[..end_pos].trim().to_string();
            let body = rest[end_pos + 4..].trim().to_string();
            Ok((frontmatter, body))
        } else {
            Err(PomlError::InvalidFrontmatter(
                "missing closing ---".to_string(),
            ))
        }
    }

    /// Parse YAML frontmatter into metadata.
    fn parse_frontmatter(yaml: &str) -> Result<PomlMetadata, PomlError> {
        serde_yaml::from_str(yaml)
            .map_err(|e| PomlError::InvalidFrontmatter(e.to_string()))
    }

    /// Extract the job name from the first H1 heading.
    fn extract_job_name(lines: &[&str]) -> Result<(String, usize), PomlError> {
        for (i, line) in lines.iter().enumerate() {
            if line.starts_with("# ") {
                let name = line[2..].trim().to_string();
                return Ok((name, i + 1));
            }
        }
        Err(PomlError::MissingField("job name (H1 heading)".to_string()))
    }

    /// Parse the document body (description, config, tasks).
    fn parse_body(
        lines: &[&str],
    ) -> Result<(String, HashMap<String, Value>, Vec<PomlTask>), PomlError> {
        let mut description = String::new();
        let mut config = HashMap::new();
        let mut tasks = Vec::new();
        let mut i = 0;

        // Parse description until we hit a heading
        while i < lines.len() {
            let line = lines[i].trim();
            if line.starts_with("##") || line.starts_with("###") {
                break;
            }
            if !description.is_empty() {
                description.push('\n');
            }
            description.push_str(line);
            i += 1;
        }
        description = description.trim().to_string();

        // Parse sections (config and tasks)
        while i < lines.len() {
            let line = lines[i].trim();

            if line.starts_with("## ") || line.starts_with("### ") {
                let heading = if line.starts_with("## ") {
                    line[3..].trim()
                } else {
                    line[4..].trim()
                };

                // Check if this is a config section
                if heading.eq_ignore_ascii_case("configuration")
                    || heading.eq_ignore_ascii_case("config")
                {
                    let (parsed_config, next_i) = Self::parse_config_section(lines, i + 1)?;
                    config = parsed_config;
                    i = next_i;
                } else {
                    // Parse as task
                    let (task, next_i) = Self::parse_task_section(lines, i)?;
                    tasks.push(task);
                    i = next_i;
                }
            } else {
                i += 1;
            }
        }

        Ok((description, config, tasks))
    }

    /// Parse a configuration section.
    fn parse_config_section(
        lines: &[&str],
        start: usize,
    ) -> Result<(HashMap<String, Value>, usize), PomlError> {
        let mut config = HashMap::new();
        let mut i = start;
        let mut in_code_block = false;
        let mut code_block = String::new();

        while i < lines.len() {
            let line = lines[i].trim();

            // Stop at next heading
            if (line.starts_with("##") || line.starts_with("###")) && !in_code_block {
                break;
            }

            // Handle code blocks
            if line.starts_with("```") {
                if in_code_block {
                    // End of code block - parse as JSON
                    if !code_block.trim().is_empty() {
                        let json_config: HashMap<String, Value> =
                            serde_json::from_str(&code_block)
                                .map_err(|e| PomlError::InvalidConfig(e.to_string()))?;
                        config.extend(json_config);
                    }
                    code_block.clear();
                    in_code_block = false;
                } else {
                    in_code_block = true;
                }
            } else if in_code_block {
                code_block.push_str(line);
                code_block.push('\n');
            } else if line.starts_with("- ") {
                // Parse list-style config
                let item = line[2..].trim();
                if let Some(colon_pos) = item.find(':') {
                    let key = item[..colon_pos].trim().to_string();
                    let value = item[colon_pos + 1..].trim();
                    config.insert(key, Self::parse_value(value));
                }
            }

            i += 1;
        }

        Ok((config, i))
    }

    /// Parse a task section.
    fn parse_task_section(
        lines: &[&str],
        start: usize,
    ) -> Result<(PomlTask, usize), PomlError> {
        let heading_line = lines[start].trim();
        let heading = if heading_line.starts_with("## ") {
            heading_line[3..].trim()
        } else {
            heading_line[4..].trim()
        };

        // Generate task ID from heading (lowercase, replace spaces with underscores)
        let id = heading
            .to_lowercase()
            .replace(' ', "_")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>();

        let mut description = String::new();
        let mut task_type = "command".to_string();
        let mut command = None;
        let mut args = Vec::new();
        let mut environment = HashMap::new();
        let mut depends_on = Vec::new();
        let mut condition = None;
        let mut extra = HashMap::new();

        let mut i = start + 1;

        while i < lines.len() {
            let line = lines[i].trim();

            // Stop at next heading
            if line.starts_with("##") || line.starts_with("###") {
                break;
            }

            // Parse metadata from list items
            if line.starts_with("- **") && line.contains("**:") {
                let rest = line[4..].trim();
                if let Some(colon_pos) = rest.find("**:") {
                    let key = rest[..colon_pos].trim().to_lowercase();
                    let value = rest[colon_pos + 3..].trim();

                    match key.as_str() {
                        "type" => task_type = Self::parse_string_value(value),
                        "command" => command = Some(Self::parse_string_value(value)),
                        "args" => args = Self::parse_array_value(value),
                        "depends_on" | "depends on" => {
                            depends_on = Self::parse_depends_on(value);
                        }
                        "condition" => condition = Some(Self::parse_string_value(value)),
                        "environment" => {
                            // Multi-line environment vars follow
                            let (env, next_i) = Self::parse_nested_list(lines, i + 1)?;
                            environment = env;
                            i = next_i - 1;
                        }
                        _ => {
                            extra.insert(key, Self::parse_value(value));
                        }
                    }
                }
            } else if line.starts_with("Depends on:") || line.starts_with("depends on:") {
                // Alternative dependency syntax
                let deps_str = line.split(':').nth(1).unwrap_or("").trim();
                depends_on = Self::parse_depends_on(deps_str);
            } else if !line.is_empty() && !line.starts_with('-') {
                // Add to description
                if !description.is_empty() {
                    description.push('\n');
                }
                description.push_str(line);
            }

            i += 1;
        }

        Ok((
            PomlTask {
                id: id.clone(),
                name: heading.to_string(),
                description: description.trim().to_string(),
                task_type,
                command,
                args,
                environment,
                depends_on,
                condition,
                extra,
            },
            i,
        ))
    }

    /// Parse a nested list (e.g., environment variables).
    fn parse_nested_list(
        lines: &[&str],
        start: usize,
    ) -> Result<(HashMap<String, String>, usize), PomlError> {
        let mut map = HashMap::new();
        let mut i = start;

        while i < lines.len() {
            let line = lines[i].trim();

            // Stop at non-indented content or next heading
            if !line.starts_with("  - ") && !line.is_empty() {
                break;
            }

            if line.starts_with("  - ") {
                let item = line[4..].trim();
                if let Some(colon_pos) = item.find(':') {
                    let key = item[..colon_pos].trim().to_string();
                    let value = item[colon_pos + 1..].trim().to_string();
                    map.insert(key, value);
                }
            }

            i += 1;
        }

        Ok((map, i))
    }

    /// Parse a value from string.
    fn parse_value(s: &str) -> Value {
        let s = s.trim();

        // Try as number
        if let Ok(n) = s.parse::<i64>() {
            return Value::Number(n.into());
        }
        if let Ok(f) = s.parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(f) {
                return Value::Number(n);
            }
        }

        // Try as boolean
        if s.eq_ignore_ascii_case("true") {
            return Value::Bool(true);
        }
        if s.eq_ignore_ascii_case("false") {
            return Value::Bool(false);
        }

        // Try as JSON
        if s.starts_with('{') || s.starts_with('[') {
            if let Ok(v) = serde_json::from_str(s) {
                return v;
            }
        }

        // Default to string, removing quotes if present
        let s = s.trim_matches('"').trim_matches('\'');
        Value::String(s.to_string())
    }

    /// Parse a string value, removing quotes and backticks.
    fn parse_string_value(s: &str) -> String {
        s.trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim_matches('`')
            .to_string()
    }

    /// Parse an array value from string.
    fn parse_array_value(s: &str) -> Vec<String> {
        if let Ok(Value::Array(arr)) = serde_json::from_str(s) {
            arr.into_iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        } else {
            vec![s.to_string()]
        }
    }

    /// Parse dependencies from markdown links or plain text.
    fn parse_depends_on(s: &str) -> Vec<String> {
        let mut deps = Vec::new();

        // Extract from markdown links: [Task Name](#task-id)
        let mut remaining = s;
        while let Some(start) = remaining.find('[') {
            if let Some(end) = remaining[start..].find(']') {
                let link_text = &remaining[start + 1..start + end];

                // Check for anchor link
                if let Some(anchor_start) = remaining[start + end..].find("(#") {
                    if let Some(anchor_end) = remaining[start + end + anchor_start..].find(')') {
                        let anchor = &remaining
                            [start + end + anchor_start + 2..start + end + anchor_start + anchor_end];
                        deps.push(anchor.to_string());
                    }
                } else {
                    // Use link text as task name, convert to ID
                    let task_id = link_text
                        .to_lowercase()
                        .replace(' ', "_")
                        .chars()
                        .filter(|c| c.is_alphanumeric() || *c == '_')
                        .collect::<String>();
                    deps.push(task_id);
                }

                remaining = &remaining[start + end + 1..];
            } else {
                break;
            }
        }

        // If no links found, try array syntax
        if deps.is_empty() {
            if let Ok(Value::Array(arr)) = serde_json::from_str(s) {
                deps = arr
                    .into_iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
            }
        }

        deps
    }

    /// Convert a POML document to YAML format.
    pub fn to_yaml(doc: &PomlDocument) -> Result<String, PomlError> {
        #[derive(Serialize)]
        struct YamlJob {
            id: String,
            name: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            schedule: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            max_concurrency: Option<usize>,
            enabled: bool,
            #[serde(skip_serializing_if = "HashMap::is_empty")]
            config: HashMap<String, Value>,
            tasks: Vec<YamlTask>,
        }

        #[derive(Serialize)]
        struct YamlTask {
            id: String,
            #[serde(rename = "type")]
            task_type: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            command: Option<String>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            args: Vec<String>,
            #[serde(skip_serializing_if = "HashMap::is_empty")]
            environment: HashMap<String, String>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            depends_on: Vec<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            condition: Option<String>,
        }

        let yaml_job = YamlJob {
            id: doc.metadata.id.clone(),
            name: doc.name.clone(),
            schedule: doc.metadata.schedule.clone(),
            max_concurrency: doc.metadata.max_concurrency,
            enabled: doc.metadata.enabled,
            config: doc.config.clone(),
            tasks: doc
                .tasks
                .iter()
                .map(|t| YamlTask {
                    id: t.id.clone(),
                    task_type: t.task_type.clone(),
                    command: t.command.clone(),
                    args: t.args.clone(),
                    environment: t.environment.clone(),
                    depends_on: t.depends_on.clone(),
                    condition: t.condition.clone(),
                })
                .collect(),
        };

        serde_yaml::to_string(&yaml_job).map_err(|e| PomlError::ParseError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_poml() {
        let poml = r#"---
id: test_job
schedule: "0 * * * *"
---

# Test Job

A simple test job.

## Say Hello

- **type**: command
- **command**: echo
- **args**: ["Hello, World!"]
"#;

        let doc = PomlParser::parse(poml).unwrap();
        assert_eq!(doc.metadata.id, "test_job");
        assert_eq!(doc.name, "Test Job");
        assert_eq!(doc.tasks.len(), 1);
        assert_eq!(doc.tasks[0].id, "say_hello");
        assert_eq!(doc.tasks[0].command, Some("echo".to_string()));
    }

    #[test]
    fn test_parse_with_dependencies() {
        let poml = r#"---
id: pipeline
---

# Pipeline

## Extract

- **type**: command
- **command**: echo
- **args**: ["extracting"]

## Transform

- **type**: command
- **command**: echo
- **args**: ["transforming"]
- **depends_on**: [Extract](#extract)
"#;

        let doc = PomlParser::parse(poml).unwrap();
        assert_eq!(doc.tasks.len(), 2);
        assert_eq!(doc.tasks[1].depends_on, vec!["extract"]);
    }

    #[test]
    fn test_parse_with_config() {
        let poml = r#"---
id: test
---

# Test

## Configuration

- batch_size: 1000
- output_dir: /tmp/data

## Task

- **type**: command
- **command**: echo
"#;

        let doc = PomlParser::parse(poml).unwrap();
        assert_eq!(doc.config.get("batch_size").unwrap(), &Value::Number(1000.into()));
    }

    #[test]
    fn test_to_yaml() {
        let poml = r#"---
id: test
---

# Test Job

## Task One

- **type**: command
- **command**: echo
- **args**: ["hello"]
"#;

        let doc = PomlParser::parse(poml).unwrap();
        let yaml = PomlParser::to_yaml(&doc).unwrap();
        assert!(yaml.contains("id: test"));
        assert!(yaml.contains("name: Test Job"));
    }
}
