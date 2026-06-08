use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
    TerminalMode, WriteLogger,
};
use std::{
    fs::{self, File},
    path::PathBuf,
};
use ups_monitor::{
    config::AppConfig,
    install::{init_linux_service, InitOptions, DEFAULT_BINARY_PATH, DEFAULT_SYSTEMD_SERVICE_PATH},
    monitor::run_monitor_loop,
    shutdown::CommandShutdownExecutor,
    snmp::{RustSnmpClient, SnmpClient},
    ups::build_adapter,
};

const DEFAULT_CONFIG: &str = include_str!("../examples/ups-monitor.yaml");

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Small SNMP UPS monitor with automatic shutdown"
)]
struct Cli {
    #[arg(short, long, default_value = "/etc/ups-monitor.yaml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run {
        #[arg(long, help = "Poll once and exit")]
        once: bool,
    },
    Init {
        #[arg(long, default_value = DEFAULT_BINARY_PATH)]
        binary_path: PathBuf,
        #[arg(long)]
        config_path: Option<PathBuf>,
        #[arg(
            long,
            default_value = DEFAULT_SYSTEMD_SERVICE_PATH,
            help = "Systemd unit path. /etc/systemd/system is portable across systemd distributions"
        )]
        service_path: PathBuf,
        #[arg(long, help = "Overwrite existing config and service files")]
        force: bool,
        #[arg(long, help = "Do not copy this executable to the service binary path")]
        skip_binary_install: bool,
        #[arg(long, help = "Run systemctl enable ups-monitor.service")]
        enable: bool,
        #[arg(long, help = "Run systemctl start ups-monitor.service")]
        start: bool,
    },
    CheckConfig,
    PrintDefaultConfig,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Run { once: false }) {
        Command::PrintDefaultConfig => {
            print!("{DEFAULT_CONFIG}");
            Ok(())
        }
        Command::CheckConfig => {
            AppConfig::from_path(&cli.config)
                .with_context(|| format!("loading {}", cli.config.display()))?;
            println!("configuration is valid: {}", cli.config.display());
            Ok(())
        }
        Command::Init {
            binary_path,
            config_path,
            service_path,
            force,
            skip_binary_install,
            enable,
            start,
        } => {
            let config_path = config_path.unwrap_or(cli.config);
            let options = InitOptions {
                binary_path,
                config_path,
                service_path,
                force,
                skip_binary_install,
                enable,
                start,
            };
            let report = init_linux_service(&options, DEFAULT_CONFIG)?;
            print_init_report(&options, &report);
            Ok(())
        }
        Command::Run { once } => {
            let config = AppConfig::from_path(&cli.config)
                .with_context(|| format!("loading {}", cli.config.display()))?;
            init_logging(&config)?;

            let snmp: Box<dyn SnmpClient> = Box::new(RustSnmpClient::new(config.snmp.clone()));
            let adapter = build_adapter(config.ups.clone())?;
            let shutdown =
                CommandShutdownExecutor::new(config.shutdown.dry_run, config.shutdown.command);

            log::info!("starting UPS monitor; once={once}");
            run_monitor_loop(
                config.monitor,
                adapter.as_ref(),
                snmp.as_ref(),
                &shutdown,
                once,
            )?;
            Ok(())
        }
    }
}

fn print_init_report(options: &InitOptions, report: &ups_monitor::install::InitReport) {
    if report.binary_installed {
        println!("installed binary: {}", options.binary_path.display());
    } else if options.skip_binary_install {
        println!("skipped binary install");
    }

    if report.config_written {
        println!("wrote sample config: {}", options.config_path.display());
    } else {
        println!(
            "kept existing config: {} (use --force to overwrite)",
            options.config_path.display()
        );
    }

    if report.service_written {
        println!("wrote systemd service: {}", options.service_path.display());
    } else {
        println!(
            "kept existing systemd service: {} (use --force to overwrite)",
            options.service_path.display()
        );
    }

    if report.daemon_reloaded {
        println!("reloaded systemd manager configuration");
    }
    if report.enabled {
        println!("enabled ups-monitor.service");
    }
    if report.started {
        println!("started ups-monitor.service");
    }

    if !report.started {
        println!(
            "edit {}, then run: sudo systemctl enable --now ups-monitor.service",
            options.config_path.display()
        );
    }
}

fn init_logging(config: &AppConfig) -> Result<()> {
    let level = parse_level(&config.logging.level);
    let log_config = ConfigBuilder::new().set_time_format_rfc3339().build();
    let mut loggers: Vec<Box<dyn SharedLogger>> = Vec::new();

    loggers.push(TermLogger::new(
        level,
        log_config.clone(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    ));

    if let Some(path) = &config.logging.file {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating log directory {}", parent.display()))?;
        }
        let file = File::create(path).with_context(|| format!("opening log {}", path.display()))?;
        loggers.push(WriteLogger::new(level, log_config, file));
    }

    CombinedLogger::init(loggers).context("initializing logger")?;
    Ok(())
}

fn parse_level(level: &str) -> LevelFilter {
    match level.trim().to_ascii_lowercase().as_str() {
        "off" => LevelFilter::Off,
        "error" => LevelFilter::Error,
        "warn" | "warning" => LevelFilter::Warn,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    }
}
