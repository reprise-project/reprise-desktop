//! Drives and volumes as UDisks2 reports them, and the calls that mount,
//! unlock and safely remove them. [`fold`] reads the map
//! `GetManagedObjects` returns without a bus, so it is testable; [`Udisks`]
//! is a blocking client over the system bus for callers without an
//! executor of their own. Polkit decides every call; the session's agent
//! prompts when it has to.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

pub const NAME: &str = "org.freedesktop.UDisks2";
pub const ROOT: &str = "/org/freedesktop/UDisks2";
pub const BLOCK_IFACE: &str = "org.freedesktop.UDisks2.Block";
pub const FILESYSTEM_IFACE: &str = "org.freedesktop.UDisks2.Filesystem";
pub const ENCRYPTED_IFACE: &str = "org.freedesktop.UDisks2.Encrypted";
pub const DRIVE_IFACE: &str = "org.freedesktop.UDisks2.Drive";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

/// How long a call waits: a mount or an unlock can hold on a polkit
/// prompt the user is still reading.
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// One interface's properties as `GetManagedObjects` returns them.
pub type Props = HashMap<String, OwnedValue>;
/// Every object UDisks2 has, by path, with its interfaces.
pub type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

/// A drive: the stick, card reader or disk that holds volumes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Disk {
    pub path: String,
    pub vendor: String,
    pub model: String,
    /// The drive or its media can come out: `Removable`, `MediaRemovable`
    /// or `Ejectable`, or it sits on USB, SDIO or FireWire.
    pub removable: bool,
    pub optical: bool,
    pub can_power_off: bool,
    pub ejectable: bool,
}

impl Disk {
    /// "Vendor Model", or a generic name when the drive reports neither.
    pub fn name(&self) -> String {
        let name = format!("{} {}", self.vendor.trim(), self.model.trim());
        match name.trim() {
            "" if self.optical => "CD Drive".into(),
            "" => "USB Mass Storage Device".into(),
            n => n.to_string(),
        }
    }
}

/// A block device: a whole disk, a partition, or the cleartext side of
/// an unlocked encrypted volume.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Volume {
    pub path: String,
    /// `/dev/sdb1`.
    pub device: String,
    /// The name the system mounts it by, where that differs from
    /// `device`: `/dev/mapper/luks-...` for a cleartext volume.
    pub preferred: String,
    /// The drive it lives on. A cleartext volume has none of its own, so
    /// it takes its encrypted backing volume's.
    pub disk: Option<String>,
    pub label: Option<String>,
    /// `IdType`: `vfat`, `ext4`, `crypto_LUKS`.
    pub fs_type: String,
    pub size: u64,
    /// It carries a file system UDisks2 can mount.
    pub filesystem: bool,
    /// Where it is mounted now; empty when it is not.
    pub mounts: Vec<PathBuf>,
    /// It is an encrypted container.
    pub encrypted: bool,
    /// For an unlocked container: its cleartext volume.
    pub cleartext: Option<String>,
    /// For a cleartext volume: the container it was unlocked from.
    pub backing: Option<String>,
    /// `HintIgnore`: not to be shown.
    pub ignore: bool,
    /// `HintSystem`: part of the system, not the user's to mount.
    pub system: bool,
    /// `HintAuto`: fine to mount on its own when it appears.
    pub auto: bool,
}

impl Volume {
    /// An encrypted container with no cleartext volume.
    pub fn locked(&self) -> bool {
        self.encrypted && self.cleartext.is_none()
    }

