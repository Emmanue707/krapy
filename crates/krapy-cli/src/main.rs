mod cmd;
mod mirror;
mod net;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use krapy_core::Preset;

use mirror::MirrorOpts;
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

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DecoderArg {
    Ffmpeg,
    Openh264,
}

impl DecoderArg {
    fn name(self) -> &'static str {
        match self {
            DecoderArg::Ffmpeg => "ffmpeg",
            DecoderArg::Openh264 => "openh264",
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
        #[command(flatten)]
        stream: StreamArgs,
        #[arg(long, default_value_t = default_desktop_name())]
        name: String,
        #[arg(
            long,
            num_args = 0..=1,
            default_missing_value = "",
            help = "launch pair uri on phone via adb instead of QR"
        )]
        adb: Option<String>,
    },
    Connect {
        device: Option<String>,
        #[command(flatten)]
        stream: StreamArgs,
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

#[derive(clap::Args)]
struct StreamArgs {
    #[arg(long, default_value_t = 27183)]
    port: u16,
    #[arg(long, value_enum, default_value_t = PresetArg::Balanced)]
    preset: PresetArg,
    #[arg(long, value_enum)]
    decoder: Option<DecoderArg>,
    #[arg(long)]
    max_size: Option<u16>,
    #[arg(long)]
    bitrate: Option<u32>,
    #[arg(long)]
    fps: Option<u8>,
    #[arg(long)]
    stats: bool,
    #[arg(long)]
    no_window: bool,
}

#[derive(Subcommand)]
enum AdbCommand {
    Pair { host_port: String, code: String },
    Connect { host_port: String },
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

fn stream_opts(args: &StreamArgs) -> MirrorOpts {
    MirrorOpts {
        preset: args.preset.into(),
        decoder: args.decoder.map(DecoderArg::name).map(str::to_string),
        max_size: args.max_size,
        bitrate: args.bitrate,
        fps: args.fps,
        stats: args.stats,
        no_window: args.no_window,
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    match cli.cmd {
        Command::List => cmd::list::run(),
        Command::Pair { stream, name, adb } => {
            cmd::pair::run(stream.port, &name, stream_opts(&stream), adb)
        }
        Command::Connect { device, stream } => {
            cmd::connect::run(device, stream.port, stream_opts(&stream))
        }
        Command::Doctor { port } => cmd::doctor::run(port),
        Command::Adb { cmd } => match cmd {
            AdbCommand::Pair { host_port, code } => cmd::adb::pair(&host_port, &code),
            AdbCommand::Connect { host_port } => cmd::adb::connect(&host_port),
        },
        Command::Alias { device, name } => cmd::alias::run(&device, &name),
        Command::Forget { device } => cmd::forget::run(&device),
    }
}
