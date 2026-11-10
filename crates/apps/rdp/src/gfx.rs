//! One connection's graphics pipeline channel (MS-RDPEGFX), which carries the H.264 frames: one
//! surface per shared screen, the size of its target and mapped at it.

use crate::layout::{Layout, Rect16};
use crate::stream::Rect;
use ironrdp_egfx::pdu::{Avc420Region, CapabilitiesAdvertisePdu, CapabilitySet};
use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer};
use ironrdp_server::{
    EgfxServerMessage, GfxContext, GfxDvcBridge, GfxServerFactory, GfxServerHandle, ServerEvent,
    ServerEventSender,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;

/// Frames per surface sent and not yet acknowledged before the display waits.
const MAX_IN_FLIGHT: u32 = 3;
/// How long after the channel is offered a resize waits for the client to open it: the
/// reactivation drops the client's channel replies, which leaves the connection on bitmaps.
pub const NEGOTIATION: Duration = Duration::from_secs(2);

/// Shared between the connection's server, which opens the channel, and its display, which
/// sends on it.
#[derive(Clone, Default)]
pub struct Gfx {
    link: Arc<Mutex<Link>>,
    /// Told when the client has opened the pipeline.
    ready: Arc<Notify>,
    /// Counts the client's capability advertisements: each one after the first drops every
    /// surface on both sides.
    advertised: Arc<AtomicU32>,
}

pub enum Prepared {
    /// The screen's surface takes the frame; a `new` one needs a whole picture. `avc444` when
    /// the client takes AVC444 too.
    Ready { new: bool, avc444: bool },
    /// The client is frames behind; try again later.
    Behind,
    /// The client takes no AVC420 (yet), or the screen has no place in the layout.
    Unavailable,
}

struct Link {
    server: Option<GfxServerHandle>,
    events: Option<UnboundedSender<ServerEvent>>,
    surfaces: Option<Surfaces>,
    start: Instant,
    /// When the channel was offered to the client.
    offered: Option<tokio::time::Instant>,
}

impl Default for Link {
    fn default() -> Link {
        Link {
            server: None,
            events: None,
            surfaces: None,
            start: Instant::now(),
            offered: None,
        }
    }
}

impl Gfx {
    /// What the connection's server opens the channel with.
    pub fn factory(&self) -> Box<dyn GfxServerFactory> {
        Box::new(Factory(self.clone()))
    }

    /// A new pipeline server for the connection, offered to the client now.
    pub fn open(&self) -> (GfxDvcBridge, GfxServerHandle) {
        let mut server = GraphicsPipelineServer::new(self.handler());
        server.set_max_frames_in_flight(MAX_IN_FLIGHT);
        let server = Arc::new(Mutex::new(server));
        let mut link = self.link.lock().unwrap();
        link.server = Some(server.clone());
        link.surfaces = None;
        link.offered = Some(tokio::time::Instant::now());
        (GfxDvcBridge::new(server.clone()), server)
    }

    /// The client is still opening the pipeline, at most [`NEGOTIATION`] after it was offered.
    pub fn negotiating(&self) -> bool {
        let link = self.link.lock().unwrap();
        match (&link.server, link.offered) {
            (Some(server), Some(at)) => {
                !server.lock().unwrap().is_ready() && at.elapsed() < NEGOTIATION
            }
            _ => false,
        }
    }

    /// Resolves once the pipeline is open or [`NEGOTIATION`] after it was offered.
    pub async fn negotiated(&self) {
        let Some(at) = self.link.lock().unwrap().offered else {
            return;
        };
        let ready = self.ready.notified();
        tokio::pin!(ready);
        // Registered before the check, so an opening between the two still wakes it.
        ready.as_mut().enable();
        if self.open_now() {
            return;
        }
        let _ = tokio::time::timeout_at(at + NEGOTIATION, ready).await;
    }

    fn handler(&self) -> Box<dyn GraphicsPipelineHandler> {
        Box::new(Handler {
            ready: self.ready.clone(),
            advertised: self.advertised.clone(),
        })
    }

    fn open_now(&self) -> bool {
        let server = self.link.lock().unwrap().server.clone();
        server.is_some_and(|s| s.lock().unwrap().is_ready())
    }

    /// Ready `screen`'s surface for an H.264 frame at its target in `layout`. A desktop or
    /// monitors other than the surfaces', or a new advertisement, reset the client's graphics
    /// first.
    pub fn prepare(&self, layout: &Layout, screen: usize) -> Prepared {
        let mut link = self.link.lock().unwrap();
        let Some(server) = link.server.clone() else {
            return Prepared::Unavailable;
        };
        let mut s = server.lock().unwrap();
        if !s.is_ready() || !s.supports_avc420() {
            return Prepared::Unavailable;
        }
        let Some(target) = layout.target(screen).filter(|t| t.w > 0 && t.h > 0) else {
            return Prepared::Unavailable;
        };
        let advertised = self.advertised.load(Ordering::Relaxed);
        let reset = (link.surfaces.as_ref()).is_none_or(|u| {
            (u.desktop, &u.monitors, u.advertised) != (layout.desktop, &layout.monitors, advertised)
        });
        if reset {
            let (w, h) = layout.desktop;
            s.resize_with_monitors(w, h, layout.monitors.clone());
            let surfaces = layout.placed.len().max(1) as u32;
            s.set_max_frames_in_flight(MAX_IN_FLIGHT * surfaces);
            link.surfaces = Some(Surfaces {
                desktop: layout.desktop,
                monitors: layout.monitors.clone(),
                advertised,
                made: Vec::new(),
            });
        }
        let made = &mut link.surfaces.as_mut().expect("set above").made;
        let at = match made.iter().position(|m| m.screen == screen) {
            Some(at) => at,
            None => {
                made.push(Surface { screen, made: None });
                made.len() - 1
            }
        };
        let new = match made[at].made {
            Some((_, was)) if was == target => false,
            old => {
                if let Some((id, _)) = old {
                    s.delete_surface(id);
                }
                made[at].made = s.create_surface(target.w, target.h).map(|id| (id, target));
                let Some((id, _)) = made[at].made else {
                    flush(&link, &mut s);
                    return Prepared::Unavailable;
                };
                s.map_surface_to_output(id, target.x.into(), target.y.into());
                true
            }
        };
        flush(&link, &mut s);
        let avc444 = s.supports_avc444();
        match new {
            true => Prepared::Ready { new, avc444 },
            false if s.should_backpressure() => Prepared::Behind,
            false => Prepared::Ready { new, avc444 },
        }
    }

    /// Send coded `video` that changes `regions` of `screen`'s surface; false if the channel
    /// would not take it.
    pub fn send(&self, screen: usize, video: Video<'_>, regions: &[Rect], qp: u8) -> bool {
        let link = self.link.lock().unwrap();
        let (Some(server), Some(surface_id)) = (link.server.clone(), link.surface(screen)) else {
            return false;
        };
        let mut s = server.lock().unwrap();
        if regions.is_empty() || s.channel_id().is_none() || link.events.is_none() {
            return false;
        }
        let region = [bounds(regions, qp)];
        let ms = link.start.elapsed().as_millis() as u32;
        let queued = match video {
            Video::Avc420(data) => s.try_send_avc420_frame(surface_id, data, &region, ms),
            Video::Avc444 { main, aux } => {
                let aux_region = aux.map(|_| &region[..]);
                s.try_send_avc444_frame(surface_id, main, &region, aux, aux_region, ms)
            }
        };
        queued.is_ok() && flush(&link, &mut s)
    }

    /// Delete `screen`'s surface, so the bitmap updates show there again.
    pub fn close(&self, screen: usize) {
        let mut link = self.link.lock().unwrap();
        let server = link.server.clone();
        let surface = (link.surfaces.as_mut())
            .and_then(|u| u.made.iter_mut().find(|m| m.screen == screen))
            .and_then(|m| m.made.take());
        let (Some(server), Some((id, _))) = (server, surface) else {
            return;
        };
        let mut s = server.lock().unwrap();
        s.delete_surface(id);
        flush(&link, &mut s);
    }
}

/// The client's graphics as last reset, and the surfaces made since.
struct Surfaces {
    desktop: (u16, u16),
    monitors: Vec<ironrdp_pdu::gcc::Monitor>,
    /// [`Gfx::advertised`] at the reset.
    advertised: u32,
    made: Vec<Surface>,
}

/// `screen`'s surface id and the target it was made for; None until made, and once closed.
struct Surface {
    screen: usize,
    made: Option<(u16, Rect16)>,
}

impl Link {
    fn surface(&self, screen: usize) -> Option<u16> {
        let surfaces = self.surfaces.as_ref()?;
        let surface = surfaces.made.iter().find(|m| m.screen == screen)?;
        surface.made.map(|(id, _)| id)
    }
}

/// H.264 for the surface: one AVC420 access unit, or a picture's AVC444 views.
#[derive(Clone, Copy)]
pub enum Video<'a> {
    Avc420(&'a [u8]),
    Avc444 {
        main: &'a [u8],
        aux: Option<&'a [u8]>,
    },
}

/// The one region a frame updates: the bounds of `regions`, as clients refuse overlapping
/// rectangles.
fn bounds(regions: &[Rect], qp: u8) -> Avc420Region {
    let edge = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    Avc420Region::new(
        edge(regions.iter().map(|r| r.x).min().unwrap_or(0)),
        edge(regions.iter().map(|r| r.y).min().unwrap_or(0)),
        edge(regions.iter().map(|r| r.x + r.width).max().unwrap_or(0)),
        edge(regions.iter().map(|r| r.y + r.height).max().unwrap_or(0)),
        qp,
        100,
    )
}

/// Hand what the pipeline queued to the connection's event loop; false if it was dropped.
fn flush(link: &Link, server: &mut GraphicsPipelineServer) -> bool {
    let messages = server.drain_output();
    if messages.is_empty() {
        return true;
    }
    let (Some(channel), Some(events)) = (server.channel_id(), &link.events) else {
        return false;
    };
    match ironrdp_dvc::encode_dvc_messages(
        channel,
        messages,
        ironrdp_svc::ChannelFlags::SHOW_PROTOCOL,
    ) {
        Ok(messages) => events
            .send(ServerEvent::Egfx(EgfxServerMessage::SendMessages {
                messages,
            }))
            .is_ok(),
        Err(e) => {
            log::warn!("graphics pipeline: {e}");
            false
        }
    }
}

struct Factory(Gfx);

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.0.link.lock().unwrap().events = Some(sender);
    }
}

