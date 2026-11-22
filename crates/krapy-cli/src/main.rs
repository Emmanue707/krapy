mod cmd;
mod net;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use krapy_core::Preset;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum PresetArg {
    HighQuality,
    Balanced,
    LowLatency,
    Hotspot,
}

impl From<PresetArg> for Preset {
    fn from(p: PresetArg) -> Self {
        match p {
            PresetArg::HighQuality => Preset::HighQuality,
            PresetArg::Balanced => Preset::Balanced,
            PresetArg::LowLatency => Preset::LowLatency,
            PresetArg::Hotspot => Preset::Hotspot,
        }
    }
}

#[derive(Parser)]
#[command(name = "krapy", version, about = "krapy desktop")]
struct Cli {
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    List,
    Pair {
        #[arg(long, default_value_t = 27183)]
        port: u16,
        #[arg(long, default_value_t = default_desktop_name())]
        name: String,
        #[arg(long, value_enum, default_value_t = PresetArg::Balanced)]
        preset: PresetArg,
        #[arg(
            long,
            num_args = 0..=1,
            default_missing_value = "",
            help = "launch pair uri on phone via adb instead of QR"
        )]
        adb: Option<String>,
    },
    Connect {
        device: String,
        #[arg(long, value_enum, default_value_t = PresetArg::Balanced)]
        preset: PresetArg,
        #[arg(long, default_value_t = 27183)]
        port: u16,
    },
    Doctor {
        #[arg(long, default_value_t = 27183)]
        port: u16,
    },
    Adb {
        #[command(subcommand)]
        cmd: AdbCommand,
    },
    Alias {
        device: String,
        name: String,
    },
    Forget {
        device: String,
    },
}

#[derive(Subcommand)]
enum AdbCommand {
    Pair {
        host_port: String,
        code: String,
    },
    Connect {
        host_port: String,
    },
}

fn default_desktop_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "krapy-desktop".into())
}

fn init_tracing(verbose: u8) {
    let default = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match cli.cmd {
        Command::List => cmd::list::run(),
        Command::Pair {
            port,
            name,
            preset,
            adb,
        } => cmd::pair::run(port, &name, preset.into(), adb),
        Command::Connect {
            device,
            preset,
            port,
        } => cmd::connect::run(&device, preset.into(), port),
        Command::Doctor { port } => cmd::doctor::run(port),
        Command::Adb { cmd } => match cmd {
            AdbCommand::Pair { host_port, code } => cmd::adb::pair(&host_port, &code),
            AdbCommand::Connect { host_port } => cmd::adb::connect(&host_port),
        },
        Command::Alias { device, name } => cmd::alias::run(&device, &name),
        Command::Forget { device } => cmd::forget::run(&device),
    }
}