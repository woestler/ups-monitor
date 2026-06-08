use crate::monitor::ShutdownReason;
use log::{info, warn};
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ShutdownError {
    #[error("shutdown command is empty")]
    EmptyCommand,
    #[error("failed to execute shutdown command {program}: {source}")]
    Execute {
        program: String,
        source: std::io::Error,
    },
    #[error("shutdown command exited with status {status}: {stderr}")]
    Failed { status: String, stderr: String },
}

pub trait ShutdownExecutor {
    fn shutdown(&self, reason: &ShutdownReason) -> Result<(), ShutdownError>;
}

#[derive(Debug, Clone)]
pub struct CommandShutdownExecutor {
    dry_run: bool,
    command: Vec<String>,
}

impl CommandShutdownExecutor {
    pub fn new(dry_run: bool, command: Vec<String>) -> Self {
        Self { dry_run, command }
    }
}

impl ShutdownExecutor for CommandShutdownExecutor {
    fn shutdown(&self, reason: &ShutdownReason) -> Result<(), ShutdownError> {
        if self.command.is_empty() {
            return Err(ShutdownError::EmptyCommand);
        }

        if self.dry_run {
            warn!(
                "dry-run enabled; would execute shutdown command {:?}; reason: {}",
                self.command,
                reason.message()
            );
            return Ok(());
        }

        let program = &self.command[0];
        let args = &self.command[1..];
        info!(
            "executing shutdown command {:?}; reason: {}",
            self.command,
            reason.message()
        );

        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|source| ShutdownError::Execute {
                program: program.clone(),
                source,
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(ShutdownError::Failed {
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn dry_run_does_not_require_real_systemctl() {
        let executor = CommandShutdownExecutor::new(true, vec!["definitely-not-real".into()]);

        executor
            .shutdown(&ShutdownReason::OnBatteryTooLong {
                elapsed: Duration::from_secs(60),
                limit: Duration::from_secs(60),
            })
            .unwrap();
    }
}
