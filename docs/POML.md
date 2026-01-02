# POML - Pedram's Orchestration Markup Language

POML is a markdown-based dialect for defining task orchestration workflows in Petit. It provides a human-readable alternative to YAML that leverages familiar markdown syntax.

## Philosophy

- **Readable**: Looks like documentation, acts like configuration
- **Natural**: Uses markdown's natural hierarchy (headings, lists, code blocks)
- **Self-documenting**: Comments are just regular markdown text
- **Expressive**: Metadata through attributes, dependencies through context

## Syntax Overview

### Job Definition

The document title (H1) defines the job name. Job metadata is specified in a YAML-style frontmatter block.

```markdown
---
id: my_job
schedule: "0 0 * * *"
max_concurrency: 1
enabled: true
---

# My Job Name

This is the job description. It can span multiple paragraphs and include
any markdown formatting you like.
```

### Configuration

Job-level configuration is defined in a "Configuration" or "Config" section using a code block:

```markdown
## Configuration

```json
{
  "batch_size": 1000,
  "output_dir": "/tmp/data"
}
```
```

Or using a list format:

```markdown
## Configuration

- batch_size: 1000
- output_dir: "/tmp/data"
- retry_count: 3
```

### Tasks

Tasks are defined as H2 or H3 headings. The heading text is the task name/ID.

```markdown
## Extract Data

Task description goes here.

- **type**: command
- **command**: `./extract.sh`
- **args**: `["--output", "/tmp/data"]`
- **environment**:
  - STAGE: extract
  - DEBUG: true
```

### Dependencies

Dependencies are expressed using markdown links or explicit depends_on:

```markdown
## Transform Data

This task transforms the extracted data.

- **depends_on**: [Extract Data](#extract-data)
- **condition**: all_success
```

Or simply:

```markdown
## Transform Data

Depends on: [Extract Data](#extract-data)
```

### Complete Example

```markdown
---
id: data_pipeline
schedule: "0 0 2 * * *"  # 2 AM daily
---

# Data Pipeline

A comprehensive ETL pipeline for processing daily data.

## Configuration

- batch_size: 1000
- output_dir: /tmp/data

## Extract

Extracts data from the source database.

- **type**: command
- **command**: echo
- **args**: ["Extracting data..."]
- **environment**:
  - STAGE: extract

## Transform

Transforms the extracted data.

- **type**: command
- **command**: echo
- **args**: ["Transforming data..."]
- **depends_on**: [Extract](#extract)
- **environment**:
  - STAGE: transform

## Load

Loads the transformed data to the destination.

- **type**: command
- **command**: echo
- **args**: ["Loading data..."]
- **depends_on**: [Transform](#transform)
- **condition**: all_success
```

## Syntax Rules

1. **Frontmatter**: YAML-style metadata between `---` markers at the document start
2. **Job Name**: The first H1 heading after frontmatter
3. **Configuration**: H2 section titled "Configuration" or "Config"
4. **Tasks**: H2 or H3 headings (excluding special sections)
5. **Task Metadata**: Unordered list with bold keys
6. **Dependencies**: Via `depends_on` field or inline links
7. **Comments**: Regular markdown text (not parsed as configuration)

## Advantages Over YAML

- **More readable**: Looks like documentation
- **Better for complex descriptions**: Full markdown support for task descriptions
- **IDE support**: Markdown editors provide better UX than YAML editors
- **Flexible formatting**: Multiple ways to express the same thing
- **Self-documenting**: The format encourages adding context and explanations
- **Version control friendly**: More semantic diffs

## Implementation

POML files use the `.poml` or `.md` extension. The parser converts POML to the internal Job representation, making it fully compatible with Petit's execution engine.
