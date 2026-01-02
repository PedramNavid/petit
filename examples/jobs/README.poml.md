# POML Examples

This directory contains example job definitions in POML (Pedram's Orchestration Markup Language) format.

## What is POML?

POML is a markdown-based format for defining task orchestration workflows. It provides a more human-readable and expressive alternative to YAML.

## Files

- `hello_world.poml` - A simple single-task job
- `data_pipeline.poml` - A complex ETL pipeline with dependencies and configuration

## Usage

To parse a POML file in your code:

```rust
use petit::{PomlParser, PomlDocument};

let content = std::fs::read_to_string("examples/jobs/hello_world.poml")?;
let doc = PomlParser::parse(&content)?;

println!("Job: {} ({})", doc.name, doc.metadata.id);
println!("Tasks: {}", doc.tasks.len());
```

To convert POML to YAML:

```rust
let yaml = PomlParser::to_yaml(&doc)?;
std::fs::write("output.yaml", yaml)?;
```

## Key Features

1. **Frontmatter** - YAML-style metadata at the top
2. **Markdown Headers** - H1 for job name, H2/H3 for tasks
3. **Lists** - Bullet lists for task properties
4. **Links** - Markdown links for task dependencies
5. **Code Blocks** - JSON configuration blocks
6. **Rich Text** - Full markdown support for descriptions

## Comparison

### YAML
```yaml
id: hello
name: Hello World
tasks:
  - id: greet
    type: command
    command: echo
    args: ["Hello!"]
```

### POML
```markdown
---
id: hello
---

# Hello World

## Greet

- **type**: command
- **command**: echo
- **args**: ["Hello!"]
```

POML is more verbose but significantly more readable and self-documenting.