impl GfxServerFactory for Factory {
    fn build_gfx_handler(&self, _: GfxContext) -> Box<dyn GraphicsPipelineHandler> {
        self.0.handler()
    }

    fn build_server_with_handle(&self, _: GfxContext) -> Option<(GfxDvcBridge, GfxServerHandle)> {
        Some(self.0.open())
    }
}

/// The bytes of a capability set IronRDP doesn't model that [`offered`] writes out.
const UNKNOWN_DATA_SHOWN: usize = 64;

/// The journal line naming every capability set in `pdu`: its version, its first four bytes as
/// flags, its length and, for a version IronRDP doesn't model, its bytes.
fn offered(pdu: &CapabilitiesAdvertisePdu) -> String {
    let sets: Vec<String> = (pdu.0.iter())
        .map(|set| {
            let flags = match set.data.first_chunk::<4>() {
                Some(flags) => format!("{:#010x}", u32::from_le_bytes(*flags)),
                None => "-".into(),
            };
            let mut line = format!(
                "{:#010x} flags {flags} len {}",
                u32::from(set.version),
                set.data.len()
            );
            if !set.version.is_known() {
                line.push_str(" data ");
                for byte in set.data.iter().take(UNKNOWN_DATA_SHOWN) {
                    line.push_str(&format!("{byte:02x}"));
                }
                if set.data.len() > UNKNOWN_DATA_SHOWN {
                    line.push_str("...");
                }
            }
            line
        })
        .collect();
    format!("client offers graphics capabilities: {}", sets.join("; "))
}

