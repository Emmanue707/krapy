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
    /// highest quality, more bandwidth
    HighQuality,
    /// default balance of quality and latency
    Balanced,
    /// favor low latency over quality
    LowLatency,
    /// tuned for phone hotspot links
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
    /// ffmpeg software decoder
    Ffmpeg,
    /// openh264 software decoder
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
#[command(
    name = "krapy",
    version,
    about = "Low-latency Android screen mirroring. Pair with a QR scan, then mirror over Wi-Fi."
)]
struct Cli {
    /// More logs (-v debug, -vv trace)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show paired phones and adb devices
    List,
    /// Show a QR code for the phone app to scan; keep it running until the phone connects
    Pair {
        #[command(flatten)]
        stream: StreamArgs,
        /// name shown to the phone during pairing
        #[arg(long, default_value_t = default_desktop_name())]
        name: String,
        /// open pair uri on phone via adb instead of QR (optional device serial)
        #[arg(long, num_args = 0..=1, default_missing_value = "", value_name = "SERIAL")]
        adb: Option<String>,
    },
    /// Mirror a paired phone (finds it again over mDNS)
    Connect {
        /// device id or alias (default when only one paired phone)
        device: Option<String>,
        #[command(flatten)]
        stream: StreamArgs,
    },
    /// Check network, port and adb setup
    Doctor {
        /// port to check for bindability
        #[arg(long, default_value_t = 27183)]
        port: u16,
    },
    /// adb fallback: pair or connect over wireless debugging
    Adb {
        #[command(subcommand)]
        cmd: AdbCommand,
    },
    /// Give a paired phone a short name
    Alias {
        /// device id or alias
        device: String,
        /// short name to remember this phone
        name: String,
    },
    /// Remove a paired phone
    Forget {
        /// device id or alias
        device: String,
    },
}

#[derive(clap::Args)]
struct StreamArgs {
    /// listener port for the phone to connect
    #[arg(long, default_value_t = 27183)]
    port: u16,
    /// stream quality preset
    #[arg(long, value_enum, default_value_t = PresetArg::Balanced)]
    preset: PresetArg,
    /// video decoder backend
    #[arg(long, value_enum)]
    decoder: Option<DecoderArg>,
    /// max video dimension in pixels
    #[arg(long)]
    max_size: Option<u16>,
    /// target video bitrate
    #[arg(long, value_name = "MBPS")]
    bitrate: Option<u32>,
    /// max capture fps
    #[arg(long)]
    fps: Option<u8>,
    /// print stream stats
    #[arg(long)]
    stats: bool,
    /// run without a mirror window
    #[arg(long)]
    no_window: bool,
}

#[derive(Subcommand)]
enum AdbCommand {
    /// pair over wireless debugging
    Pair {
        /// phone wireless debugging address
        #[arg(value_name = "HOST:PORT")]
        host_port: String,
        /// pairing code from the phone
        #[arg(value_name = "CODE")]
        code: String,
    },
    /// connect to a paired wireless adb endpoint
    Connect {
        /// phone wireless debugging address
        #[arg(value_name = "HOST:PORT")]
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
        0 => "info,wgpu_hal=error,wgpu_core=error,naga=error,winit=warn",
        1 => "debug,wgpu_hal=warn,wgpu_core=warn,naga=warn,winit=warn",
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
