# krapy

Low-latency Android screen mirroring to Windows, macOS and Linux, built for
flaky Wi-Fi and phone hotspots. A light, open source scrcpy alternative.

*Krapy* means mirror in Jiwaka, from Jiwaka Province in Papua New Guinea.

A small Rust desktop app and a tiny Kotlin phone app. Scan a QR code to pair.
No adb, no developer options. If the phone's IP changes it finds the desktop
again over mDNS instead of freezing.

Status: v0.1, early. It builds and the protocol is tested end to end on
loopback. It has not been tested against a real phone yet.

Site: https://krapy.waghitech.com

## How it works

```
phone (Kotlin)                                desktop (Rust)
MediaProjection -> MediaCodec H.264  --UDP-->  reassemble -> decode -> wgpu
                 (hardware, low latency)       (drop late)   (ffmpeg)  (YUV shader)
       ^                                                         |
       +----------- TCP control: quality, keyframe, input -------+
```

- The phone encodes in hardware with a short GOP, no B-frames, and sends each
  frame as soon as it leaves the encoder.
- Video goes over UDP in fragments of at most 1100 bytes. A lost packet never
  stalls newer frames; incomplete older frames are dropped and a keyframe is
  requested.
- At most one frame waits between each stage on the desktop. The newest
  frame always wins.
- YUV goes straight to the GPU. Colour conversion happens in a shader.
- The desktop measures RTT, jitter and loss every 500 ms and sends a new
  bitrate/fps. It steps down fast and climbs back slowly.
- Everything is encrypted with ChaCha20-Poly1305. Keys come from the one-time
  key in the QR code. See [docs/protocol.md](docs/protocol.md).

## Layout

| path | what |
|------|------|
| `crates/krapy-cli` | the `krapy` binary |
| `crates/krapy-core` | presets, adaptive bitrate, backoff, paired devices |
| `crates/krapy-transport` | crypto, wire format, reassembly, session, mDNS, adb |
| `crates/krapy-decode` | `Decoder` trait, ffmpeg (default) and openh264 backends |
| `crates/krapy-render` | winit + wgpu window, input, clipboard |
| `android/` | the phone app |
| `web/` | krapy.waghitech.com |
| `scripts/` | lean FFmpeg build |

## Download

Grab the latest release: https://github.com/Emmanue707/krapy/releases/latest

- `krapy-<version>.apk` for the phone (Android 10 or newer)
- `krapy-<version>-linux-x86_64.tar.gz`, `-windows-x86_64.zip`, `-macos-aarch64.tar.gz` for the desktop

The Linux build has a minimal FFmpeg (H.264 decoder only) linked in
statically. Windows and macOS builds use openh264 for now.

`adb` is only needed for the adb fallback.

### Linux install

No sudo needed:

```
tar xzf krapy-<version>-linux-x86_64.tar.gz
cd krapy-<version>-linux-x86_64
mkdir -p ~/.local/bin && cp krapy ~/.local/bin/
krapy pair
```

If `krapy` is not found, open a new terminal or add `~/.local/bin` to your
`PATH`. Leave `krapy pair` running while the phone scans; it is the side the
phone connects to.

## Build from source

- Rust (stable)
- Lean static FFmpeg (recommended, needs `make` and `nasm`):

  ```
  scripts/build-ffmpeg.sh
  FFMPEG_DIR=$PWD/target/ffmpeg cargo build --release -p krapy-cli --no-default-features --features ffmpeg-static
  ```

- Or against system FFmpeg (Debian/Ubuntu: `sudo apt install pkg-config clang libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libavdevice-dev libavfilter-dev`), `cargo build --release`.
- Or no FFmpeg at all: `cargo build --release -p krapy-cli --no-default-features --features openh264`.

## Quickstart

```
cargo build --release
./target/release/krapy doctor
./target/release/krapy pair
```

Keep `krapy pair` running. Open the app, tap Scan QR, point it at the
terminal. Mirroring starts.

Next time:

```
krapy connect            # or: krapy connect <alias>
```

Other commands:

```
krapy list                      # paired phones, adb devices
krapy alias <device> <name>
krapy forget <device>
krapy pair --adb [serial]       # send the pairing link over adb, no scan
krapy connect --usb [--serial S]  # one-command USB mirroring (adb reverse)
krapy adb                         # same as connect --usb
krapy adb pair <host:port> <code>
krapy adb connect <host:port>
```

## Connect with USB (adb)

You do not need sudo to run krapy.

1. Install adb: `sudo dnf install android-tools` (Fedora) or `sudo apt install adb` (Ubuntu/Debian)
2. On the phone: Settings > About phone > tap Build number 7 times, then Developer options > USB debugging on
3. Plug the phone in and tap Allow on the "Allow USB debugging?" prompt
4. Run `krapy connect --usb` and tap Start now on the phone

