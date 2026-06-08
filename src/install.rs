use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};
use thiserror::Error;

pub const DEFAULT_BINARY_PATH: &str = "/usr/local/bin/ups-monitor";
pub const DEFAULT_CONFIG_PATH: &str = "/etc/ups-monitor.yaml";
pub const DEFAULT_SERVICE_PATH: &str = "/etc/systemd/system/ups-monitor.service";

#[derive(Debug, Clone)]
pub struct InitOptions {
    pub binary_path: PathBuf,
    pub config_path: PathBuf,
    pub service_path: PathBuf,
    pub force: bool,
    pub skip_binary_install: bool,
    pub enable: bool,
    pub start: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InitReport {
    pub binary_installed: bool,
    pub config_written: bool,
    pub service_written: bool,
    pub daemon_reloaded: bool,
    pub enabled: bool,
    pub started: bool,
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("init is only supported on Linux systems")]
    UnsupportedPlatform,
    #[error("failed to locate current executable: {0}")]
    CurrentExe(std::io::Error),
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to copy binary from {from} to {to}: {source}")]
    CopyBinary {
        from: PathBuf,
        to: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to set permissions on {path}: {source}")]
    Permissions {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {kind} to {path}: {source}")]
    Write {
        kind: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("systemctl {action} failed with status {status}: {stderr}")]
    Systemctl {
        action: &'static str,
        status: String,
        stderr: String,
    },
    #[error("failed to execute systemctl {action}: {source}")]
    ExecuteSystemctl {
        action: &'static str,
        source: std::io::Error,
    },
}

impl Default for InitOptions {
    fn default() -> Self {
        Self {
            binary_path: DEFAULT_BINARY_PATH.into(),
            config_path: DEFAULT_CONFIG_PATH.into(),
            service_path: DEFAULT_SERVICE_PATH.into(),
            force: false,
            skip_binary_install: false,
            enable: false,
            start: false,
        }
    }
}

pub fn init_linux_service(
    options: &InitOptions,
    default_config: &str,
) -> Result<InitReport, InitError> {
    if !cfg!(target_os = "linux") {
        return Err(InitError::UnsupportedPlatform);
    }

    let mut report = InitReport::default();

    if !options.skip_binary_install {
        install_current_binary(&options.binary_path)?;
        report.binary_installed = true;
    }

    report.config_written = write_config(default_config, &options.config_path, options.force)?;
    report.service_written = write_service(
        &options.binary_path,
        &options.config_path,
        &options.service_path,
        options.force,
    )?;

    run_systemctl("daemon-reload", &["daemon-reload"])?;
    report.daemon_reloaded = true;

    if options.enable {
        run_systemctl(
            "enable ups-monitor.service",
            &["enable", "ups-monitor.service"],
        )?;
        report.enabled = true;
    }

    if options.start {
        run_systemctl(
            "start ups-monitor.service",
            &["start", "ups-monitor.service"],
        )?;
        report.started = true;
    }

    Ok(report)
}

fn install_current_binary(destination: &Path) -> Result<(), InitError> {
    let current_exe = env::current_exe().map_err(InitError::CurrentExe)?;
    if same_path(&current_exe, destination) {
        return Ok(());
    }

    ensure_parent(destination)?;
    fs::copy(&current_exe, destination).map_err(|source| InitError::CopyBinary {
        from: current_exe,
        to: destination.to_path_buf(),
        source,
    })?;
    set_mode(destination, 0o755)
}

fn write_config(default_config: &str, path: &Path, force: bool) -> Result<bool, InitError> {
    if path.exists() && !force {
        return Ok(false);
    }
    ensure_parent(path)?;
    fs::write(path, default_config).map_err(|source| InitError::Write {
        kind: "configuration file",
        path: path.to_path_buf(),
        source,
    })?;
    set_mode(path, 0o644)?;
    Ok(true)
}

fn write_service(
    binary_path: &Path,
    config_path: &Path,
    service_path: &Path,
    force: bool,
) -> Result<bool, InitError> {
    if service_path.exists() && !force {
        return Ok(false);
    }

    ensure_parent(service_path)?;
    fs::write(
        service_path,
        render_systemd_service(binary_path, config_path),
    )
    .map_err(|source| InitError::Write {
        kind: "systemd service",
        path: service_path.to_path_buf(),
        source,
    })?;
    set_mode(service_path, 0o644)?;
    Ok(true)
}

fn render_systemd_service(binary_path: &Path, config_path: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=SNMP UPS Monitor\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} --config {} run\n\
         Restart=always\n\
         RestartSec=10\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        systemd_arg(binary_path),
        systemd_arg(config_path)
    )
}

fn systemd_arg(path: &Path) -> String {
    let value = path.to_string_lossy();
    if value
        .chars()
        .all(|ch| !ch.is_whitespace() && ch != '"' && ch != '\\')
    {
        return value.into_owned();
    }

    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn run_systemctl(action: &'static str, args: &[&str]) -> Result<(), InitError> {
    let output = Command::new("systemctl")
        .args(args)
        .output()
        .map_err(|source| InitError::ExecuteSystemctl { action, source })?;

    if !output.status.success() {
        return Err(InitError::Systemctl {
            action,
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(())
}

fn ensure_parent(path: &Path) -> Result<(), InitError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| InitError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<(), InitError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
        InitError::Permissions {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_service_with_config_path() {
        let service = render_systemd_service(
            Path::new("/usr/local/bin/ups-monitor"),
            Path::new("/etc/ups-monitor.yaml"),
        );

        assert!(service
            .contains("ExecStart=/usr/local/bin/ups-monitor --config /etc/ups-monitor.yaml run"));
        assert!(service.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn quotes_systemd_arguments_with_spaces() {
        let arg = systemd_arg(Path::new("/opt/UPS Monitor/bin/ups-monitor"));

        assert_eq!(arg, "\"/opt/UPS Monitor/bin/ups-monitor\"");
    }
}
