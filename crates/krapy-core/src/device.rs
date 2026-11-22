use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[derive(Debug, Clone)]
pub struct Identity {
    secret: [u8; 32],
    path: PathBuf,
}

impl Identity {
    pub fn load(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir).context("create config dir")?;
        let path = dir.join("identity");
        if path.exists() {
            let bytes = fs::read(&path).context("read identity")?;
            if bytes.len() != 32 {
                anyhow::bail!("identity file must be 32 bytes");
            }
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            Ok(Self { secret, path })
        } else {
            let mut secret = [0u8; 32];
            rand::rng().fill_bytes(&mut secret);
            Self::write_secret(&path, &secret)?;
            Ok(Self { secret, path })
        }
    }

    #[cfg(unix)]
    fn write_secret(path: &Path, secret: &[u8; 32]) -> Result<()> {
        use std::fs::OpenOptions;
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .context("create identity")?;
        f.write_all(secret)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn write_secret(path: &Path, secret: &[u8; 32]) -> Result<()> {
        fs::write(path, secret).context("write identity")?;
        Ok(())
    }

    pub fn id(&self) -> String {
        let hash = Sha256::digest(self.secret);
        hex::encode(&hash[..8])
    }

    pub fn secret(&self) -> &[u8; 32] {
        &self.secret
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeviceFile {
    devices: Vec<PairedDevice>,
}

#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    devices: Vec<PairedDevice>,
}

impl Store {
    pub fn config_dir() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "krapy").context("resolve config dir")?;
        Ok(dirs.config_dir().to_path_buf())
    }

    pub fn open_default() -> Result<Self> {
        let dir = Self::config_dir()?;
        Self::open(dir.join("devices.json"))
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create store dir")?;
        }
        let devices = if path.exists() {
            let data = fs::read_to_string(&path).context("read devices.json")?;
            if data.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str::<DeviceFile>(&data)
                    .context("parse devices.json")?
                    .devices
            }
        } else {
            Vec::new()
        };
        Ok(Self { path, devices })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn devices(&self) -> &[PairedDevice] {
        &self.devices
    }

    pub fn save(&self) -> Result<()> {
        let file = DeviceFile {
            devices: self.devices.clone(),
        };
        let json = serde_json::to_string_pretty(&file).context("serialize devices")?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).context("write devices temp")?;
        fs::rename(&tmp, &self.path).context("rename devices")?;
        Ok(())
    }

    pub fn find(&self, id_or_alias: &str) -> Option<&PairedDevice> {
        self.devices
            .iter()
            .find(|d| d.id == id_or_alias || d.alias.as_deref() == Some(id_or_alias))
    }

    pub fn upsert(&mut self, device: PairedDevice) -> Result<()> {
        if let Some(pos) = self.devices.iter().position(|d| d.id == device.id) {
            self.devices[pos] = device;
        } else {
            self.devices.push(device);
        }
        self.save()
    }

    pub fn remove(&mut self, id_or_alias: &str) -> Result<bool> {
        let before = self.devices.len();
        self.devices
            .retain(|d| d.id != id_or_alias && d.alias.as_deref() != Some(id_or_alias));
        let removed = self.devices.len() < before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn set_alias(&mut self, id_or_alias: &str, alias: Option<String>) -> Result<bool> {
        let id = self
            .devices
            .iter()
            .find(|d| d.id == id_or_alias || d.alias.as_deref() == Some(id_or_alias))
            .map(|d| d.id.clone());
        let Some(id) = id else {
            return Ok(false);
        };
        if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
            d.alias = alias;
            self.save()?;
            return Ok(true);
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn identity_id_stable() {
        let dir = tempdir().unwrap();
        let id1 = Identity::load(dir.path()).unwrap();
        let hex = id1.id();
        assert_eq!(hex.len(), 16);
        let id2 = Identity::load(dir.path()).unwrap();
        assert_eq!(id1.id(), id2.id());
    }

    #[test]
    fn store_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let mut s = Store::open(path.clone()).unwrap();
        let dev = PairedDevice {
            id: "abcd1234".into(),
            name: "phone".into(),
            alias: None,
            secret: "aa".repeat(64),
            last_addr: Some("10.0.0.2:27183".into()),
            last_seen: Some(1),
        };
        s.upsert(dev.clone()).unwrap();
        let s2 = Store::open(path).unwrap();
        assert_eq!(s2.find("abcd1234"), Some(&dev));
    }

    #[test]
    fn find_by_alias_and_forget() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let mut s = Store::open(path.clone()).unwrap();
        s.upsert(PairedDevice {
            id: "id1".into(),
            name: "n".into(),
            alias: Some("kitchen".into()),
            secret: "bb".repeat(64),
            last_addr: None,
            last_seen: None,
        })
        .unwrap();
        assert!(s.find("kitchen").is_some());
        assert!(s.set_alias("kitchen", Some("den".into())).unwrap());
        assert!(s.remove("den").unwrap());
        assert!(s.find("id1").is_none());
    }
}
