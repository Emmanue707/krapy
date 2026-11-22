use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quality {
    pub bitrate: u32,
    pub fps: u8,
    pub max_size: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    HighQuality,
    Balanced,
    LowLatency,
    Hotspot,
}

impl Preset {
    pub fn quality(self) -> Quality {
        match self {
            Preset::HighQuality => Quality {
                bitrate: 16_000_000,
                fps: 60,
                max_size: 1920,
            },
            Preset::Balanced => Quality {
                bitrate: 8_000_000,
                fps: 60,
                max_size: 1600,
            },
            Preset::LowLatency => Quality {
                bitrate: 4_000_000,
                fps: 60,
                max_size: 1280,
            },
            Preset::Hotspot => Quality {
                bitrate: 2_000_000,
                fps: 30,
                max_size: 960,
            },
        }
    }
}

impl fmt::Display for Preset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Preset {
    pub fn as_str(self) -> &'static str {
        match self {
            Preset::HighQuality => "high-quality",
            Preset::Balanced => "balanced",
            Preset::LowLatency => "low-latency",
            Preset::Hotspot => "hotspot",
        }
    }
}

impl FromStr for Preset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "high-quality" => Ok(Preset::HighQuality),
            "balanced" => Ok(Preset::Balanced),
            "low-latency" => Ok(Preset::LowLatency),
            "hotspot" => Ok(Preset::Hotspot),
            _ => Err(format!("unknown preset {:?}", s)),
        }
    }
}