/// Logs the negotiation, counts advertisements and tells the display it is done. Runs with the
/// pipeline server locked, so it never takes [`Gfx::link`].
struct Handler {
    ready: Arc<Notify>,
    advertised: Arc<AtomicU32>,
}

impl GraphicsPipelineHandler for Handler {
    fn capabilities_advertise(&mut self, pdu: &CapabilitiesAdvertisePdu) {
        log::info!("{}", offered(pdu));
        self.advertised.fetch_add(1, Ordering::Relaxed);
    }

    fn on_ready(&mut self, negotiated: &CapabilitySet) {
        log::info!("graphics pipeline ready: {negotiated:?}");
        self.ready.notify_waiters();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ironrdp_dvc::DvcProcessor;
    use ironrdp_egfx::pdu::{
        CapabilitiesV81Flags, CapabilitiesV107Flags, CapabilityVersion, Codec1Type,
        FrameAcknowledgePdu, GfxPdu, QueueDepth, RawCapabilitySet, WireToSurface1Pdu,
        encode_avc420_bitmap_stream,
    };
    use ironrdp_pdu::gcc::{Monitor, MonitorFlags};
    use ironrdp_pdu::geometry::ExclusiveRectangle;
    use tokio::sync::mpsc::UnboundedReceiver;

    const AU: &[u8] = &[0, 0, 0, 1, 0x65];

    /// The layout of one `size` screen and no client monitors.
    fn one(size: (u32, u32)) -> Layout {
        Layout::new(
            &[crate::layout::ScreenRect {
                position: (0, 0),
                logical: size,
                pixels: size,
            }],
            &[],
        )
    }

    /// A pipeline the client opened with `caps`, and the events its PDUs go out on.
    fn opened_with(caps: CapabilitySet) -> (Gfx, GfxServerHandle, UnboundedReceiver<ServerEvent>) {
        let gfx = Gfx::default();
        let (events, events_rx) = tokio::sync::mpsc::unbounded_channel();
        gfx.factory().set_sender(events);
        let (_bridge, server) = gfx.open();
        server.lock().unwrap().start(0).unwrap();
        client_sends(
            &server,
            GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[caps])),
        );
        (gfx, server, events_rx)
    }

    fn opened() -> (Gfx, GfxServerHandle, UnboundedReceiver<ServerEvent>) {
        opened_with(CapabilitySet::V8_1 {
            flags: CapabilitiesV81Flags::AVC420_ENABLED,
        })
    }

    /// The client opens `server`'s pipeline with AVC420.
    pub(crate) fn client_opens(server: &GfxServerHandle) {
        client_sends(
            server,
            GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[
                CapabilitySet::V8_1 {
                    flags: CapabilitiesV81Flags::AVC420_ENABLED,
                },
            ])),
        );
    }

    #[tokio::test(start_paused = true)]
    async fn every_screen_waiting_for_the_pipeline_goes_on_once_it_opens() {
        let gfx = Gfx::default();
        let (_bridge, server) = gfx.open();
        let start = tokio::time::Instant::now();
        let opening = async {
            tokio::task::yield_now().await;
            client_opens(&server);
        };
        tokio::join!(gfx.negotiated(), gfx.negotiated(), opening);
        gfx.negotiated().await;
        assert!(
            start.elapsed() < NEGOTIATION,
            "both waiting and later screens go on at the opening"
        );
    }

    fn client_sends(server: &GfxServerHandle, pdu: GfxPdu) {
        let payload = ironrdp_core::encode_vec(&pdu).unwrap();
        server.lock().unwrap().process(0, &payload).unwrap();
    }

    fn acknowledge(server: &GfxServerHandle, frame_id: u32, queue_depth: u32) {
        client_sends(
            server,
            GfxPdu::FrameAcknowledge(FrameAcknowledgePdu {
                queue_depth: QueueDepth::from_u32(queue_depth),
                frame_id,
                total_frames_decoded: 0,
            }),
        );
    }

    /// The graphics PDUs the connection was handed so far.
    pub(crate) fn sent(events: &mut UnboundedReceiver<ServerEvent>) -> Vec<GfxPdu> {
        use ironrdp_dvc::pdu::{DrdynvcDataPdu, DrdynvcServerPdu};
        let mut zgfx = ironrdp_graphics::zgfx::Decompressor::new();
        let mut pdus = Vec::new();
        while let Ok(ServerEvent::Egfx(EgfxServerMessage::SendMessages { messages })) =
            events.try_recv()
        {
            for message in messages {
                let bytes = message.encode_unframed_pdu().unwrap();
                let DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(data)) =
                    ironrdp_core::decode(&bytes).unwrap()
                else {
                    panic!("each message is one DVC data PDU");
                };
                let mut gfx = Vec::new();
                zgfx.decompress(data.data(), &mut gfx).unwrap();
                pdus.push(ironrdp_core::decode(&gfx).unwrap());
            }
        }
        pdus
    }

    /// The ids of the frames in `pdus`.
    fn frame_ids(pdus: &[GfxPdu]) -> Vec<u32> {
        (pdus.iter())
            .filter_map(|pdu| match pdu {
                GfxPdu::StartFrame(start) => Some(start.frame_id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_offered_line_names_each_set_and_the_bytes_of_an_unknown_one() {
        let mut pdu = CapabilitiesAdvertisePdu::from_typed(&[CapabilitySet::V10_7 {
            flags: CapabilitiesV107Flags::SMALL_CACHE,
        }]);
        let hevc = [0x00, 0x04, 0x00, 0x00];
        pdu.0.push(RawCapabilitySet::new(
            CapabilityVersion(0x000B_0200),
            hevc.to_vec(),
        ));
        pdu.0.push(RawCapabilitySet::new(
            CapabilityVersion(0x0001_0000),
            vec![0x02, 0x00, 0x00, 0x10, 0xab],
        ));
        assert_eq!(
            offered(&pdu),
            "client offers graphics capabilities: \
             0x000a0701 flags 0x00000002 len 4; \
             0x000b0200 flags 0x00000400 len 4 data 00040000; \
             0x00010000 flags 0x10000002 len 5 data 02000010ab"
        );
    }

    #[test]
    fn the_offered_line_cuts_unknown_data_at_64_bytes() {
        let mut pdu = CapabilitiesAdvertisePdu(Vec::new());
        pdu.0.push(RawCapabilitySet::new(
            CapabilityVersion(0x0001_0000),
            vec![0xee; 70],
        ));
        let line = offered(&pdu);
        assert!(
            line.ends_with(&format!("len 70 data {}...", "ee".repeat(64))),
            "{line}"
        );
    }

    #[test]
    fn three_frames_the_client_has_not_acknowledged_hold_the_next() {
        let (gfx, server, mut events) = opened();
        assert!(matches!(
            gfx.prepare(&one((64, 64)), 0),
            Prepared::Ready { new: true, .. }
        ));
        let whole = [Rect::full((64, 64))];
        for _ in 0..3 {
            assert!(gfx.send(0, Video::Avc420(AU), &whole, 22));
        }
        assert!(matches!(gfx.prepare(&one((64, 64)), 0), Prepared::Behind));
        let ids = frame_ids(&sent(&mut events));
        assert_eq!(ids.len(), 3);
        acknowledge(&server, ids[0], 1);
        assert!(
            matches!(
                gfx.prepare(&one((64, 64)), 0),
                Prepared::Ready { new: false, .. }
            ),
            "an acknowledgement frees a place"
        );
    }

    #[test]
    fn a_client_advertising_again_gets_its_graphics_reset_and_new_surfaces() {
        let (gfx, server, mut events) = opened();
        let whole = [Rect::full((64, 64))];
        gfx.prepare(&one((64, 64)), 0);
        assert!(gfx.send(0, Video::Avc420(AU), &whole, 22));
        sent(&mut events);
        client_opens(&server);
        assert!(matches!(
            gfx.prepare(&one((64, 64)), 0),
            Prepared::Ready { new: true, .. }
        ));
        let pdus = sent(&mut events);
        assert!(
            matches!(
                &pdus[..],
                [
                    GfxPdu::ResetGraphics(_),
                    GfxPdu::CreateSurface(c),
                    GfxPdu::MapSurfaceToOutput(_),
                ] if c.surface_id == 0
            ),
            "{pdus:?}"
        );
        assert!(gfx.send(0, Video::Avc420(AU), &whole, 22));
        assert_eq!(last_frame(&sent(&mut events)).surface_id, 0);
    }

    #[test]
    fn the_reset_lists_the_picture_as_one_primary_monitor() {
        let (gfx, _server, mut events) = opened();
        gfx.prepare(&one((1920, 1080)), 0);
        let [GfxPdu::ResetGraphics(reset), ..] = &sent(&mut events)[..] else {
            panic!("the first PDU is a reset");
        };
        assert_eq!((reset.width, reset.height), (1920, 1080));
        assert_eq!(
            reset.monitors,
            vec![Monitor {
                left: 0,
                top: 0,
                right: 1919,
                bottom: 1079,
                flags: MonitorFlags::PRIMARY,
            }]
        );
    }

    fn screen(x: i32, y: i32, w: u32, h: u32) -> crate::layout::ScreenRect {
        crate::layout::ScreenRect {
            position: (x, y),
            logical: (w, h),
            pixels: (w, h),
        }
    }

    fn monitor(left: i32, right: i32, bottom: i32, primary: bool) -> Monitor {
        let flags = match primary {
            true => MonitorFlags::PRIMARY,
            false => MonitorFlags::empty(),
        };
        Monitor {
            left,
            top: 0,
            right,
            bottom,
            flags,
        }
    }

    /// The reset, then each surface made as (id, width, height) with where it is mapped.
    type Made = (u32, u32, Vec<Monitor>, Vec<((u16, u16, u16), (u32, u32))>);

    fn made(pdus: &[GfxPdu]) -> Made {
        let mut resets = pdus.iter().filter_map(|pdu| match pdu {
            GfxPdu::ResetGraphics(r) => Some(r),
            _ => None,
        });
        let reset = resets.next().expect("a reset");
        assert!(resets.next().is_none(), "one reset");
        let mut surfaces = Vec::new();
        for pair in pdus.windows(2) {
            if let [GfxPdu::CreateSurface(c), GfxPdu::MapSurfaceToOutput(m)] = pair {
                assert_eq!(c.surface_id, m.surface_id);
                surfaces.push((
                    (c.surface_id, c.width, c.height),
                    (m.output_origin_x, m.output_origin_y),
                ));
            }
        }
        (reset.width, reset.height, reset.monitors.clone(), surfaces)
    }

    #[test]
    fn two_screens_open_two_surfaces_mapped_at_their_targets() {
        let (gfx, _server, mut events) = opened();
        let client = [
            monitor(0, 2559, 1439, true),
            monitor(2560, 4479, 1079, false),
        ];
        let layout = Layout::new(
            &[screen(0, 0, 1920, 1200), screen(1920, 0, 1920, 1080)],
            &client,
        );
        for screen in [0, 1] {
            assert!(
                matches!(
                    gfx.prepare(&layout, screen),
                    Prepared::Ready { new: true, .. }
                ),
                "screen {screen} gets a new surface"
            );
        }
        assert!(matches!(
            gfx.prepare(&layout, 0),
            Prepared::Ready { new: false, .. }
        ));
        assert_eq!(
            made(&sent(&mut events)),
            (
                4480,
                1440,
                client.to_vec(),
                vec![((0, 2304, 1440), (128, 0)), ((1, 1920, 1080), (2560, 0))]
            )
        );
        let whole = [Rect::full((1920, 1080))];
        assert!(gfx.send(1, Video::Avc420(AU), &whole, 22));
        assert_eq!(last_frame(&sent(&mut events)).surface_id, 1);
    }

    #[test]
    fn a_secondary_at_negative_x_is_reset_where_the_client_has_it_and_mapped_in_the_desktop() {
        let (gfx, _server, mut events) = opened();
        let client = [
            monitor(-1920, -1, 1079, false),
            monitor(0, 2559, 1439, true),
        ];
        let layout = Layout::new(
            &[screen(0, 0, 1920, 1200), screen(1920, 0, 1920, 1080)],
            &client,
        );
        gfx.prepare(&layout, 0);
        gfx.prepare(&layout, 1);
        assert_eq!(
            made(&sent(&mut events)),
            (
                4480,
                1440,
                client.to_vec(),
                vec![((0, 2304, 1440), (2048, 0)), ((1, 1920, 1080), (0, 0))]
            ),
            "the monitors as the client sent them, the surfaces from the desktop's top left"
        );
    }

    #[test]
    fn a_moved_target_makes_only_its_own_surface_again() {
        let (gfx, _server, mut events) = opened();
        let client = [
            monitor(0, 1919, 1079, true),
            monitor(1920, 3839, 1079, false),
        ];
        let screens = [screen(0, 0, 1920, 1080), screen(1920, 0, 1920, 1080)];
        let before = Layout::new(&screens, &client);
        gfx.prepare(&before, 0);
        gfx.prepare(&before, 1);
        sent(&mut events);
        let after = Layout::new(&[screens[0], screen(1920, 0, 1920, 1200)], &client);
        assert!(matches!(
            gfx.prepare(&after, 0),
            Prepared::Ready { new: false, .. }
        ));
        assert!(matches!(
            gfx.prepare(&after, 1),
            Prepared::Ready { new: true, .. }
        ));
        let pdus = sent(&mut events);
        assert!(
            matches!(
                &pdus[..],
                [
                    GfxPdu::DeleteSurface(d),
                    GfxPdu::CreateSurface(c),
                    GfxPdu::MapSurfaceToOutput(m),
                ] if d.surface_id == 1
                    && (c.surface_id, c.width, c.height) == (2, 1728, 1080)
                    && (m.output_origin_x, m.output_origin_y) == (1920 + 96, 0)
            ),
            "{pdus:?}"
        );
    }

    /// The WireToSurface1 of the frame `pdus` end with.
    fn last_frame(pdus: &[GfxPdu]) -> &WireToSurface1Pdu {
        let [
            ..,
            GfxPdu::StartFrame(start),
            GfxPdu::WireToSurface1(w),
            GfxPdu::EndFrame(end),
        ] = pdus
        else {
            panic!("a frame is StartFrame, WireToSurface1 and EndFrame");
        };
        assert_eq!(start.frame_id, end.frame_id);
        w
    }

    #[test]
    fn a_one_pixel_column_has_exclusive_edges() {
        let (gfx, _server, mut events) = opened();
        gfx.prepare(&one((1920, 1080)), 0);
        let column = Rect {
            x: 1047,
            y: 10,
            width: 1,
            height: 5,
        };
        assert!(gfx.send(0, Video::Avc420(AU), &[column], 22));
        let pdus = sent(&mut events);
        let w = last_frame(&pdus);
        assert_eq!(w.codec_id, Codec1Type::Avc420);
        let region = Avc420Region::new(1047, 10, 1048, 15, 22, 100);
        assert_eq!(w.bitmap_data, encode_avc420_bitmap_stream(&[region], AU));
        assert_eq!(
            w.destination_rectangle,
            ExclusiveRectangle {
                left: 1047,
                top: 10,
                right: 1048,
                bottom: 15,
            }
        );
    }

    #[test]
    fn overlapping_damage_goes_as_its_bounds_up_to_the_pictures_edge() {
        let (gfx, _server, mut events) = opened();
        gfx.prepare(&one((1920, 1080)), 0);
        let column = Rect {
            x: 1047,
            y: 10,
            width: 1,
            height: 5,
        };
        let edge = Rect {
            x: 1900,
            y: 1070,
            width: 20,
            height: 10,
        };
        assert!(gfx.send(0, Video::Avc420(AU), &[column, edge, column], 22));
        let pdus = sent(&mut events);
        let w = last_frame(&pdus);
        let bounds = ExclusiveRectangle {
            left: 1047,
            top: 10,
            right: 1920,
            bottom: 1080,
        };
        assert_eq!(w.destination_rectangle, bounds);
        let region = Avc420Region::new(1047, 10, 1920, 1080, 22, 100);
        assert_eq!(
            w.bitmap_data,
            encode_avc420_bitmap_stream(&[region], AU),
            "one region, the bounds"
        );
    }

    #[test]
    fn an_avc444_frame_carries_lc_and_its_views() {
        use ironrdp_egfx::pdu::{Avc444BitmapStream, CapabilitiesV107Flags, Encoding};
        let (gfx, _server, mut events) = opened_with(CapabilitySet::V10_7 {
            flags: CapabilitiesV107Flags::empty(),
        });
        assert!(matches!(
            gfx.prepare(&one((64, 64)), 0),
            Prepared::Ready { avc444: true, .. }
        ));
        let block = Rect {
            x: 16,
            y: 32,
            width: 16,
            height: 16,
        };
        let (main, aux) = ([0, 0, 0, 1, 0x65], [0, 0, 0, 1, 0x41]);
        let region = Avc420Region::new(16, 32, 32, 48, 22, 100);
        let first = encode_avc420_bitmap_stream(std::slice::from_ref(&region), &main);
        for (views, lc) in [(Some(&aux[..]), 0), (None, 1)] {
            let video = Video::Avc444 {
                main: &main,
                aux: views,
            };
            assert!(gfx.send(0, video, &[block], 22));
            let pdus = sent(&mut events);
            let w = last_frame(&pdus);
            assert_eq!(w.codec_id, Codec1Type::Avc444);
            let data = &w.bitmap_data;
            let word = u32::from_le_bytes(data[..4].try_into().unwrap());
            assert_eq!(word >> 30, lc, "LC");
            assert_eq!(word & 0x3FFF_FFFF, first.len() as u32);
            assert_eq!(data[4..4 + first.len()], first, "the main view first");
            let rest =
                views.map(|aux| encode_avc420_bitmap_stream(std::slice::from_ref(&region), aux));
            assert_eq!(data[4 + first.len()..], rest.unwrap_or_default());
            let stream: Avc444BitmapStream<'_> = ironrdp_core::decode(data).unwrap();
            let encoding = match lc {
                0 => Encoding::LUMA_AND_CHROMA,
                _ => Encoding::LUMA,
            };
            assert_eq!(stream.encoding, encoding);
            assert_eq!(stream.stream1.data, main);
            assert_eq!(stream.stream2.map(|s| s.data), views);
        }
    }

    #[test]
    fn a_frame_the_channel_drops_awaits_no_acknowledgement() {
        let gfx = Gfx::default();
        let (_bridge, server) = gfx.open();
        let advertise = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED,
            },
        ]));
        let payload = ironrdp_core::encode_vec(&advertise).unwrap();
        server.lock().unwrap().process(0, &payload).unwrap();
        assert!(matches!(
            gfx.prepare(&one((64, 64)), 0),
            Prepared::Ready { new: true, .. }
        ));
        let whole = [Rect::full((64, 64))];
        for _ in 0..MAX_IN_FLIGHT {
            let au = Video::Avc420(&[0, 0, 0, 1, 0x65]);
            assert!(!gfx.send(0, au, &whole, 22), "the channel is not started");
        }
        assert!(
            matches!(
                gfx.prepare(&one((64, 64)), 0),
                Prepared::Ready { new: false, .. }
            ),
            "not behind"
        );
    }

    #[test]
    fn frames_flow_again_when_acknowledgements_resume_after_a_suspension() {
        const SUSPEND: u32 = 0xFFFF_FFFF;
        let (gfx, server, mut events) = opened();
        gfx.prepare(&one((64, 64)), 0);
        let whole = [Rect::full((64, 64))];
        let send = || assert!(gfx.send(0, Video::Avc420(AU), &whole, 22));
        let ready = || {
            matches!(
                gfx.prepare(&one((64, 64)), 0),
                Prepared::Ready { new: false, .. }
            )
        };
        (0..3).for_each(|_| send());
        let first = frame_ids(&sent(&mut events))[0];
        acknowledge(&server, first, SUSPEND);
        assert!(ready(), "suspended acknowledgements never hold frames");
        (0..1000).for_each(|_| send());
        assert!(ready(), "nothing is counted while suspended");
        let last = *frame_ids(&sent(&mut events)).last().unwrap();
        acknowledge(&server, last, 0);
        (0..2).for_each(|_| send());
        assert!(ready());
        send();
        assert!(!ready(), "counting resumes with the acknowledgements");
    }

    #[test]
    fn a_suspension_outlasts_closing_and_reopening_the_surface() {
        const SUSPEND: u32 = 0xFFFF_FFFF;
        let (gfx, server, mut events) = opened();
        gfx.prepare(&one((64, 64)), 0);
        assert!(gfx.send(0, Video::Avc420(AU), &[Rect::full((64, 64))], 22));
        let first = frame_ids(&sent(&mut events))[0];
        acknowledge(&server, first, SUSPEND);
        for size in [(64, 64), (128, 96)] {
            gfx.close(0);
            assert!(matches!(
                gfx.prepare(&one(size), 0),
                Prepared::Ready { new: true, .. }
            ));
            for _ in 0..4 {
                assert!(gfx.send(0, Video::Avc420(AU), &[Rect::full(size)], 22));
                assert!(
                    matches!(
                        gfx.prepare(&one(size), 0),
                        Prepared::Ready { new: false, .. }
                    ),
                    "no acknowledgement is awaited at {size:?}"
                );
            }
        }
    }
}
