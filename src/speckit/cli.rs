//! Spec-Kit CLI Executor
//!
//! Handles spawning and managing spec-kit CLI processes.

use anyhow::{Context, Result};
use async_process::{Command, Stdio};
use std::path::Path;
use std::time::Duration;
use tokio::time::timeout;

use super::errors::SpecKitError;

/// Result of executing a spec-kit command
#[derive(Debug, Clone)]
pub struct CommandResult {
    /// Standard output
    pub stdout: String,

    /// Standard error
    pub stderr: String,

    /// Exit code
    pub exit_code: i32,
}

impl CommandResult {
    /// Check if the command was successful
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }

    /// Get a summary of the result
    pub fn summary(&self) -> String {
        if self.is_success() {
            format!("Success\n{}", self.stdout.trim())
        } else {
            format!(
                "Failed (exit code {})\n{}",
                self.exit_code,
                self.stderr.trim()
            )
        }
    }
}

/// Spec-kit CLI integration
#[derive(Debug, Clone)]
pub struct SpecKitCli {
    /// Path to the specify command
    cli_path: String,

    /// Path to Python interpreter (reserved for future use)
    #[allow(dead_code)]
    python_path: String,

    /// Default timeout for commands (in seconds)
    timeout_seconds: u64,

    /// Test mode flag
    test_mode: bool,
}

impl SpecKitCli {
    /// Create a new spec-kit CLI interface
    pub fn new() -> Self {
        Self {
            cli_path: "uvx".to_string(),
            python_path: "python3".to_string(),
            timeout_seconds: 300, // 5 minutes
            test_mode: false,
        }
    }

    /// Create a test mode CLI (returns mock data)
    #[cfg(test)]
    pub fn new_test_mode() -> Self {
        Self {
            cli_path: "specify".to_string(),
            python_path: "python3".to_string(),
            timeout_seconds: 300,
            test_mode: true,
        }
    }

    /// Set the CLI path
    pub fn with_cli_path(mut self, path: impl Into<String>) -> Self {
        self.cli_path = path.into();
        self
    }