    pub fn name(&self, disk: Option<&Disk>) -> String {
        match &self.label {
            Some(l) => l.clone(),
            None if disk.is_some_and(|d| d.optical) => "CD Drive".into(),
            None => "Removable Disk".into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// By path.
    pub disks: Vec<Disk>,
    /// By device.
    pub volumes: Vec<Volume>,
}

impl Snapshot {
    pub fn disk(&self, path: &str) -> Option<&Disk> {
        self.disks.iter().find(|d| d.path == path)
    }

    pub fn volume(&self, path: &str) -> Option<&Volume> {
        self.volumes.iter().find(|v| v.path == path)
    }

    /// The volume behind `/dev/sdb1` or `/dev/mapper/luks-...`, as
    /// `/proc/mounts` names it.
    pub fn volume_by_device(&self, device: &str) -> Option<&Volume> {
        self.volumes
            .iter()
            .find(|v| v.device == device || v.preferred == device)
    }

    /// Whether the device `/proc/mounts` names is a removable volume
    /// (a cleartext one included).
    pub fn removable_device(&self, device: &str) -> bool {
        self.volume_by_device(device)
            .is_some_and(|v| self.removable(v))
    }

    pub fn disk_of(&self, v: &Volume) -> Option<&Disk> {
        v.disk.as_deref().and_then(|p| self.disk(p))
    }

    /// Whether `v` is the user's to mount and remove: shown, not the
    /// system's, on a removable drive.
    pub fn removable(&self, v: &Volume) -> bool {
        !v.ignore && !v.system && self.disk_of(v).is_some_and(|d| d.removable)
    }

    /// Removable volumes with a file system that is not mounted, and
    /// removable containers still locked.
    pub fn unmounted(&self) -> Vec<&Volume> {
        self.volumes
            .iter()
            .filter(|v| (v.filesystem || v.locked()) && v.mounts.is_empty() && self.removable(v))
            .collect()
    }

    /// Every volume on drive `disk`, cleartext ones included.
    pub fn volumes_on(&self, disk: &str) -> Vec<&Volume> {
        self.volumes
            .iter()
            .filter(|v| v.disk.as_deref() == Some(disk))
            .collect()
    }
}

fn flag(p: &Props, key: &str) -> bool {
    p.get(key)
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false)
}

fn text(p: &Props, key: &str) -> String {
    p.get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
}

/// An object path property, None for `/` (UDisks2's "none").
fn object(p: &Props, key: &str) -> Option<String> {
    p.get(key)
        .and_then(|v| OwnedObjectPath::try_from(v.clone()).ok())
        .map(|o| o.as_str().to_string())
        .filter(|o| o != "/")
}

/// A NUL-terminated byte string (`ay`) as a path.
fn bytes_path(b: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    PathBuf::from(std::ffi::OsString::from_vec(b[..end].to_vec()))
}

/// A device node property (`ay`) as text; the kernel names them in ASCII.
fn dev_name(p: &Props, key: &str) -> String {
    p.get(key)
        .and_then(|v| Vec::<u8>::try_from(v.clone()).ok())
        .map(|b| bytes_path(b).to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn disk_of(path: &str, p: &Props) -> Disk {
    let bus = text(p, "ConnectionBus");
    Disk {
        path: path.to_string(),
        vendor: text(p, "Vendor"),
        model: text(p, "Model"),
        removable: flag(p, "Removable")
            || flag(p, "MediaRemovable")
            || flag(p, "Ejectable")
            || matches!(bus.as_str(), "usb" | "sdio" | "ieee1394"),
        optical: flag(p, "Optical"),
        can_power_off: flag(p, "CanPowerOff"),
        ejectable: flag(p, "Ejectable"),
    }
}

fn volume_of(path: &str, ifaces: &HashMap<String, Props>, block: &Props) -> Volume {
    let fs = ifaces.get(FILESYSTEM_IFACE);
    let crypt = ifaces.get(ENCRYPTED_IFACE);
    let label = Some(text(block, "IdLabel"))
        .filter(|l| !l.is_empty())
        .or_else(|| Some(text(block, "HintName")).filter(|l| !l.is_empty()));
    Volume {
        path: path.to_string(),
        device: dev_name(block, "Device"),
        preferred: dev_name(block, "PreferredDevice"),
        disk: object(block, "Drive"),
        label,
        fs_type: text(block, "IdType"),
        size: block
            .get("Size")
            .and_then(|v| u64::try_from(v.clone()).ok())
            .unwrap_or(0),
        filesystem: fs.is_some(),
        mounts: fs
            .and_then(|f| f.get("MountPoints"))
            .and_then(|v| Vec::<Vec<u8>>::try_from(v.clone()).ok())
            .map(|ms| ms.into_iter().map(bytes_path).collect())
            .unwrap_or_default(),
        encrypted: crypt.is_some(),
        cleartext: crypt.and_then(|c| object(c, "CleartextDevice")),
        backing: object(block, "CryptoBackingDevice"),
        ignore: flag(block, "HintIgnore"),
        system: flag(block, "HintSystem"),
        auto: flag(block, "HintAuto"),
    }
}

/// Every drive and block device in `objects`. A cleartext volume takes
/// the drive and the hints of the container it was unlocked from: UDisks2
/// gives it no drive, so it would otherwise read as the system's.
pub fn fold(objects: &Objects) -> Snapshot {
    let mut disks: Vec<Disk> = objects
        .iter()
        .filter_map(|(p, ifaces)| ifaces.get(DRIVE_IFACE).map(|d| disk_of(p.as_str(), d)))
        .collect();
    disks.sort_by(|a, b| a.path.cmp(&b.path));
    let mut volumes: Vec<Volume> = objects
        .iter()
        .filter_map(|(p, ifaces)| {
            ifaces
                .get(BLOCK_IFACE)
                .map(|b| volume_of(p.as_str(), ifaces, b))
        })
        .collect();
    let containers: HashMap<String, Volume> = volumes
        .iter()
        .filter(|v| v.encrypted)
        .map(|v| (v.path.clone(), v.clone()))
        .collect();
    for v in &mut volumes {
        if let Some(c) = v.backing.as_ref().and_then(|b| containers.get(b)) {
            v.disk = v.disk.take().or_else(|| c.disk.clone());
            v.ignore = c.ignore;
            v.system = c.system;
            v.auto = c.auto;
        }
    }
    volumes.sort_by(|a, b| a.device.cmp(&b.device));
    Snapshot { disks, volumes }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Something still has a file open on the volume.
    Busy,
    /// Polkit said no.
    NotAuthorized,
    /// The user dismissed the prompt.
    Cancelled,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub fn classify(name: &str, message: &str) -> Error {
    let kind = match name.strip_prefix("org.freedesktop.UDisks2.Error.") {
        Some("DeviceBusy") => ErrorKind::Busy,
        Some("Cancelled" | "NotAuthorizedDismissed") => ErrorKind::Cancelled,
        Some(n) if n.starts_with("NotAuthorized") => ErrorKind::NotAuthorized,
        _ => ErrorKind::Other,
    };
    Error {
        kind,
        message: message.trim().to_string(),
    }
}

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Error {
        match &e {
            zbus::Error::MethodError(name, detail, _) => {
                classify(name.as_str(), detail.as_deref().unwrap_or(""))
            }
            other => Error {
                kind: ErrorKind::Other,
                message: other.to_string(),
            },
        }
    }
}

/// The empty `a{sv}` every call takes: UDisks2 picks the mount point and
/// options itself.
fn no_options() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

#[derive(Clone)]
pub struct Udisks {
    conn: zbus::blocking::Connection,
}

impl Udisks {
    /// Connect to the system bus. None without one.
    pub fn system() -> Option<Udisks> {
        let conn = zbus::blocking::connection::Builder::system()
            .and_then(|b| b.method_timeout(CALL_TIMEOUT).build());
        match conn {
            Ok(conn) => Some(Udisks { conn }),
            Err(e) => {
                log::info!("no system bus ({e}); no drives from UDisks2");
                None
            }
        }
    }

    pub fn on(conn: zbus::blocking::Connection) -> Udisks {
        Udisks { conn }
    }

    /// The system bus connection, for a caller's own calls beside these.
    pub fn connection(&self) -> &zbus::blocking::Connection {
        &self.conn
    }

    /// Every drive and volume. Empty when UDisks2 cannot be reached.
    pub fn snapshot(&self) -> Snapshot {
        let reply = self.conn.call_method(
            Some(NAME),
            ROOT,
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        );
        match reply.and_then(|m| m.body().deserialize::<Objects>()) {
            Ok(objects) => fold(&objects),
            Err(e) => {
                log::debug!("no UDisks2 objects ({e})");
                Snapshot::default()
            }
        }
    }

    fn call<B>(
        &self,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> Result<zbus::Message, Error>
    where
        B: serde::ser::Serialize + zbus::zvariant::DynamicType,
    {
        let path = ObjectPath::try_from(path).map_err(zbus::Error::from)?;
        Ok(self
            .conn
            .call_method(Some(NAME), &path, Some(iface), method, body)?)
    }

    /// Mount the file system on `volume`; where it went.
    pub fn mount(&self, volume: &str) -> Result<PathBuf, Error> {
        let reply = self.call(volume, FILESYSTEM_IFACE, "Mount", &(no_options(),))?;
        let at: String = reply.body().deserialize()?;
        Ok(PathBuf::from(at))
    }

    pub fn unmount(&self, volume: &str) -> Result<(), Error> {
        self.call(volume, FILESYSTEM_IFACE, "Unmount", &(no_options(),))
            .map(|_| ())
    }

    /// Unlock the encrypted `volume`; its cleartext volume.
    pub fn unlock(&self, volume: &str, passphrase: &str) -> Result<String, Error> {
        let reply = self.call(
            volume,
            ENCRYPTED_IFACE,
            "Unlock",
            &(passphrase, no_options()),
        )?;
        let o: OwnedObjectPath = reply.body().deserialize()?;
        Ok(o.as_str().to_string())
    }

    pub fn lock(&self, volume: &str) -> Result<(), Error> {
        self.call(volume, ENCRYPTED_IFACE, "Lock", &(no_options(),))
            .map(|_| ())
    }

    /// Make drive `disk` safe to pull out: unmount every file system on
    /// it, cleartext ones first, lock every unlocked container, then power
    /// the drive off (or eject its media). Once everything is unmounted
    /// the data is on the drive, so a drive that refuses the power-off is
    /// still safe and only logged.
    pub fn safely_remove(&self, snap: &Snapshot, disk: &str) -> Result<(), Error> {
        let on = snap.volumes_on(disk);
        let (clear, plain): (Vec<&&Volume>, Vec<&&Volume>) =
            on.iter().partition(|v| v.backing.is_some());
        for v in clear.iter().chain(plain.iter()) {
            if !v.mounts.is_empty() {
                self.unmount(&v.path)?;
            }
        }
        for v in &on {
            if v.cleartext.is_some() {
                self.lock(&v.path)?;
            }
        }
        let Some(d) = snap.disk(disk) else {
            return Ok(());
        };
        let off = if d.can_power_off {
            self.call(disk, DRIVE_IFACE, "PowerOff", &(no_options(),))
        } else if d.ejectable {
            self.call(disk, DRIVE_IFACE, "Eject", &(no_options(),))
        } else {
            return Ok(());
        };
        if let Err(e) = off {
            log::info!("{disk} stayed powered after its volumes came off: {e}");
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn props(pairs: Vec<(&str, OwnedValue)>) -> Props {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn s(v: &str) -> OwnedValue {
        OwnedValue::try_from(Value::from(v)).unwrap()
    }

    fn o(v: &str) -> OwnedValue {
        OwnedValue::try_from(Value::from(ObjectPath::try_from(v).unwrap())).unwrap()
    }

    fn ay(v: &str) -> OwnedValue {
        let mut b = v.as_bytes().to_vec();
        b.push(0);
        OwnedValue::try_from(Value::from(b)).unwrap()
    }

    fn aay(vs: &[&str]) -> OwnedValue {
        let list: Vec<Vec<u8>> = vs
            .iter()
            .map(|v| {
                let mut b = v.as_bytes().to_vec();
                b.push(0);
                b
            })
            .collect();
        OwnedValue::try_from(Value::from(list)).unwrap()
    }

    fn b(v: bool) -> OwnedValue {
        OwnedValue::from(v)
    }

    fn drive(vendor: &str, model: &str, removable: bool, bus: &str) -> HashMap<String, Props> {
        HashMap::from([(
            DRIVE_IFACE.to_string(),
            props(vec![
                ("Vendor", s(vendor)),
                ("Model", s(model)),
                ("Removable", b(removable)),
                ("MediaRemovable", b(removable)),
                ("Ejectable", b(removable)),
                ("CanPowerOff", b(removable)),
                ("Optical", b(false)),
                ("ConnectionBus", s(bus)),
            ]),
        )])
    }

    fn block(dev: &str, drive: &str, label: &str, id_type: &str, system: bool) -> Props {
        props(vec![
            ("Device", ay(dev)),
            ("Drive", o(drive)),
            ("IdLabel", s(label)),
            ("IdType", s(id_type)),
            ("HintIgnore", b(false)),
            ("HintSystem", b(system)),
            ("HintAuto", b(!system)),
            ("CryptoBackingDevice", o("/")),
            ("Size", OwnedValue::from(64u64 << 20)),
        ])
    }

    const BD: &str = "/org/freedesktop/UDisks2/block_devices";
    const DR: &str = "/org/freedesktop/UDisks2/drives";

    /// A partitioned USB stick with a mounted FAT partition, a LUKS
    /// stick unlocked to a cleartext ext4, and the internal disk.
    pub(crate) fn objects() -> Objects {
        let path = |p: &str| OwnedObjectPath::try_from(p.to_string()).unwrap();
        let mut m = Objects::new();
        m.insert(
            path(&format!("{DR}/Stick")),
            drive("Kingston", "DataTraveler", true, "usb"),
        );
        m.insert(path(&format!("{DR}/Vault")), drive("", "", true, "usb"));
        m.insert(
            path(&format!("{DR}/Ssd")),
            drive("ATA", "Samsung SSD", false, ""),
        );
        m.insert(
            path(&format!("{BD}/sdb")),
            HashMap::from([(
                BLOCK_IFACE.to_string(),
                block("/dev/sdb", &format!("{DR}/Stick"), "", "", false),
            )]),
        );
        m.insert(
            path(&format!("{BD}/sdb1")),
            HashMap::from([
                (
                    BLOCK_IFACE.to_string(),
                    block(
                        "/dev/sdb1",
                        &format!("{DR}/Stick"),
                        "KINGSTON",
                        "vfat",
                        false,
                    ),
                ),
                (
                    FILESYSTEM_IFACE.to_string(),
                    props(vec![("MountPoints", aay(&["/run/media/u/KINGSTON"]))]),
                ),
            ]),
        );
        m.insert(
            path(&format!("{BD}/sdc")),
            HashMap::from([
                (
                    BLOCK_IFACE.to_string(),
                    block("/dev/sdc", &format!("{DR}/Vault"), "", "crypto_LUKS", false),
                ),
                (
                    ENCRYPTED_IFACE.to_string(),
                    props(vec![("CleartextDevice", o(&format!("{BD}/dm_2d0")))]),
                ),
            ]),
        );
        let mut clear = block("/dev/dm-0", "/", "SECRET", "ext4", true);
        clear.insert("CryptoBackingDevice".into(), o(&format!("{BD}/sdc")));
        clear.insert("PreferredDevice".into(), ay("/dev/mapper/luks-1"));
        m.insert(
            path(&format!("{BD}/dm_2d0")),
            HashMap::from([
                (BLOCK_IFACE.to_string(), clear),
                (
                    FILESYSTEM_IFACE.to_string(),
                    props(vec![("MountPoints", aay(&[]))]),
                ),
            ]),
        );
        m.insert(
            path(&format!("{BD}/sda2")),
            HashMap::from([
                (
                    BLOCK_IFACE.to_string(),
                    block("/dev/sda2", &format!("{DR}/Ssd"), "", "ext4", true),
                ),
                (
                    FILESYSTEM_IFACE.to_string(),
                    props(vec![("MountPoints", aay(&[]))]),
                ),
            ]),
        );
        m
    }

    #[test]
    fn fold_reads_drives_volumes_mounts_and_cleartext() {
        let snap = fold(&objects());
        assert_eq!(snap.disks.len(), 3);
        let fat = snap.volume_by_device("/dev/sdb1").unwrap();
        assert!(fat.filesystem && !fat.encrypted);
        assert_eq!(fat.label.as_deref(), Some("KINGSTON"));
        assert_eq!(fat.mounts, [PathBuf::from("/run/media/u/KINGSTON")]);
        assert_eq!(snap.disk_of(fat).unwrap().name(), "Kingston DataTraveler");
        let luks = snap.volume_by_device("/dev/sdc").unwrap();
        assert!(luks.encrypted && !luks.filesystem);
        assert_eq!(luks.cleartext.as_deref(), Some(&*format!("{BD}/dm_2d0")));
        let clear = snap.volume_by_device("/dev/dm-0").unwrap();
        assert_eq!(
            clear.disk.as_deref(),
            Some(&*format!("{DR}/Vault")),
            "a cleartext volume takes its container's drive"
        );
        assert!(
            !clear.system && clear.auto,
            "and its container's hints, not the system hint of a volume with no drive"
        );
        assert_eq!(
            snap.disk_of(clear).unwrap().name(),
            "USB Mass Storage Device"
        );
    }

    #[test]
    fn unmounted_lists_only_removable_file_systems_not_mounted() {
        let snap = fold(&objects());
        let names: Vec<&str> = snap.unmounted().iter().map(|v| v.device.as_str()).collect();
        assert_eq!(
            names,
            ["/dev/dm-0"],
            "the mounted FAT, the container, the partition table and the internal disk stay out"
        );
        assert_eq!(snap.volumes_on(&format!("{DR}/Vault")).len(), 2);
    }

    #[test]
    fn a_locked_container_is_listed_as_unmounted() {
        let path = |p: &str| OwnedObjectPath::try_from(p.to_string()).unwrap();
        let mut m = objects();
        m.remove(&path(&format!("{BD}/dm_2d0")));
        m.get_mut(&path(&format!("{BD}/sdc"))).unwrap().insert(
            ENCRYPTED_IFACE.to_string(),
            props(vec![("CleartextDevice", o("/"))]),
        );
        let snap = fold(&m);
        let names: Vec<&str> = snap.unmounted().iter().map(|v| v.device.as_str()).collect();
        assert_eq!(names, ["/dev/sdc"]);
        assert!(snap.unmounted()[0].locked());
    }

    #[test]
    fn a_mounted_device_is_found_by_either_of_its_names() {
        let snap = fold(&objects());
        assert!(snap.removable_device("/dev/mapper/luks-1"));
        assert!(snap.removable_device("/dev/dm-0"));
        assert!(snap.removable_device("/dev/sdb1"));
        assert!(!snap.removable_device("/dev/sda2"), "the internal disk");
    }

    #[test]
    fn udisks_errors_are_classified_by_name() {
        let e = classify(
            "org.freedesktop.UDisks2.Error.DeviceBusy",
            "Error unmounting /dev/sdb1: target is busy",
        );
        assert_eq!(e.kind, ErrorKind::Busy);
        assert_eq!(e.message, "Error unmounting /dev/sdb1: target is busy");
        assert_eq!(
            classify("org.freedesktop.UDisks2.Error.NotAuthorizedDismissed", "").kind,
            ErrorKind::Cancelled
        );
        assert_eq!(
            classify("org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain", "").kind,
            ErrorKind::NotAuthorized
        );
        assert_eq!(
            classify("org.freedesktop.UDisks2.Error.Failed", "x").kind,
            ErrorKind::Other
        );
    }

    #[test]
    fn a_volume_without_a_label_is_named_by_its_drive() {
        let v = Volume::default();
        assert_eq!(v.name(None), "Removable Disk");
        let cd = Disk {
            optical: true,
            ..Disk::default()
        };
        assert_eq!(v.name(Some(&cd)), "CD Drive");
    }
}
