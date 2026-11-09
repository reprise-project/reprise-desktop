//! Drives and volumes as UDisks2 reports them, and the calls that mount,
//! unlock and safely remove them. [`fold`] reads the map
//! `GetManagedObjects` returns without a bus, so it is testable; [`Udisks`]
//! is a blocking client over the system bus for callers without an
//! executor of their own; [`follow`] posts a fresh [`Snapshot`] after each
//! change. Polkit decides every call; the session's agent prompts when it has to. A disk image
//! is set up read-only on a loop device ([`Udisks::mount_image`]) and stands in for a drive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use zbus::{ObjectPath, OwnedObjectPath, OwnedVariant, Variant};

pub const NAME: &str = "org.freedesktop.UDisks2";
pub const ROOT: &str = "/org/freedesktop/UDisks2";
pub const BLOCK_IFACE: &str = "org.freedesktop.UDisks2.Block";
pub const FILESYSTEM_IFACE: &str = "org.freedesktop.UDisks2.Filesystem";
pub const ENCRYPTED_IFACE: &str = "org.freedesktop.UDisks2.Encrypted";
pub const DRIVE_IFACE: &str = "org.freedesktop.UDisks2.Drive";
pub const LOOP_IFACE: &str = "org.freedesktop.UDisks2.Loop";
pub const PARTITION_IFACE: &str = "org.freedesktop.UDisks2.Partition";
pub const MANAGER: &str = "/org/freedesktop/UDisks2/Manager";
pub const MANAGER_IFACE: &str = "org.freedesktop.UDisks2.Manager";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

/// How long a call waits: a mount or an unlock can hold on a polkit
/// prompt the user is still reading.
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// How long a new loop device's file systems take to be probed and announced.
const PROBE_WAIT: Duration = Duration::from_secs(5);
const PROBE_POLL: Duration = Duration::from_millis(100);

/// The types Explorer mounts as disk images, under their current and their older names.
pub const IMAGE_MIMES: [&str; 5] = [
    "application/vnd.efi.iso",
    "application/x-cd-image",
    "application/x-iso9660-image",
    "application/vnd.efi.img",
    "application/x-raw-disk-image",
];

pub fn is_disk_image(mime: &str) -> bool {
    IMAGE_MIMES.contains(&mime)
}

/// The caption of the question before a disk image is mounted.
pub const MOUNT_TITLE: &str = "Mount Disk Image";

/// The question's buttons, Mount first and the default.
pub const MOUNT_BUTTONS: [&str; 2] = ["&Mount", "Cancel"];

/// The question's text for the disk image `name`.
pub fn mount_text(name: &str) -> String {
    format!(
        "\"{name}\" is a disk image. Mounting it has this computer read the file system \
         inside, which an image from someone you do not trust can use to harm it. It mounts \
         read-only.\n\nDo you want to mount it?"
    )
}

/// One interface's properties as `GetManagedObjects` returns them.
pub type Props = HashMap<String, OwnedVariant>;
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
    /// The disk image this user set up the volume's loop device on, the volume itself or a
    /// partition of it.
    pub image: Option<Image>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Image {
    pub file: PathBuf,
    /// The loop device's block object.
    pub loop_path: String,
}

impl Volume {
    /// An encrypted container with no cleartext volume.
    pub fn locked(&self) -> bool {
        self.encrypted && self.cleartext.is_none()
    }