    /// Set the timeout
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = seconds;
        self
    }

    /// Check if spec-kit is installed (via uvx)
    pub async fn is_installed(&self) -> bool {
        if self.test_mode {
            return true;
        }

        // Check if uvx is available
        let uvx_available = Command::new(&self.cli_path)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if !uvx_available {
            tracing::warn!("uvx command not found - spec-kit requires uv/uvx");
            return false;
        }

        // Check if we can run spec-kit via uvx
        Command::new(&self.cli_path)
            .args([
                "--from",
                "git+https://github.com/github/spec-kit.git",
                "specify",
                "--help",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Execute a spec-kit command
    async fn execute_command(&self, args: &[&str]) -> Result<CommandResult> {
        if self.test_mode {
            return Ok(CommandResult {
                stdout: "Test mode: command executed successfully".to_string(),
                stderr: String::new(),
                exit_code: 0,
            });
        }

        // Build the full command with uvx + spec-kit repo + specify + args
        let mut full_args = vec![
            "--from",
            "git+https://github.com/github/spec-kit.git",
            "specify",
        ];
        full_args.extend_from_slice(args);

        tracing::debug!(
            command = %self.cli_path,
            args = ?full_args,
            "Executing spec-kit command via uvx"
        );

        let command_future = Command::new(&self.cli_path)
            .args(&full_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        let output = timeout(Duration::from_secs(self.timeout_seconds), command_future)
            .await
            .context("Command timeout")?
            .context("Failed to execute command")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        let result = CommandResult {
            stdout,
            stderr,
            exit_code,
        };

        if !result.is_success() {
            tracing::warn!(
                exit_code = result.exit_code,
                stderr = %result.stderr,
                "Spec-kit command failed"
            );
        }

        Ok(result)
    }

    /// Initialize a new spec-kit project
    pub async fn init(&self, project_name: &str, path: &Path) -> Result<CommandResult> {
        // specify >= v0.8.0 removed --path from `init`; use --here instead.
        // Create the target directory and run `init --here --force` inside it.
        tokio::fs::create_dir_all(path)
            .await
            .with_context(|| format!("Failed to create project directory: {}", path.display()))?;

        if self.test_mode {
            return Ok(CommandResult {
                stdout: format!(
                    "Test mode: initialized project '{}' at {}",
                    project_name,
                    path.display()
                ),
                stderr: String::new(),
                exit_code: 0,
            });
        }

        // Build the full command with uvx + spec-kit repo + specify init --here --force
        let mut full_args = vec![
            "--from",
            "git+https://github.com/github/spec-kit.git",
            "specify",
            "init",
            "--here",
            "--force",
        ];

        tracing::debug!(
            command = %self.cli_path,
            args = ?full_args,
            cwd = %path.display(),
            "Executing spec-kit init via uvx"
        );

        let command_future = Command::new(&self.cli_path)
            .args(&full_args)
            .current_dir(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        let output = timeout(Duration::from_secs(self.timeout_seconds), command_future)
            .await
            .context("Command timeout")?
            .context("Failed to execute command")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        if exit_code != 0 {
            return Err(SpecKitError::command_failed(
                format!("specify init {}", project_name),
                &stderr,
                exit_code,
            )
            .into());
        }

        Ok(CommandResult {
            stdout,
            stderr,
            exit_code,
        })
    }

    /// Create a constitution file
    pub async fn constitution(&self, content: &str, output_path: &Path) -> Result<CommandResult> {
        // For now, we'll write directly to the file since spec-kit
        // accepts input via stdin or prompts
        tokio::fs::write(output_path, content)
            .await
            .context("Failed to write constitution file")?;

        Ok(CommandResult {
            stdout: format!("Constitution written to {}", output_path.display()),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    /// Create a specification file
    pub async fn specify(
        &self,
        requirements: &str,
        output_path: &Path,
        _format: &str,
    ) -> Result<CommandResult> {
        // Write requirements to file
        // Spec-kit typically uses interactive prompts or file input
        tokio::fs::write(output_path, requirements)
            .await
            .context("Failed to write specification file")?;

        Ok(CommandResult {
            stdout: format!("Specification written to {}", output_path.display()),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    /// Create a technical plan
    pub async fn plan(&self, spec_file: &Path, output_path: &Path) -> Result<CommandResult> {
        // Read the spec file
        let spec_content = match tokio::fs::read_to_string(spec_file).await {
            Ok(content) => content,
            Err(e) => {
                return Err(SpecKitError::command_failed(
                    "specify plan",
                    &format!("Failed to read spec file: {}", e),
                    1,
                )
                .into());
            }
        };

        // Extract requirements section from spec
        let requirements = Self::extract_section(&spec_content, "Requirements");

        // Generate plan from template
        let plan = Self::generate_plan(&requirements, spec_file);

        // Write to output
        tokio::fs::write(output_path, &plan)
            .await
            .context("Failed to write plan file")?;

        tracing::info!(output = %output_path.display(), "Plan generated internally");

        Ok(CommandResult {
            stdout: format!("Technical plan created at {}", output_path.display()),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    /// Generate a task list
    pub async fn tasks(&self, plan_file: &Path, output_path: &Path) -> Result<CommandResult> {
        // Read the plan file
        let plan_content = match tokio::fs::read_to_string(plan_file).await {
            Ok(content) => content,
            Err(e) => {
                return Err(SpecKitError::command_failed(
                    "specify tasks",
                    &format!("Failed to read plan file: {}", e),
                    1,
                )
                .into());
            }
        };

        // Extract milestones from plan
        let milestones = Self::extract_milestones(&plan_content);

        // Generate tasks from template
        let tasks = Self::generate_tasks(&milestones, plan_file);

        // Write to output
        tokio::fs::write(output_path, &tasks)
            .await
            .context("Failed to write tasks file")?;

        tracing::info!(output = %output_path.display(), "Tasks generated internally");

        Ok(CommandResult {
            stdout: format!("Task list created at {}", output_path.display()),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    /// Extract a named section from markdown content
    fn extract_section(content: &str, section_name: &str) -> String {
        let mut in_section = false;
        let mut result = String::new();

        for line in content.lines() {
            if line.starts_with("## ") && line[3..].trim() == section_name {
                in_section = true;
                continue;
            }
            if in_section && line.starts_with("## ") {
                break;
            }
            if in_section && !line.trim().is_empty() {
                result.push_str(line.trim());
                result.push('\n');
            }
        }

        if result.is_empty() {
            // Fallback: include first non-empty lines after frontmatter
            for line in content.lines().skip(1) {
                if !line.trim().is_empty() && !line.starts_with('#') {
                    result.push_str(line.trim());
                    result.push('\n');
                    if result.len() > 500 {
                        break;
                    }
                }
            }
        }
        result
    }

    /// Extract milestones from a plan file
    fn extract_milestones(content: &str) -> Vec<(String, String)> {
        let mut milestones = Vec::new();
        let mut in_milestones = false;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("## Milestones") {
                in_milestones = true;
                continue;
            }
            if in_milestones && trimmed.starts_with("## ") {
                break;
            }
            if in_milestones && !trimmed.is_empty() {
                // Parse "1. **M1 Name**: description" pattern
                if let Some(rest) = trimmed.splitn(2, ". ").nth(1) {
                    let name = if let (Some(start), Some(end)) =
                        (rest.find("**"), rest.rfind("**"))
                    {
                        let inner = &rest[start + 2..end];
                        let after = &rest[end + 2..];
                        let after_trimmed = after.trim_start_matches(':').trim();
                        format!("{}: {}", inner, after_trimmed)
                    } else {
                        rest.to_string()
                    };
                    milestones.push((name, rest.to_string()));
                }
            }
        }

        // Also try "### Phase" headers
        if milestones.is_empty() {
            let mut current_phase = String::new();
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("### Phase") {
                    current_phase = trimmed.trim_start_matches("### ").to_string();
                } else if !current_phase.is_empty()
                    && trimmed.starts_with("- ")
                    && !trimmed.contains("This plan was")
                {
                    milestones.push((
                        current_phase.clone(),
                        trimmed.trim_start_matches("- ").to_string(),
                    ));
                }
            }
        }

        milestones
    }

    /// Generate a structured plan from spec requirements
    fn generate_plan(requirements: &str, spec_file: &Path) -> String {
        let spec_name = spec_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "spec".to_string());

        format!(
            r##"# Implementation Plan

**Spec**: {spec}
**Generated**: auto-generated by spec-kit-mcp

## Summary

{req_summary}

## Technical Context

- **Language/Version**: TBD — choose based on project requirements
- **Primary Dependencies**: TBD
- **Storage**: TBD
- **Testing**: TBD
- **Target Platform**: TBD

## Constitution Check

- ⬜ Verify alignment with project constitution

## Project Structure

```
src/
├── main/          # Application entry point
└── lib/           # Core library code
tests/             # Test suite
```

## Architecture Decisions

1. **Decision 1**: TBD — describe key architectural choice
2. **Decision 2**: TBD — describe key architectural choice

## Milestones

1. **M1 Foundation**: Project setup, core abstractions, build system
2. **M2 Core Implementation**: Primary feature set from specification
3. **M3 Integration & Polish**: Wire components, error handling, UX polish
4. **M4 Testing & Release**: Test suite, documentation, release preparation

## Risks & Mitigations

- **Risk**: TBD — identify key risk
  **Mitigation**: TBD — mitigation strategy

## Done Criteria

- Core requirements from specification are implemented
- Tests pass with adequate coverage
- Documentation is complete
- Code review has been performed
"##,
            spec = spec_name,
            req_summary = if requirements.len() > 500 {
                &requirements[..500]
            } else {
                requirements
            },
        )
    }

    /// Generate tasks from plan milestones
    fn generate_tasks(milestones: &[(String, String)], plan_file: &Path) -> String {
        let plan_name = plan_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "plan".to_string());

        let mut tasks = format!(
            r##"# Tasks

**Plan**: {plan}
**Generated**: auto-generated by spec-kit-mcp

## Task List

| ID | Phase/Milestone | Task | Priority | Dependencies | Status |
|----|-----------------|------|----------|-------------|--------|
"##,
            plan = plan_name,
        );

        if milestones.is_empty() {
            // Fallback tasks
            tasks.push_str(
                "| T001 | Foundation | Project setup and scaffolding | High | None | pending |\n",
            );
            tasks.push_str("| T002 | Core | Core implementation from plan | High | T001 | pending |\n");
            tasks.push_str("| T003 | Polish | Testing, docs, and polish | High | T002 | pending |\n");
        } else {
            for (i, (name, desc)) in milestones.iter().enumerate() {
                let tid = format!("T{:03}", i + 1);
                let short_name = if name.len() > 40 { &name[..40] } else { name };
                let short_desc = if desc.len() > 80 { &desc[..80] } else { desc };
                tasks.push_str(&format!(
                    "| {tid} | {name} | {desc} | High | None | pending |\n",
                    tid = tid,
                    name = short_name,
                    desc = short_desc,
                ));
            }
        }

        tasks.push_str(
            r##"
## Notes

- This task list was auto-generated by spec-kit-mcp. Review and refine before implementing.
- Break down further based on the implementation plan's phases and milestones.
"##,
        );

        tasks
    }

    /// Analyze project consistency
    pub async fn analyze(&self, project_path: &Path) -> Result<CommandResult> {
        let path_str = project_path.to_str().ok_or_else(|| {
            SpecKitError::InvalidPath("Project path contains invalid UTF-8".to_string())
        })?;

        let result = self
            .execute_command(&["analyze", "--path", path_str])
            .await?;

        if !result.is_success() {
            return Err(SpecKitError::command_failed(
                "specify analyze",
                &result.stderr,
                result.exit_code,
            )
            .into());
        }

        Ok(result)
    }
}

impl Default for SpecKitCli {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_cli_creation() {
        let cli = SpecKitCli::new();
        assert_eq!(cli.timeout_seconds, 300);
    }

    #[tokio::test]
    async fn test_constitution_write() {
        let cli = SpecKitCli::new_test_mode();
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("constitution.md");

        let result = cli
            .constitution("Test constitution", &output_path)
            .await
            .unwrap();

        assert!(result.is_success());
        assert!(output_path.exists());
    }

    #[tokio::test]
    async fn test_specify_write() {
        let cli = SpecKitCli::new_test_mode();
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("specification.md");

        let result = cli
            .specify("Test requirements", &output_path, "markdown")
            .await
            .unwrap();

        assert!(result.is_success());
        assert!(output_path.exists());
    }
}
