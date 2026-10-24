//! Drag and drop through the data device: drags this program starts and drops it takes.

use crate::clipboard::read_capped;
use crate::draw::copy_pixels;
use crate::sizing::MAX_SIDE;
use crate::{
    Accept, Actions, DragAction, DragOut, DropKind, Dropped, Event, Host, Offered, URI_LIST, WinId,
    dnd,
};
use reprisekit::raster::Canvas;
use smithay_client_toolkit::{
    data_device_manager::{
        WritePipe,
        data_device::DataDeviceHandler,
        data_offer::{DataOfferHandler, DragOffer},
        data_source::{DataSourceHandler, DragSource},
    },
    reexports::calloop::timer::{TimeoutAction, Timer},
    shm::{
        Shm,
        slot::{Buffer, SlotPool},
    },
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use wayland_client::{
    Connection, Proxy, QueueHandle,
    protocol::{
        wl_data_device::WlDataDevice, wl_data_device_manager::DndAction,
        wl_data_offer::WlDataOffer, wl_data_source::WlDataSource, wl_shm, wl_surface,
    },
};

pub(crate) struct Outgoing {
    win: WinId,
    paths: Vec<PathBuf>,
    action: Option<DragAction>,
    dropped: bool,
    transfers: reprisebus::filetransfer::Transfers,
    pub(crate) icon: Option<DragIcon>,
    source: DragSource,
}

pub(crate) struct DragIcon {
    surface: wl_surface::WlSurface,
    buffer: Buffer,
    _pool: SlotPool,
    pub(crate) scale: i32,
    grab: (i32, i32),
}

impl Drop for DragIcon {
    fn drop(&mut self) {
        self.surface.destroy();
    }
}

/// An shm buffer holding `image`'s device pixels.
fn icon_buffer(image: &Canvas, shm: &Shm) -> Option<(Buffer, SlotPool)> {
    let (dw, dh) = image.device_size();
    if dw == 0 || dh == 0 || dw > MAX_SIDE as u32 || dh > MAX_SIDE as u32 {
        return None;
    }
    let (dw, dh) = (dw as i32, dh as i32);
    let mut pool = SlotPool::new((dw * dh * 4) as usize, shm).ok()?;
    let (buffer, pixels) = pool
        .create_buffer(dw, dh, dw * 4, wl_shm::Format::Argb8888)
        .ok()?;
    copy_pixels(pixels, &image.pixels);
    Some((buffer, pool))
}

impl DragIcon {
    /// Shows a buffer drawn at another scale; the grab, in logical px, stays where it was.
    fn redraw(&self) {
        self.surface.set_buffer_scale(self.scale);
        if self.buffer.attach_to(&self.surface).is_err() {
            return;
        }
        self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        self.surface.commit();
    }

    /// Runs after `start_drag`, so the compositor's icon sees the offset commit.
    fn show(&self) {
        let (gx, gy) = self.grab;
        self.surface.set_buffer_scale(self.scale);
        if self.surface.version() >= 5 {
            if self.buffer.attach_to(&self.surface).is_err() {
                return;
            }
            self.surface.offset(-gx, -gy);
        } else {
            self.surface.attach(Some(self.buffer.wl_buffer()), -gx, -gy);
        }
        self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        self.surface.commit();
    }
}

pub(crate) struct Asked {
    pub(crate) win: WinId,
    pub(crate) offer: WlDataOffer,
    source: Actions,
}

/// So the dragging program never waits on a menu nobody answers.
const ASK_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) struct Incoming {
    win: WinId,
    /// Held raw: SCTK's `DragOffer` refuses requests once the drag has
    /// left, and the compositor sends `leave` right after `drop`.
    offer: WlDataOffer,
    /// The enter's serial, which every `accept` names.
    serial: u32,
    mimes: Vec<String>,
    pos: (i32, i32),
    /// What the dragging program allows.
    source: Actions,
    action: Option<DragAction>,
    accept: Option<(Accept, &'static str)>,
    /// Whether the files read ahead of the drop were there; None until
    /// that read lands. It picks the mime a `DropKind::Uris` drop is read
    /// in (`dnd::drop_mime`).
    files: Option<bool>,
    /// Its files come through the document portal (`dnd::through_portal`):
    /// a sandboxed program's drag. They are never moved.
    portal: bool,
}

impl Host {
    pub(crate) fn start_drag(&mut self, win: WinId, out: DragOut) {
        if let Some(old) = self.drag_out.take() {
            self.drag_ended(old, false);
        }
        let (Some(manager), Some(device)) = (&self.data_device_manager, &self.data_device) else {
            return;
        };
        let Some(origin) = self
            .slots
            .iter()
            .find(|s| s.id == win)
            .map(|s| s.window.wl_surface().clone())
        else {
            return;
        };
        let transfers = reprisebus::filetransfer::Transfers::for_drag();
        let mimes = dnd::source_mimes(transfers.offered());
        let source = manager.create_drag_and_drop_source(&self.qh, mimes, DndAction::all());
        let icon = self.drag_icon(&out.image, out.grab);
        source.start_drag(
            device,
            &origin,
            icon.as_ref().map(|i| &i.surface),
            self.press_serial,
        );
        if let Some(i) = &icon {
            i.show();
        }
        self.drag_out = Some(Outgoing {
            win,
            paths: out.paths,
            action: None,
            dropped: false,
            transfers,
            icon,
            source,
        });
    }

    fn drag_icon(&self, image: &Canvas, grab: (i32, i32)) -> Option<DragIcon> {
        let (buffer, pool) = icon_buffer(image, &self.shm)?;
        Some(DragIcon {
            surface: self.compositor.create_surface(&self.qh),
            buffer,
            _pool: pool,
            scale: image.scale(),
            grab,
        })
    }

    /// The compositor shows drag icon `surface` at `factor` now: the program draws its image
    /// again at that scale. False when `surface` is no drag icon.
    pub(crate) fn rescale_drag_icon(
        &mut self,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) -> bool {
        let Some(out) = &self.drag_out else {
            return false;
        };
        let Some(icon) = out.icon.as_ref().filter(|i| &i.surface == surface) else {
            return false;
        };
        let scale = factor.max(1);
        if icon.scale != scale {
            self.draw_drag_icon(scale);
        }
        true
    }

    /// The running drag's icon shows the program's image drawn again at `scale`.
    pub(crate) fn draw_drag_icon(&mut self, scale: i32) {
        let Some(win) = self.drag_out.as_ref().map(|o| o.win) else {
            return;
        };
        self.ctx.icons.set_scale(scale);
        let image = self.app.drag_image(win, scale, &mut self.ctx);
        if let Some(image) = image.filter(|i| i.scale() == scale)
            && let Some((buffer, pool)) = icon_buffer(&image, &self.shm)
            && let Some(icon) = self.drag_out.as_mut().and_then(|o| o.icon.as_mut())
        {
            // The old buffer goes once the new one is committed.
            let _old = (
                std::mem::replace(&mut icon.buffer, buffer),
                std::mem::replace(&mut icon._pool, pool),
            );
            icon.scale = scale;
            icon.redraw();
        }
    }

    fn drag_ended(&mut self, out: Outgoing, finished: bool) {
        let dropped = out.dropped;
        let action = if finished { out.action } else { None };
        let win = out.win;
        drop(out);
        self.send(win, Event::DragEnd { dropped, action });
    }

    fn drag_of(&mut self, source: &WlDataSource) -> Option<&mut Outgoing> {
        self.drag_out
            .as_mut()
            .filter(|o| o.source.inner() == source)
    }

    /// The program answered a drag over window `win`. An answer that
    /// changes nothing is not resent: `set_actions` makes the compositor
    /// pick the action again.
    pub(crate) fn drag_accept(&mut self, win: WinId, accept: Option<Accept>) {
        let Some(inc) = self.drag_in.as_mut().filter(|i| i.win == win) else {
            return;
        };
        let files = inc.files == Some(true);
        let answer = accept.and_then(|a| {
            let mime = dnd::drop_mime(&inc.mimes, a.kind, files, inc.portal)?;
            let a = Accept {
                actions: a.actions.read_as(mime == dnd::FILE_TRANSFER),
                ..a
            };
            (a.actions != Actions::default()).then_some((a, mime))
        });
        if answer == inc.accept {
            return;
        }
        inc.accept = answer;
        let (mime, (actions, preferred)) = match answer {
            Some((a, mime)) => (Some(mime.to_string()), a.wl_actions()),
            None => (None, (DndAction::empty(), DndAction::empty())),
        };
        inc.offer.accept(inc.serial, mime);
        inc.offer.set_actions(actions, preferred);
    }

    /// The data of a drop on `win` arrived (None when the read failed):
    /// hand it over, then finish the drop, unless it asks and waits for
    /// `Reply::answer_drop`. A drop whose offer is gone, whose read
    /// failed, or that the window cannot take (closed, or behind a modal
    /// child) is cancelled, and the window hears `DragLeave` instead.
    fn dropped(
        &mut self,
        win: WinId,
        offer: WlDataOffer,
        (x, y): (i32, i32),
        source: Actions,
        data: Option<Dropped>,
        action: DragAction,
    ) {
        let ev = data
            .filter(|_| offer.is_alive())
            .map(|data| Event::DragDrop {
                x,
                y,
                data,
                action,
                source,
            });
        let Some(ev) = ev.filter(|ev| self.slot_of_id(win) && !self.gated(win, ev)) else {
            offer.destroy();
            self.send(win, Event::DragLeave);
            return;
        };
        if action == DragAction::Ask {
            let asked = Asked {
                win,
                offer: offer.clone(),
                source,
            };
            if let Some(old) = self.asked_drop.replace(asked) {
                self.cancel_asked(old);
            }
            let timed = offer.clone();
            let _ = self.loop_handle.insert_source(
                Timer::from_duration(ASK_TIMEOUT),
                move |_, _, host| {
                    if host.asked_drop.as_ref().is_some_and(|a| a.offer == timed)
                        && let Some(a) = host.asked_drop.take()
                    {
                        host.cancel_asked(a);
                    }
                    TimeoutAction::Drop
                },
            );
        }
        self.send(win, ev);
        if action != DragAction::Ask {
            offer.finish();
            offer.destroy();
        }
    }

    /// Settle the `Ask` drop with the user's choice, or cancel it: an
    /// offer destroyed unfinished cancels the drag at its source. A
    /// choice the source does not allow would leave the compositor no
    /// action to finish with (a protocol error), so it cancels too, and
    /// so does an offer already gone; the window hears `DropCancelled`.
    pub(crate) fn answer_drop(&mut self, action: Option<DragAction>) {
        let Some(asked) = self.asked_drop.take() else {
            return;
        };
        match action {
            Some(a @ (DragAction::Copy | DragAction::Move))
                if asked.source.has(a) && asked.offer.is_alive() =>
            {
                asked.offer.set_actions(a.to_wl(), a.to_wl());
                asked.offer.finish();
                asked.offer.destroy();
            }
            None | Some(DragAction::Ask) => asked.offer.destroy(),
            Some(_) => self.cancel_asked(asked),
        }
    }

    fn cancel_asked(&mut self, asked: Asked) {
        asked.offer.destroy();
        self.send(asked.win, Event::DropCancelled);
    }

    fn peek_files(&mut self, offer: WlDataOffer, mime: &'static str) {
        let pipe = match smithay_client_toolkit::data_device_manager::data_offer::receive(
            &offer,
            mime.to_string(),
        ) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("drag: no pipe for {mime}: {e}");
                return;
            }
        };
        read_capped(
            &self.loop_handle,
            pipe,
            reprisefs::pipe::PASTE_TIMEOUT,
            move |host: &mut Host, buf| {
                let Some(buf) = buf else {
                    return;
                };
                if mime == dnd::FILE_TRANSFER {
                    host.retrieve(buf.bytes(), move |host, paths| match paths {
                        Some(paths) => host.peeked(&offer, paths),
                        // No files through the portal (a source that sends
                        // its key only at the drop): the URI list.
                        None => {
                            let uris = host.drag_in.as_ref().is_some_and(|i| {
                                i.offer == offer && i.mimes.iter().any(|m| m == URI_LIST)
                            });
                            if uris {
                                host.peek_files(offer, URI_LIST);
                            }
                        }
                    });
                } else {
                    host.peeked(
                        &offer,
                        reprisefs::fileclip::decode(URI_LIST, buf.bytes()).paths,
                    );
                }
            },
        );
    }

    fn peeked(&mut self, offer: &WlDataOffer, paths: Vec<PathBuf>) {
        let Some(inc) = self.drag_in.as_mut().filter(|i| &i.offer == offer) else {
            return;
        };
        inc.files = Some(!paths.is_empty());
        let win = inc.win;
        self.send(win, Event::DragFiles { paths });
    }

    fn retrieve(
        &self,
        bytes: &[u8],
        done: impl FnOnce(&mut Host, Option<Vec<PathBuf>>) + Send + 'static,
    ) {
        let tx = self.portal_done.clone();
        reprisebus::filetransfer::retrieve_key(bytes, move |paths| {
            let _ = tx.send(Box::new(move |host| done(host, paths)));
        });
    }

    fn read_drop(&mut self, d: Landing, mime: &'static str) {
        let pipe = match smithay_client_toolkit::data_device_manager::data_offer::receive(
            &d.offer,
            mime.to_string(),
        ) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("drop: no pipe for {mime}: {e}");
                d.offer.destroy();
                self.send(d.win, Event::DragLeave);
                return;
            }
        };
        read_capped(
            &self.loop_handle,
            pipe,
            reprisefs::pipe::PASTE_TIMEOUT,
            move |host: &mut Host, buf| match buf {
                Some(b) if mime == dnd::FILE_TRANSFER => {
                    host.retrieve(b.bytes(), move |host, paths| match paths {
                        Some(p) => {
                            let data = dnd::portal_drop(p, d.kind);
                            host.land(d, Some(data));
                        }
                        None if d.uris && d.offer.is_alive() => host.read_drop(d, URI_LIST),
                        None => host.land(d, None),
                    });
                }
                buf => {
                    let data = buf.map(|b| dnd::decode_drop(b.bytes(), d.kind, mime));
                    host.land(d, data);
                }
            },
        );
    }

    fn land(&mut self, d: Landing, data: Option<Dropped>) {
        self.dropped(d.win, d.offer, d.pos, d.source, data, d.action);
    }
}