    pub fn name(&self, disk: Option<&Disk>) -> String {
        let image = self.image.as_ref().and_then(|i| i.file.file_name());
        match &self.label {
            Some(l) => l.clone(),
            None if image.is_some() => image.unwrap_or_default().to_string_lossy().into_owned(),
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

    /// Whether `v` is the user's to mount and remove: shown, and on a disk image the user set up
    /// or not the system's on a removable drive.
    pub fn removable(&self, v: &Volume) -> bool {
        !v.ignore
            && (v.image.is_some() || (!v.system && self.disk_of(v).is_some_and(|d| d.removable)))
    }

    /// The loop device this user set up on `file`, if there is one.
    pub fn image_of(&self, file: &Path) -> Option<&str> {
        self.volumes
            .iter()
            .filter_map(|v| v.image.as_ref())
            .find(|i| i.file == file)
            .map(|i| i.loop_path.as_str())
    }

    /// The file systems on loop device `loop_path`: its own, or else each partition's.
    pub fn image_volumes(&self, loop_path: &str) -> Vec<&Volume> {
        self.volumes
            .iter()
            .filter(|v| v.filesystem && !v.ignore)
            .filter(|v| v.image.as_ref().is_some_and(|i| i.loop_path == loop_path))
            .collect()
    }

    /// A drive for every mounted disk image volume, which `/proc/mounts` alone leaves out with
    /// every other loop device.
    pub fn image_drives(&self) -> Vec<crate::places::Drive> {
        self.volumes
            .iter()
            .filter(|v| v.image.is_some() && !v.ignore)
            .filter_map(|v| {
                Some(crate::places::Drive {
                    mount: v.mounts.first()?.clone(),
                    device: v.device.clone(),
                    fs: v.fs_type.clone(),
                    label: Some(v.name(None)),
                    removable: true,
                    optical: matches!(v.fs_type.as_str(), "iso9660" | "udf"),
                    total: v.size,
                    free: 0,
                })
            })
            .collect()
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

/// The most of a drive's label shown, in characters: NTFS's longest.
const LABEL_MAX: usize = 32;

fn volume_of(path: &str, ifaces: &HashMap<String, Props>, block: &Props) -> Volume {
    let fs = ifaces.get(FILESYSTEM_IFACE);
    let crypt = ifaces.get(ENCRYPTED_IFACE);
    let shown = |key| Some(crate::names::shown_line(&text(block, key), LABEL_MAX));
    let label = shown("IdLabel")
        .filter(|l| !l.is_empty())
        .or_else(|| shown("HintName").filter(|l| !l.is_empty()));
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
        image: None,
    }
}

/// The image file behind a loop device this user set up.
fn own_loop(lp: &Props, uid: u32) -> Option<PathBuf> {
    let by = lp
        .get("SetupByUID")
        .and_then(|v| u32::try_from(v.clone()).ok());
    let file = dev_name(lp, "BackingFile");
    (by == Some(uid) && !file.is_empty()).then(|| PathBuf::from(file))
}

/// Every drive and block device in `objects`. A cleartext volume takes
/// the drive and the hints of the container it was unlocked from: UDisks2
/// gives it no drive, so it would otherwise read as the system's.
///
/// A loop device `uid` set up carries its [`Image`], and so do its partitions; when the image's
/// whole device holds a file system (a hybrid ISO), its partitions are ignored.
pub fn fold(objects: &Objects) -> Snapshot {
    // SAFETY: getuid cannot fail and touches no memory.
    fold_for(objects, unsafe { libc::getuid() })
}

fn fold_for(objects: &Objects, uid: u32) -> Snapshot {
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
    let images: HashMap<String, (PathBuf, bool)> = objects
        .iter()
        .filter_map(|(p, ifaces)| {
            let file = own_loop(ifaces.get(LOOP_IFACE)?, uid)?;
            Some((p.to_string(), (file, ifaces.contains_key(FILESYSTEM_IFACE))))
        })
        .collect();
    let tables: HashMap<String, String> = objects
        .iter()
        .filter_map(|(p, ifaces)| {
            Some((
                p.to_string(),
                object(ifaces.get(PARTITION_IFACE)?, "Table")?,
            ))
        })
        .collect();
    for v in &mut volumes {
        if let Some(c) = v.backing.as_ref().and_then(|b| containers.get(b)) {
            v.disk = v.disk.take().or_else(|| c.disk.clone());
            v.ignore = c.ignore;
            v.system = c.system;
            v.auto = c.auto;
        }
        let table = tables.get(&v.path).cloned();
        let whole = table.as_deref().unwrap_or(&v.path);
        if let Some((file, whole_fs)) = images.get(whole) {
            v.image = Some(Image {
                file: file.clone(),
                loop_path: whole.to_string(),
            });
            v.ignore |= table.is_some() && *whole_fs;
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
        let timed_out = reprisebus::polkit::refusal(&e, |_| false, || false)
            == Some(reprisebus::polkit::Refusal::TimedOut);
        if timed_out {
            return Error {
                kind: ErrorKind::Other,
                message: reprisebus::polkit::TIMED_OUT.into(),
            };
        }
        match reprisebus::error::name(&e) {
            Some(name) => classify(&name, reprisebus::error::message(&e).unwrap_or("")),
            None => Error {
                kind: ErrorKind::Other,
                message: e.to_string(),
            },
        }
    }
}

fn io_error(e: std::io::Error) -> Error {
    Error {
        kind: ErrorKind::Other,
        message: e.to_string(),
    }
}

/// The empty `a{sv}` every call takes: UDisks2 picks the mount point and
/// options itself.
fn no_options() -> HashMap<&'static str, Variant<'static>> {
    HashMap::new()
}

/// The signals that mean a drive, volume or mount changed.
const SIGNALS: &[&str] = &["InterfacesAdded", "InterfacesRemoved", "PropertiesChanged"];

/// A mount point appears a moment after the signal that announced the
/// device; a burst of signals costs one read after this pause.
const SETTLE: Duration = Duration::from_millis(150);

/// Follow UDisks2 on a thread of its own: `post` gets a whole [`Snapshot`]
/// at once and again after every burst of changes. Without a system bus it
/// gets one empty snapshot.
pub fn follow(post: impl FnMut(Snapshot) + Send + 'static) {
    if let Err(e) = std::thread::Builder::new()
        .name("udisks".into())
        .spawn(move || zbus::block_on(run(post)))
    {
        log::warn!("could not start the UDisks2 thread: {e}");
    }
}

async fn run(mut post: impl FnMut(Snapshot)) {
    use futures_util::FutureExt;
    use futures_util::stream::StreamExt;

    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::info!("no system bus ({e}); no drives to follow");
            post(Snapshot::default());
            return;
        }
    };
    let mut rules = Vec::new();
    for member in SIGNALS {
        match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .member(*member)
            .build()
        {
            Ok(r) => rules.push(r.into()),
            Err(e) => log::warn!("no match rule for {member}: {e}"),
        }
    }
    // Subscribed before the first read, so no change slips between.
    let mut signals = reprisebus::signals(&conn, NAME, rules).await;
    loop {
        post(read(&conn).await);
        if signals.next().await.is_none() {
            log::warn!("the UDisks2 signal streams all ended");
            return;
        }
        async_io::Timer::after(SETTLE).await;
        while let Some(Some(_)) = signals.next().now_or_never() {}
    }
}

async fn read(conn: &zbus::Connection) -> Snapshot {
    let reply = conn
        .call_method(
            Some(NAME),
            ROOT,
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        )
        .await;
    match reply.and_then(|m| m.body().deserialize::<Objects>()) {
        Ok(objects) => fold(&objects),
        Err(e) => {
            log::info!("no UDisks2 objects ({e})");
            Snapshot::default()
        }
    }
}

#[derive(Clone)]
pub struct Udisks {
    conn: zbus::Connection,
}

impl Udisks {
    /// Connect to the system bus. None without one.
    pub fn system() -> Option<Udisks> {
        let conn =
            reprisebus::connect(zbus::connection::Builder::system().method_timeout(CALL_TIMEOUT));
        match conn {
            Ok(conn) => Some(Udisks { conn }),
            Err(e) => {
                log::info!("no system bus ({e}); no drives from UDisks2");
                None
            }
        }
    }

    pub fn on(conn: zbus::Connection) -> Udisks {
        Udisks { conn }
    }

    /// The system bus connection, for a caller's own calls beside these.
    pub fn connection(&self) -> &zbus::Connection {
        &self.conn
    }

    /// Every drive and volume. Empty when UDisks2 cannot be reached.
    pub fn snapshot(&self) -> Snapshot {
        let reply = reprisebus::wait(self.conn.call_method(
            Some(NAME),
            ROOT,
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        ));
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
        B: serde::ser::Serialize + zbus::DynamicType,
    {
        let path = ObjectPath::try_from(path)?;
        Ok(reprisebus::wait(self.conn.call_method(
            Some(NAME),
            &path,
            Some(iface),
            method,
            body,
        ))?)
    }

    /// Mount the file system on `volume`; where it went.
    pub fn mount(&self, volume: &str) -> Result<PathBuf, Error> {
        let reply = self.call(volume, FILESYSTEM_IFACE, "Mount", &(no_options(),))?;
        let at: String = reply.body().deserialize()?;
        Ok(PathBuf::from(at))
    }

    /// [`Udisks::mount`] read-only.
    fn mount_read_only(&self, volume: &str) -> Result<PathBuf, Error> {
        let options = HashMap::from([("options", Variant::from("ro"))]);
        let reply = self.call(volume, FILESYSTEM_IFACE, "Mount", &(options,))?;
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

    /// Set disk image `file` up read-only on a loop device and mount its file system read-only,
    /// or each partition's when the whole image holds none; where the first went. An image already set up
    /// by this user is reused. With nothing mountable on it the loop device is removed again.
    pub fn mount_image(&self, file: &Path) -> Result<PathBuf, Error> {
        let file = std::fs::canonicalize(file).map_err(io_error)?;
        let loop_path = match self.snapshot().image_of(&file) {
            Some(l) => l.to_string(),
            None => self.loop_setup(&file)?,
        };
        let volumes = self.probed(&loop_path);
        let mut first: Option<Result<PathBuf, Error>> = None;
        for v in &volumes {
            let at = match v.mounts.first() {
                Some(m) => Ok(m.clone()),
                None => self.mount_read_only(&v.path),
            };
            if let Err(e) = &at {
                log::info!("mounting {} of {}: {e}", v.device, file.display());
            }
            first = match first {
                Some(Ok(at)) => Some(Ok(at)),
                _ => Some(at),
            };
        }
        let failed = match first {
            Some(Ok(at)) => return Ok(at),
            Some(Err(e)) => e,
            None => Error {
                kind: ErrorKind::Other,
                message: format!(
                    "{} holds no file system that can be opened.",
                    file.file_name().unwrap_or_default().to_string_lossy()
                ),
            },
        };
        if let Err(e) = self.delete_loop(&loop_path) {
            log::info!("{loop_path} stayed set up: {e}");
        }
        Err(failed)
    }

    fn loop_setup(&self, file: &Path) -> Result<String, Error> {
        let f = std::fs::File::open(file).map_err(io_error)?;
        let options = HashMap::from([("read-only", Variant::from(true))]);
        let reply = self.call(
            MANAGER,
            MANAGER_IFACE,
            "LoopSetup",
            &(zbus::Fd::from(&f), options),
        )?;
        let o: OwnedObjectPath = reply.body().deserialize()?;
        Ok(o.as_str().to_string())
    }

    /// The file systems on a new loop device once probed: UDisks2 announces its partitions and
    /// their file systems a moment after the setup returns.
    fn probed(&self, loop_path: &str) -> Vec<Volume> {
        let end = std::time::Instant::now() + PROBE_WAIT;
        loop {
            let found = !self.snapshot().image_volumes(loop_path).is_empty();
            if found || std::time::Instant::now() >= end {
                if found {
                    std::thread::sleep(SETTLE);
                }
                let snap = self.snapshot();
                return snap.image_volumes(loop_path).into_iter().cloned().collect();
            }
            std::thread::sleep(PROBE_POLL);
        }
    }

    fn delete_loop(&self, loop_path: &str) -> Result<(), Error> {
        self.call(loop_path, LOOP_IFACE, "Delete", &(no_options(),))
            .map(|_| ())
    }

    /// Eject a disk image: unmount every file system on loop device `loop_path`, partitions
    /// first, then remove the loop device.
    pub fn eject_image(&self, snap: &Snapshot, loop_path: &str) -> Result<(), Error> {
        let mut on: Vec<&Volume> = snap
            .volumes
            .iter()
            .filter(|v| v.image.as_ref().is_some_and(|i| i.loop_path == loop_path))
            .filter(|v| !v.mounts.is_empty())
            .collect();
        on.sort_by_key(|v| v.path == loop_path);
        for v in on {
            self.unmount(&v.path)?;
        }
        self.delete_loop(loop_path)
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

    fn props(pairs: Vec<(&str, OwnedVariant)>) -> Props {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn s(v: &str) -> OwnedVariant {
        OwnedVariant::try_from(Variant::from(v)).unwrap()
    }

    fn o(v: &str) -> OwnedVariant {
        OwnedVariant::try_from(Variant::from(ObjectPath::try_from(v).unwrap())).unwrap()
    }

    fn ay(v: &str) -> OwnedVariant {
        let mut b = v.as_bytes().to_vec();
        b.push(0);
        OwnedVariant::try_from(Variant::from(b)).unwrap()
    }

    fn aay(vs: &[&str]) -> OwnedVariant {
        let list: Vec<Vec<u8>> = vs
            .iter()
            .map(|v| {
                let mut b = v.as_bytes().to_vec();
                b.push(0);
                b
            })
            .collect();
        OwnedVariant::try_from(Variant::from(list)).unwrap()
    }

    fn b(v: bool) -> OwnedVariant {
        OwnedVariant::from(v)
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
            ("Size", OwnedVariant::from(64u64 << 20)),
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
    fn a_reply_named_in_the_dbus_namespace_reads_as_its_message() {
        let e = Error::from(reprisebus::error::reply(
            "org.freedesktop.DBus.Error.UnknownObject",
            "No such interface on object",
        ));
        assert_eq!(e.kind, ErrorKind::Other);
        assert_eq!(e.message, "No such interface on object");
    }

    #[test]
    fn a_prompt_left_open_past_the_call_timeout_says_to_try_again() {
        let io = zbus::Error::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out",
        ));
        assert_eq!(Error::from(io).message, reprisebus::polkit::TIMED_OUT);
        let other = zbus::Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert_ne!(Error::from(other).message, reprisebus::polkit::TIMED_OUT);
    }

    fn u(v: u32) -> OwnedVariant {
        OwnedVariant::from(v)
    }

    /// Loop device `name` set up on `file` by `uid`, with an iso9660 file system when `fs`.
    fn loop_dev(name: &str, file: &str, uid: u32, fs: bool) -> HashMap<String, Props> {
        let mut ifaces = HashMap::from([
            (
                BLOCK_IFACE.to_string(),
                block(
                    &format!("/dev/{name}"),
                    "/",
                    "",
                    if fs { "iso9660" } else { "" },
                    true,
                ),
            ),
            (
                LOOP_IFACE.to_string(),
                props(vec![("BackingFile", ay(file)), ("SetupByUID", u(uid))]),
            ),
        ]);
        if fs {
            let block = ifaces.get_mut(BLOCK_IFACE).unwrap();
            block.insert("IdLabel".into(), s("DEBIAN_13"));
            ifaces.insert(
                FILESYSTEM_IFACE.to_string(),
                props(vec![("MountPoints", aay(&[]))]),
            );
        }
        ifaces
    }

    fn partition(
        name: &str,
        table: &str,
        fs_type: &str,
        mounted: &[&str],
    ) -> HashMap<String, Props> {
        HashMap::from([
            (
                BLOCK_IFACE.to_string(),
                block(&format!("/dev/{name}"), "/", "", fs_type, true),
            ),
            (
                PARTITION_IFACE.to_string(),
                props(vec![("Table", o(&format!("{BD}/{table}")))]),
            ),
            (
                FILESYSTEM_IFACE.to_string(),
                props(vec![("MountPoints", aay(mounted))]),
            ),
        ])
    }

    fn uid() -> u32 {
        // SAFETY: getuid cannot fail and touches no memory.
        unsafe { libc::getuid() }
    }

    #[test]
    fn a_loop_device_this_user_set_up_is_a_removable_image_and_anyone_elses_is_not() {
        let path = |p: &str| OwnedObjectPath::try_from(p.to_string()).unwrap();
        let mut m = Objects::new();
        m.insert(
            path(&format!("{BD}/loop0")),
            loop_dev("loop0", "/home/u/debian.iso", uid(), true),
        );
        m.insert(
            path(&format!("{BD}/loop1")),
            loop_dev("loop1", "/var/lib/snapd/snaps/core.snap", uid() + 1, true),
        );
        let snap = fold(&m);
        let iso = snap.volume_by_device("/dev/loop0").unwrap();
        assert_eq!(
            iso.image,
            Some(Image {
                file: "/home/u/debian.iso".into(),
                loop_path: format!("{BD}/loop0"),
            })
        );
        assert!(snap.removable(iso), "though UDisks2 calls it the system's");
        assert_eq!(iso.name(None), "DEBIAN_13");
        let other = snap.volume_by_device("/dev/loop1").unwrap();
        assert_eq!(other.image, None);
        assert!(!snap.removable(other));
        assert_eq!(
            snap.image_of(Path::new("/home/u/debian.iso")),
            Some(&*format!("{BD}/loop0"))
        );
        assert_eq!(
            snap.image_of(Path::new("/var/lib/snapd/snaps/core.snap")),
            None
        );
        let names: Vec<&str> = snap.unmounted().iter().map(|v| v.device.as_str()).collect();
        assert_eq!(
            names,
            ["/dev/loop0"],
            "an image not mounted yet is listed like a stick"
        );
    }

    #[test]
    fn a_partitioned_images_partitions_carry_it_and_a_hybrid_isos_are_ignored() {
        let path = |p: &str| OwnedObjectPath::try_from(p.to_string()).unwrap();
        let mut m = Objects::new();
        m.insert(
            path(&format!("{BD}/loop0")),
            loop_dev("loop0", "/home/u/disk.img", uid(), false),
        );
        m.insert(
            path(&format!("{BD}/loop0p1")),
            partition("loop0p1", "loop0", "vfat", &["/run/media/u/BOOT"]),
        );
        m.insert(
            path(&format!("{BD}/loop0p2")),
            partition("loop0p2", "loop0", "ext4", &[]),
        );
        m.insert(
            path(&format!("{BD}/loop1")),
            loop_dev("loop1", "/home/u/hybrid.iso", uid(), true),
        );
        m.insert(
            path(&format!("{BD}/loop1p1")),
            partition("loop1p1", "loop1", "iso9660", &[]),
        );
        let snap = fold(&m);
        let parts: Vec<&str> = snap
            .image_volumes(&format!("{BD}/loop0"))
            .iter()
            .map(|v| v.device.as_str())
            .collect();
        assert_eq!(parts, ["/dev/loop0p1", "/dev/loop0p2"]);
        assert_eq!(
            snap.volume_by_device("/dev/loop0p2").unwrap().name(None),
            "disk.img",
            "an unlabelled partition is named after its image"
        );
        let hybrid: Vec<&str> = snap
            .image_volumes(&format!("{BD}/loop1"))
            .iter()
            .map(|v| v.device.as_str())
            .collect();
        assert_eq!(hybrid, ["/dev/loop1"], "the whole ISO, not its partition");
        let drives = snap.image_drives();
        assert_eq!(drives.len(), 1, "only what is mounted");
        assert_eq!(drives[0].mount, PathBuf::from("/run/media/u/BOOT"));
        assert_eq!(drives[0].device, "/dev/loop0p1");
        assert!(drives[0].removable && !drives[0].optical);
    }

    #[test]
    fn a_drive_label_loses_control_and_bidi_characters_and_its_tail_past_32() {
        let path = |p: &str| OwnedObjectPath::try_from(p.to_string()).unwrap();
        let mut m = Objects::new();
        for (dev, label) in [
            ("sdc1", "\u{202e}gpj.exe\tHome\u{7}"),
            ("sdd1", "\u{2066}\n\u{200f}"),
            ("sde1", &"x".repeat(40)),
        ] {
            let mut p = partition(dev, "sdc", "vfat", &[]);
            let block = p.get_mut(BLOCK_IFACE).unwrap();
            block.insert("IdLabel".into(), s(label));
            m.insert(path(&format!("{BD}/{dev}")), p);
        }
        let snap = fold(&m);
        let name = |d: &str| snap.volume_by_device(d).unwrap().name(None);
        assert_eq!(name("/dev/sdc1"), "gpj.exeHome");
        assert_eq!(name("/dev/sdd1"), "Removable Disk", "nothing left to show");
        assert_eq!(name("/dev/sde1"), "x".repeat(32));
    }

    #[test]
    fn disk_images_are_told_by_their_current_and_older_types() {
        assert!(is_disk_image("application/vnd.efi.iso"));
        assert!(is_disk_image("application/x-cd-image"));
        assert!(is_disk_image("application/x-raw-disk-image"));
        assert!(!is_disk_image("application/zip"));
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

    /// A fake UDisks2 on a private bus: the calls `mount_image` and `eject_image` make, in order.
    mod bus {
        use super::*;
        use std::io::{BufRead, BufReader};
        use std::process::{Child, Command, Stdio};
        use std::sync::{Arc, Mutex};

        struct Bus(Child, String);

        impl Drop for Bus {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        fn bus() -> Option<Bus> {
            let mut child = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let mut line = String::new();
            BufReader::new(child.stdout.take()?)
                .read_line(&mut line)
                .ok()?;
            Some(Bus(child, line.trim().to_string()))
        }

        #[derive(Clone, Copy, PartialEq)]
        enum Image {
            Iso,
            Partitioned,
            Blank,
        }

        struct State {
            image: Image,
            file: String,
            set_up: bool,
            mounted: Vec<String>,
            calls: Vec<String>,
        }

        type Shared = Arc<Mutex<State>>;

        fn mounts_of(st: &State, name: &str) -> OwnedVariant {
            let at = format!("/run/media/u/{name}");
            let list: Vec<&str> = match st.mounted.iter().any(|m| m == name) {
                true => vec![at.as_str()],
                false => vec![],
            };
            aay(&list)
        }

        fn objects(st: &State) -> Objects {
            let path = |p: &str| OwnedObjectPath::try_from(p.to_string()).unwrap();
            let mut m = Objects::new();
            if !st.set_up {
                return m;
            }
            let mut whole = loop_dev("loop0", &st.file, uid(), st.image == Image::Iso);
            if let Some(fs) = whole.get_mut(FILESYSTEM_IFACE) {
                fs.insert("MountPoints".into(), mounts_of(st, "loop0"));
            }
            m.insert(path(&format!("{BD}/loop0")), whole);
            if st.image == Image::Partitioned {
                for p in ["loop0p1", "loop0p2"] {
                    let mut part = partition(p, "loop0", "vfat", &[]);
                    let fs = part.get_mut(FILESYSTEM_IFACE).unwrap();
                    fs.insert("MountPoints".into(), mounts_of(st, p));
                    m.insert(path(&format!("{BD}/{p}")), part);
                }
            }
            m
        }

        struct ObjectManager(Shared);

        #[zbus::interface(name = "org.freedesktop.DBus.ObjectManager")]
        impl ObjectManager {
            fn get_managed_objects(&self) -> Objects {
                objects(&self.0.lock().unwrap())
            }
        }

        struct Manager(Shared);

        #[zbus::interface(name = "org.freedesktop.UDisks2.Manager")]
        impl Manager {
            fn loop_setup(
                &self,
                _fd: zbus::OwnedFd,
                options: HashMap<String, OwnedVariant>,
            ) -> OwnedObjectPath {
                let ro = options
                    .get("read-only")
                    .and_then(|v| bool::try_from(v.clone()).ok());
                let mut st = self.0.lock().unwrap();
                st.calls.push(format!("LoopSetup read-only={ro:?}"));
                st.set_up = true;
                OwnedObjectPath::try_from(format!("{BD}/loop0")).unwrap()
            }
        }

        struct Filesystem(Shared, &'static str);

        #[zbus::interface(name = "org.freedesktop.UDisks2.Filesystem")]
        impl Filesystem {
            fn mount(&self, options: HashMap<String, OwnedVariant>) -> String {
                let how = options
                    .get("options")
                    .and_then(|v| String::try_from(v.clone()).ok());
                let mut st = self.0.lock().unwrap();
                st.calls.push(format!("Mount {} options={how:?}", self.1));
                st.mounted.push(self.1.to_string());
                format!("/run/media/u/{}", self.1)
            }

            fn unmount(&self, _options: HashMap<String, OwnedVariant>) {
                let mut st = self.0.lock().unwrap();
                st.calls.push(format!("Unmount {}", self.1));
                st.mounted.retain(|m| m != self.1);
            }
        }

        struct Loop(Shared);

        #[zbus::interface(name = "org.freedesktop.UDisks2.Loop")]
        impl Loop {
            fn delete(&self, _options: HashMap<String, OwnedVariant>) {
                let mut st = self.0.lock().unwrap();
                st.calls.push("Delete loop0".into());
                st.set_up = false;
                st.mounted.clear();
            }
        }

        struct Fake {
            udisks: Udisks,
            state: Shared,
            file: PathBuf,
            _service: zbus::Connection,
            _dir: tempfile::TempDir,
            _bus: Bus,
        }

        fn fake(image: Image, set_up: bool) -> Option<Fake> {
            let Some(bus) = bus() else {
                eprintln!("no dbus-daemon; skipped");
                return None;
            };
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("disk.img");
            std::fs::write(&file, b"image").unwrap();
            let file = std::fs::canonicalize(&file).unwrap();
            let state = Arc::new(Mutex::new(State {
                image,
                file: file.to_string_lossy().into_owned(),
                set_up,
                mounted: Vec::new(),
                calls: Vec::new(),
            }));
            let service = zbus::block_on(async {
                let mut b = zbus::connection::Builder::address(bus.1.as_str())
                    .name(NAME)
                    .serve_at(ROOT, ObjectManager(state.clone()))
                    .serve_at(MANAGER, Manager(state.clone()))
                    .serve_at(format!("{BD}/loop0"), Loop(state.clone()));
                for name in ["loop0", "loop0p1", "loop0p2"] {
                    b = b.serve_at(format!("{BD}/{name}"), Filesystem(state.clone(), name));
                }
                b.build().await.expect("serve")
            });
            let client = zbus::block_on(async {
                zbus::connection::Builder::address(bus.1.as_str())
                    .build()
                    .await
                    .expect("connect")
            });
            Some(Fake {
                udisks: Udisks::on(client),
                state,
                file,
                _service: service,
                _dir: dir,
                _bus: bus,
            })
        }

        fn calls(f: &Fake) -> Vec<String> {
            f.state.lock().unwrap().calls.clone()
        }

        #[test]
        fn an_iso_is_set_up_read_only_and_its_file_system_mounted() {
            let Some(f) = fake(Image::Iso, false) else {
                return;
            };
            let at = f.udisks.mount_image(&f.file).expect("mounted");
            assert_eq!(at, PathBuf::from("/run/media/u/loop0"));
            assert_eq!(
                calls(&f),
                [
                    "LoopSetup read-only=Some(true)",
                    "Mount loop0 options=Some(\"ro\")"
                ]
            );
        }

        #[test]
        fn a_partitioned_image_mounts_each_partition_and_goes_to_the_first() {
            let Some(f) = fake(Image::Partitioned, false) else {
                return;
            };
            let at = f.udisks.mount_image(&f.file).expect("mounted");
            assert_eq!(at, PathBuf::from("/run/media/u/loop0p1"));
            assert_eq!(
                calls(&f),
                [
                    "LoopSetup read-only=Some(true)",
                    "Mount loop0p1 options=Some(\"ro\")",
                    "Mount loop0p2 options=Some(\"ro\")"
                ]
            );
        }

        #[test]
        fn an_image_already_set_up_gets_no_second_loop_device() {
            let Some(f) = fake(Image::Iso, true) else {
                return;
            };
            f.udisks.mount_image(&f.file).expect("mounted");
            assert_eq!(calls(&f), ["Mount loop0 options=Some(\"ro\")"]);
        }

        #[test]
        fn an_image_with_no_file_system_is_refused_and_its_loop_device_removed() {
            let Some(f) = fake(Image::Blank, false) else {
                return;
            };
            let e = f.udisks.mount_image(&f.file).expect_err("nothing to mount");
            assert_eq!(
                e.message,
                "disk.img holds no file system that can be opened."
            );
            assert_eq!(
                calls(&f),
                ["LoopSetup read-only=Some(true)", "Delete loop0"]
            );
        }

        #[test]
        fn eject_unmounts_the_partitions_then_removes_the_loop_device() {
            let Some(f) = fake(Image::Partitioned, false) else {
                return;
            };
            f.udisks.mount_image(&f.file).expect("mounted");
            f.state.lock().unwrap().calls.clear();
            let snap = f.udisks.snapshot();
            f.udisks
                .eject_image(&snap, &format!("{BD}/loop0"))
                .expect("ejected");
            assert_eq!(
                calls(&f),
                ["Unmount loop0p1", "Unmount loop0p2", "Delete loop0"]
            );
        }
    }
}
