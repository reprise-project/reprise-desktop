//! `org.freedesktop.impl.portal.Screenshot`: one frame of the first output via
//! ext-image-copy-capture into a PNG, returned as a `file://` URI. PickColor
//! needs no prompt; the click is the consent.

use crate::wayland::{Target, Wayland};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

pub struct Screenshot {
    pub frontend: crate::Frontend,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    async fn screenshot(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        self.frontend.check(&hdr)?;
        log::info!("Screenshot for {app_id:?}");
        if needs_consent(&options) {
            let cancel = Arc::new(AtomicBool::new(false));
            crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
            let id = app_id.clone();
            let parent = crate::dialog::parent_handle(&parent_window);
            let ok = blocking::unblock(move || {
                crate::permit::ask(
                    crate::permit::Kind::Screenshot,
                    &id,
                    Vec::new(),
                    false,
                    parent,
                    cancel,
                )
                .allowed()
            })
            .await;
            let _ = server.remove::<crate::Request, _>(handle.clone()).await;
            if !ok {
                log::info!("Screenshot for {app_id:?} denied");
                return Ok((crate::RESPONSE_CANCELLED, HashMap::new()));
            }
        }
        match blocking::unblock(take).await {
            Ok(path) => {
                let mut r = HashMap::new();
                let uri = format!("file://{}", path.display());
                r.insert(
                    "uri".to_string(),
                    Value::from(uri).try_to_owned().expect("string"),
                );
                Ok((crate::RESPONSE_OK, r))
            }
            Err(e) => {
                log::warn!("Screenshot: {e}");
                Ok((crate::RESPONSE_OTHER, HashMap::new()))
            }
        }
    }

    async fn pick_color(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        _options: HashMap<String, Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        self.frontend.check(&hdr)?;
        log::info!("PickColor for {app_id:?}");
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
        let color = blocking::unblock(move || crate::colorpick::pick(&cancel)).await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        let Some(color) = color else {
            return Ok((crate::RESPONSE_CANCELLED, HashMap::new()));
        };
        let mut r = HashMap::new();
        r.insert(
            "color".to_string(),
            Value::from(color).try_to_owned().expect("struct"),
        );
        Ok((crate::RESPONSE_OK, r))
    }
}

/// False only when the frontend already settled the `screenshot`
/// permission: a non-interactive call with `permission_store_checked`.
fn needs_consent(options: &HashMap<String, Value<'_>>) -> bool {
    let flag = |key| matches!(options.get(key), Some(Value::Bool(true)));
    flag("interactive") || !flag("permission_store_checked")
}

/// Capture the first output and write it out; blocking.
fn take() -> Result<std::path::PathBuf, String> {
    let mut wl = Wayland::connect()?;
    let out = wl.outputs().into_iter().next().ok_or("no output")?;
    let session = wl.open_session(&Target::Output(out.name.clone()), true)?;
    let c = wl.constraints();
    let format = c.pick_format().ok_or("no 32 bpp shm format")?;
    let buf = wl.shm_buffer(c.width, c.height, format)?;
    // An output always has a first frame to give; two seconds is for a
    // compositor busy with something else.
    wl.capture(&session, &buf, std::time::Duration::from_secs(2))
        .map_err(|e| format!("capture failed: {e:?}"))?;
    let pixels = buf.read()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (file, path) =
        reprisefs::ops::create_new_numbered(&target_dir()?, &format!("Screenshot {now}.png"))
            .map_err(|e| e.to_string())?;
    if let Err(e) = write_png(file, c.width, c.height, &pixels, format) {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

/// Pictures when it exists, else the runtime dir, which only this user can
/// write. Never a shared directory such as `/tmp`.
fn target_dir() -> Result<std::path::PathBuf, String> {
    if let Some(pictures) = reprisefs::places::user_dir("PICTURES", "Pictures")
        && pictures.is_dir()
    {
        return Ok(pictures);
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "no Pictures folder and no XDG_RUNTIME_DIR".to_string())
}

fn write_png(
    file: std::fs::File,
    w: u32,
    h: u32,
    pixels: &[u8],
    format: wayland_client::protocol::wl_shm::Format,
) -> Result<(), String> {
    use wayland_client::protocol::wl_shm::Format;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let (words, _) = pixels.as_chunks::<4>();
    for px in words.iter().take((w * h) as usize) {
        let (r, g, b) = match format {
            Format::Argb8888 | Format::Xrgb8888 => (px[2], px[1], px[0]),
            _ => (px[0], px[1], px[2]),
        };
        rgb.extend_from_slice(&[r, g, b]);
    }
    writer.write_image_data(&rgb).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, bool)]) -> HashMap<String, Value<'static>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::from(*v)))
            .collect()
    }

    #[test]
    fn only_a_store_checked_non_interactive_call_skips_the_prompt() {
        assert!(needs_consent(&opts(&[])));
        assert!(needs_consent(&opts(&[("interactive", true)])));
        assert!(needs_consent(&opts(&[("permission_store_checked", false)])));
        assert!(needs_consent(&opts(&[
            ("interactive", true),
            ("permission_store_checked", true)
        ])));
        assert!(!needs_consent(&opts(&[("permission_store_checked", true)])));
        assert!(!needs_consent(&opts(&[
            ("interactive", false),
            ("permission_store_checked", true)
        ])));
    }
}