impl DataDeviceHandler for Host {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        x: f64,
        y: f64,
        surface: &wl_surface::WlSurface,
    ) {
        let Some(offer) = self
            .data_device
            .as_ref()
            .and_then(|d| d.data().drag_offer())
        else {
            return;
        };
        offer.accept_mime_type(offer.serial, None);
        // A v1 or v2 offer has no actions and no finish: the host takes
        // only drops it can settle, so such a drag is refused.
        if offer.inner().version() < 3 {
            return;
        }
        let Some(win) = self.slot_of(surface) else {
            return;
        };
        let mimes = offer.with_mime_types(|m| m.to_vec());
        let offered = Offered::from_mimes(&mimes);
        let action = DragAction::from_wl(offer.selected_action);
        let source = Actions::from_wl(offer.source_actions);
        let (x, y) = (x as i32, y as i32);
        // One dropped elsewhere and awaiting its finish is over.
        let own = self.drag_out.as_ref().is_some_and(|o| !o.dropped);
        let portal = dnd::through_portal(&mimes, own);
        let source = source.read_as(portal);
        let peek = dnd::peek_mime(&mimes, portal);
        self.drag_in = Some(Incoming {
            win,
            offer: offer.inner().clone(),
            serial: offer.serial,
            mimes,
            pos: (x, y),
            source,
            action,
            accept: None,
            files: None,
            portal,
        });
        self.send(
            win,
            Event::DragEnter {
                x,
                y,
                offered,
                source,
                action,
                own,
            },
        );
        if let Some(mime) = peek.filter(|_| !own) {
            self.peek_files(offer.inner().clone(), mime);
        }
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        if let Some(inc) = self.drag_in.take() {
            self.send(inc.win, Event::DragLeave);
        }
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, x: f64, y: f64) {
        let Some(inc) = self.drag_in.as_mut() else {
            return;
        };
        let (x, y) = (x as i32, y as i32);
        inc.pos = (x, y);
        let (win, action) = (inc.win, inc.action);
        self.send(win, Event::DragMotion { x, y, action });
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        let Some(inc) = self.drag_in.take() else {
            return;
        };
        let (Some((accept, mime)), Some(action)) = (inc.accept, inc.action) else {
            inc.offer.destroy();
            self.send(inc.win, Event::DragLeave);
            return;
        };
        let landing = Landing {
            win: inc.win,
            uris: inc.mimes.iter().any(|m| m == URI_LIST),
            offer: inc.offer,
            pos: inc.pos,
            source: inc.source,
            kind: accept.kind,
            action,
        };
        self.read_drop(landing, mime);
    }
}

