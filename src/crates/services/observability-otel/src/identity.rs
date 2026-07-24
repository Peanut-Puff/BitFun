use crate::TelemetryRuntimeError;
use fs2::FileExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const INSTALLATION_ID_FILE: &str = "installation-id";
const INSTALLATION_ID_LOCK_FILE: &str = ".installation-id.lock";

#[derive(Debug, Clone)]
pub struct InstallationIdentityStore {
    directory: PathBuf,
}

impl InstallationIdentityStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn identity_path(&self) -> PathBuf {
        self.directory.join(INSTALLATION_ID_FILE)
    }

    pub fn scoped_id(&self, audience: &str) -> Result<String, TelemetryRuntimeError> {
        let root = self.load_or_create_root()?;
        let mut mac = Hmac::<Sha256>::new_from_slice(root.as_bytes())
            .map_err(|_| TelemetryRuntimeError::InvalidConfig("identity key is invalid"))?;
        mac.update(b"bitfun-installation-v1\0");
        mac.update(audience.as_bytes());
        let digest = mac.finalize().into_bytes();
        Ok(hex::encode(&digest[..16]))
    }

    pub fn reset(&self) -> Result<bool, TelemetryRuntimeError> {
        let lock = self.lock()?;
        let removed = match std::fs::remove_file(self.identity_path()) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(TelemetryRuntimeError::Identity(error)),
        };
        drop(lock);
        Ok(removed)
    }

    fn load_or_create_root(&self) -> Result<Uuid, TelemetryRuntimeError> {
        let lock = self.lock()?;
        let result = match read_root(&self.identity_path()) {
            Ok(root) => Ok(root),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => self.create_root(),
            Err(error) => Err(TelemetryRuntimeError::Identity(error)),
        };
        drop(lock);
        result
    }

    fn lock(&self) -> Result<File, TelemetryRuntimeError> {
        std::fs::create_dir_all(&self.directory).map_err(TelemetryRuntimeError::Identity)?;
        set_directory_permissions(&self.directory).map_err(TelemetryRuntimeError::Identity)?;
        let lock_path = self.directory.join(INSTALLATION_ID_LOCK_FILE);
        let lock = secure_open(&lock_path, false).map_err(TelemetryRuntimeError::Identity)?;
        lock.lock_exclusive()
            .map_err(TelemetryRuntimeError::Identity)?;
        Ok(lock)
    }

    fn create_root(&self) -> Result<Uuid, TelemetryRuntimeError> {
        let root = Uuid::new_v4();
        let temporary = self
            .directory
            .join(format!(".{INSTALLATION_ID_FILE}.{}.tmp", Uuid::new_v4()));
        let mut file = secure_open(&temporary, true).map_err(TelemetryRuntimeError::Identity)?;
        file.write_all(root.hyphenated().to_string().as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(TelemetryRuntimeError::Identity)?;
        drop(file);
        if let Err(error) = std::fs::rename(&temporary, self.identity_path()) {
            let _ = std::fs::remove_file(&temporary);
            return Err(TelemetryRuntimeError::Identity(error));
        }
        Ok(root)
    }
}

fn read_root(path: &Path) -> std::io::Result<Uuid> {
    set_file_permissions(path)?;
    let mut contents = String::new();
    File::open(path)?.take(128).read_to_string(&mut contents)?;
    Uuid::parse_str(contents.trim()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "installation identity is corrupt",
        )
    })
}

fn secure_open(path: &Path, truncate: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .truncate(truncate);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    set_file_permissions(path)?;
    Ok(file)
}

#[cfg(unix)]
fn set_directory_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_directory_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_file_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_ids_are_stable_per_audience_and_root_is_not_returned() {
        let temporary = tempfile::tempdir().unwrap();
        let store = InstallationIdentityStore::new(temporary.path().join("telemetry"));

        let first = store.scoped_id("https://collector-a.test").unwrap();
        let repeated = store.scoped_id("https://collector-a.test").unwrap();
        let other = store.scoped_id("https://collector-b.test").unwrap();
        let root = std::fs::read_to_string(store.identity_path()).unwrap();

        assert_eq!(first, repeated);
        assert_ne!(first, other);
        assert_eq!(first.len(), 32);
        assert!(!first.contains(root.trim()));
    }

    #[test]
    fn reset_rotates_the_next_scoped_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let store = InstallationIdentityStore::new(temporary.path().join("telemetry"));
        let before = store.scoped_id("audience").unwrap();

        assert!(store.reset().unwrap());
        let after = store.scoped_id("audience").unwrap();

        assert_ne!(before, after);
    }

    #[cfg(unix)]
    #[test]
    fn identity_is_stored_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let store = InstallationIdentityStore::new(temporary.path().join("telemetry"));
        store.scoped_id("audience").unwrap();

        let mode = std::fs::metadata(store.identity_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