Wireless debugging: `krapy adb pair <ip:port> <code>` then `krapy connect --usb` (adb reverse works over wireless adb too).

Useful flags: `--preset high-quality|balanced|low-latency|hotspot`,
`--bitrate`, `--fps`, `--max-size`, `--decoder ffmpeg|openh264`, `--stats`,
`--no-window`.

Touch and key input from the desktop need the krapy accessibility service
turned on in the phone's settings. Viewing works without it.

## Enable remote control (Android 13+)

Android 13 and newer block accessibility for apps installed outside the Play
Store, so the switch shows "Restricted setting" at first. Mirroring works
without it; this is only for desktop touch and keys.

1. Settings > Apps > krapy.
2. Tap the ⋮ menu (top right) > Allow restricted settings, then confirm with
   your PIN or fingerprint.
3. Settings > Accessibility > krapy > turn on.

## Presets

| preset | bitrate | fps | max size |
|--------|---------|-----|----------|
| high-quality | 16 Mbps | 60 | 1920 |
| balanced | 8 Mbps | 60 | 1600 |
| low-latency | 4 Mbps | 60 | 1280 |
| hotspot | 2 Mbps | 30 | 960 |

The preset is the ceiling. Adaptive control moves below it.

## Targets and measurements

Targets: under 50 ms glass-to-glass on good Wi-Fi, under 30 MB desktop RAM.

Measured so far, on one Linux VPS (4 vCPU, no GPU), `krapy pair --no-window
--stats` over loopback, fed by `examples/fakephone.rs` with a 1280x720 60 fps
4 Mbps single-slice H.264 clip. Software decode, no window, no real phone.

| build | idle RSS | peak RSS | decode | avg CPU |
|-------|----------|----------|--------|---------|
| lean static FFmpeg (`ffmpeg-static`) | 5.7 MB | 19.8 to 20.9 MB | 3.2 ms | 24% of one core |
| openh264 (`openh264`) | 7.0 MB | 30.0 to 30.3 MB | 4.2 ms | 28% |
| system FFmpeg 6.1 (default) | 20.4 MB | 36.0 to 36.4 MB | 3.2 ms | 25% |

All three hold 57 to 59 fps. Most of the system FFmpeg cost is shared
libraries the distro build drags in (x265, rsvg, glib, icu).

| other | measured |
|-------|----------|
| loopback RTT / jitter | 0.3 to 1.0 ms / 0.1 to 0.4 ms |
| with 3% packet loss | bitrate stepped 8 Mbps to the 0.5 Mbps floor in about 3 s |
| release APK (signed) | 525 KB |

Reproduce:

```
cargo build --release --example fakephone
./target/release/krapy pair --no-window --stats
./target/release/examples/fakephone '<uri from pair>' clip.h264 --fps 60 --seconds 10
```

Glass-to-glass latency has not been measured yet. That needs a real phone.

## Troubleshooting

**QR won't scan.** Update the phone app to the latest release. Use the QR
shown in the krapy mirror window (often easier than the terminal), or copy the
`uri:` line printed above the QR and use **Paste link instead** in the app.

**Phone can't reach the PC.** Phone and desktop must be on the same network
(not guest Wi-Fi with client isolation). On Fedora allow the port:

```
sudo firewall-cmd --add-port=27183/tcp --add-port=27183/udp
sudo firewall-cmd --permanent --add-port=27183/tcp --add-port=27183/udp && sudo firewall-cmd --reload
```

On Ubuntu: `sudo ufw allow 27183/tcp` and `sudo ufw allow 27183/udp`. On
Windows (admin cmd): `netsh advfirewall firewall add rule name=krapy dir=in
action=allow protocol=TCP localport=27183` and the same with `protocol=UDP`.
Run `krapy doctor` for a quick local check.

## Build the phone app

```
cd android
./gradlew assembleRelease
```

## Roadmap to v0.1

- [x] protocol, crypto and framing, tested on both sides with shared vectors
- [x] desktop: pair, connect, doctor, list, adb helpers
- [x] desktop: decode, render, input, clipboard, adaptive bitrate
- [x] phone: QR pairing, capture, encode, stream, reconnect over mDNS
- [x] phone: accessibility input, clipboard
- [ ] test on real phones and hotspots, fix what breaks
- [ ] measure glass-to-glass latency
- [ ] hardware decode (VAAPI, VideoToolbox, D3D11)
- [x] signed APK and desktop binaries on GitHub releases

## License

MIT or Apache-2.0, at your option. See LICENSE-MIT and LICENSE-APACHE.

Copyright (c) 2026 Emmanuel Kokele.
