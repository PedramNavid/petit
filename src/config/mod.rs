//! Configuration loading and parsing.
//!
//! This module provides YAML-based configuration for jobs and global settings.
//!
//! # Security Considerations
//!
//! ## Secrets Management
//!
//! **Never hardcode secrets in YAML files.** Configuration files are typically stored in version
//! control and should not contain sensitive information like API keys, passwords, or tokens.
//!
//! ### Best Practices for Secrets
//!
//! 1. **Use environment variable references**: Reference environment variables that will be
//!    resolved at runtime:
//!
//!    ```yaml
//!    tasks:
//!      - id: api_call
//!        type: command
//!        command: curl
//!        args: ["-H", "Authorization: Bearer $API_TOKEN", "https://api.example.com"]
//!        environment:
//!          API_TOKEN: ${API_TOKEN}  # Resolved from system environment at runtime
//!    ```
//!
//! 2. **Keep secrets out of version control**: Add sensitive files to `.gitignore` and use
//!    separate configuration management for secrets (e.g., HashiCorp Vault, AWS Secrets Manager,
//!    or environment-specific `.env` files that are not committed).
//!
//! 3. **Use restricted file permissions**: Ensure configuration files containing environment
//!    variable mappings have appropriate permissions (e.g., `chmod 600`).
//!
//! 4. **Audit and rotate secrets regularly**: Implement a process for regularly reviewing and
//!    rotating credentials referenced in configurations.
//!
//! ## Command Safety
//!
//! The config system allows execution of arbitrary commands, which poses security risks if
//! not handled properly.
//!
//! ### Command Injection Prevention
//!
//! **Never interpolate untrusted input into command strings.** Always use the structured `args`
//! array for parameters instead of string concatenation:
//!
//! ```yaml
//! # UNSAFE: Vulnerable to command injection if $USER_INPUT comes from untrusted source
//! tasks:
//!   - id: bad_example
//!     type: command
//!     command: sh
//!     args: ["-c", "echo $USER_INPUT"]  # If USER_INPUT is from user, this is dangerous
//!
//! # SAFER: Use parameterized execution
//! tasks:
//!   - id: good_example
//!     type: command
//!     command: echo
//!     args: ["${USER_INPUT}"]  # Command arguments are passed separately, not interpreted by shell
//! ```
//!
//! ### Additional Command Safety Practices
//!
//! 1. **Principle of least privilege**: Run tasks with minimal required permissions. Consider
//!    using dedicated service accounts with restricted access.
//!
//! 2. **Validate input sources**: If configuration values come from external sources (e.g.,
//!    API responses, user input), validate and sanitize them before use.
//!
//! 3. **Use absolute paths**: Specify full paths to executables to prevent PATH manipulation
//!    attacks:
//!
//!    ```yaml
//!    tasks:
//!      - id: secure_command
//!        type: command
//!        command: /usr/bin/python3  # Absolute path prevents PATH-based attacks
//!        args: ["script.py"]
//!    ```
//!
//! 4. **Limit working directories**: Be explicit about `working_dir` and avoid using untrusted
//!    paths that could lead to directory traversal issues.
//!
//! 5. **Review command permissions**: Regularly audit what commands are configured and ensure
//!    they follow organizational security policies.
//!
//! ## Environment Variable Security
//!
//! Environment variables can expose sensitive information if not handled carefully:
//!
//! 1. **Avoid logging environment variables**: Task execution logs may capture environment
//!    variables. Be cautious about what gets logged.
//!
//! 2. **Scope variables appropriately**: Use task-level environment variables instead of
//!    global ones when possible to limit exposure.
//!
//! 3. **Document variable requirements**: Clearly document which environment variables are
//!    required and their expected format to prevent misconfigurations.
//!
//! ## Configuration File Security
//!
//! 1. **Validate configuration sources**: Only load configuration files from trusted locations
//!    with proper access controls.
//!
//! 2. **Use file integrity monitoring**: In production environments, consider monitoring
//!    configuration files for unauthorized changes.
//!
//! 3. **Separate concerns**: Keep job definitions separate from credentials and infrastructure
//!    configuration.

mod builder;
mod yaml;

pub use builder::{JobConfigBuilder, load_jobs_from_directory};
pub use yaml::{
    ConfigError, GlobalConfig, JobConfig, RetryConfig, ScheduleConfig, TaskConfig, YamlLoader,
};