struct Landing {
    win: WinId,
    offer: WlDataOffer,
    pos: (i32, i32),
    source: Actions,
    kind: DropKind,
    action: DragAction,
    uris: bool,
}

impl DataOfferHandler for Host {
    /// The dragging program changed what it allows. SCTK holds the
    /// offer's lock here, so only the raw offer is touched.
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        offer: &mut DragOffer,
        actions: DndAction,
    ) {
        if let Some(inc) = self.drag_in.as_mut().filter(|i| &i.offer == offer.inner()) {
            inc.source = Actions::from_wl(actions).read_as(inc.portal);
        }
    }

    /// The compositor picked another action for the drag over us. SCTK
    /// holds the offer's lock here, so only the raw offer is touched.
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        offer: &mut DragOffer,
        action: DndAction,
    ) {
        let Some(inc) = self.drag_in.as_mut().filter(|i| &i.offer == offer.inner()) else {
            return;
        };
        let action = DragAction::from_wl(action);
        if inc.action == action {
            return;
        }
        inc.action = action;
        let (win, (x, y)) = (inc.win, inc.pos);
        self.send(win, Event::DragMotion { x, y, action });
    }
}

impl DataSourceHandler for Host {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        fd: WritePipe,
    ) {
        let clipboard = self
            .copy_source
            .as_ref()
            .is_some_and(|s| s.inner() == source);
        // A drag source that is no longer ours sends nothing, never the clipboard.
        let bytes: Arc<[u8]> = if clipboard {
            if let Some(offers) = &self.copy_clip {
                match offers.iter().find(|(m, _)| *m == mime) {
                    Some((_, bytes)) => bytes.as_slice().into(),
                    None => return,
                }
            } else {
                self.copy_text.clone()
            }
        } else if let Some(out) = self.drag_of(source) {
            // Each request is its own transfer: a target may read the key
            // while the drag passes over it and again at the drop.
            if mime == dnd::FILE_TRANSFER {
                let fd: std::os::fd::OwnedFd = fd.into();
                out.transfers.send_key(out.paths.clone(), move |key| {
                    reprisefs::pipe::write_detached(fd, key.into())
                });
                return;
            }
            match dnd::encode_files(&out.paths, &mime) {
                Some(b) => b.into(),
                None => return,
            }
        } else {
            return;
        };
        // The receiver owns the pipe; the write must not stall the loop.
        reprisefs::pipe::write_detached(fd.into(), bytes);
    }
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if self.drag_of(source).is_some() {
            if let Some(out) = self.drag_out.take() {
                self.drag_ended(out, false);
            }
        } else if self
            .copy_source
            .as_ref()
            .is_some_and(|s| s.inner() == source)
        {
            self.copy_source = None;
        }
    }
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if let Some(out) = self.drag_of(source) {
            out.dropped = true;
        }
    }
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        if self.drag_of(source).is_some()
            && let Some(out) = self.drag_out.take()
        {
            self.drag_ended(out, true);
        }
    }
    fn action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        action: DndAction,
    ) {
        if let Some(out) = self.drag_of(source) {
            out.action = DragAction::from_wl(action);
        }
    }
}
