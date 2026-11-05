//! One connection's graphics pipeline channel (MS-RDPEGFX), which carries the H.264 frames: one
//! surface the size of the picture, mapped at the origin.

use crate::stream::Rect;
use ironrdp_core::{Encode, EncodeResult, WriteCursor};
use ironrdp_dvc::{DvcEncode, DvcMessage};
use ironrdp_egfx::pdu::{
    Avc420Region, CapabilitiesAdvertisePdu, CapabilitySet, Codec1Type, EndFramePdu, GfxPdu,
    PixelFormat, StartFramePdu, Timestamp, WireToSurface1Pdu, encode_avc420_bitmap_stream,
};
use ironrdp_egfx::server::{GraphicsPipelineHandler, GraphicsPipelineServer};
use ironrdp_pdu::gcc::{Monitor, MonitorFlags};
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_server::{
    EgfxServerMessage, GfxDvcBridge, GfxServerFactory, GfxServerHandle, ServerEvent,
    ServerEventSender,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;

/// Frames sent and not yet acknowledged before the display waits.
const MAX_IN_FLIGHT: usize = 3;
/// The queue depth of a frame acknowledgement that suspends acknowledgements.
const SUSPEND_ACKS: u32 = 0xFFFF_FFFF;
/// How long after the channel is offered a resize waits for the client to open it: the
/// reactivation drops the client's channel replies, which leaves the connection on bitmaps.
pub const NEGOTIATION: Duration = Duration::from_secs(2);

/// Shared between the connection's server, which opens the channel, and its display, which
/// sends on it.
#[derive(Clone, Default)]
pub struct Gfx {
    link: Arc<Mutex<Link>>,
    /// Taken after the link and the server, never before.
    acks: Arc<Mutex<Acks>>,
    /// Told when the client has opened the pipeline.
    ready: Arc<Notify>,
}

pub enum Prepared {
    /// The surface takes the frame; a `new` one needs a whole picture. `avc444` when the
    /// client takes AVC444 too.
    Ready { new: bool, avc444: bool },
    /// The client is frames behind; try again later.
    Behind,
    /// The client takes no AVC420 (yet).
    Unavailable,
}

struct Link {
    server: Option<GfxServerHandle>,
    events: Option<UnboundedSender<ServerEvent>>,
    surface: Option<(u16, (u32, u32))>,
    start: Instant,
    /// When the channel was offered to the client.
    offered: Option<tokio::time::Instant>,
}

impl Default for Link {
    fn default() -> Link {
        Link {
            server: None,
            events: None,
            surface: None,
            start: Instant::now(),
            offered: None,
        }
    }
}

/// The frames the client has not acknowledged; none are counted while it suspends
/// acknowledgements, as it never acknowledges those.
#[derive(Default)]
struct Acks {
    next_id: u32,
    unacked: HashSet<u32>,
    suspended: bool,
}

impl Acks {
    fn behind(&self) -> bool {
        !self.suspended && self.unacked.len() >= MAX_IN_FLIGHT
    }

    /// The id the next frame sent gets.
    fn next(&self) -> u32 {
        self.next_id
    }

    /// Count the next frame as sent; its id.
    fn begin(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = id.wrapping_add(1);
        if !self.suspended {
            self.unacked.insert(id);
        }
        id
    }

    fn acknowledged(&mut self, id: u32, queue_depth: u32) {
        self.suspended = queue_depth == SUSPEND_ACKS;
        if self.suspended {
            self.unacked.clear();
        } else {
            self.unacked.remove(&id);
        }
    }
}

impl Gfx {
    /// What the connection's server opens the channel with.
    pub fn factory(&self) -> Box<dyn GfxServerFactory> {
        Box::new(Factory(self.clone()))
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
        let _ = tokio::time::timeout_at(at + NEGOTIATION, self.ready.notified()).await;
    }

    /// Ready the surface for an H.264 frame of `size`.
    pub fn prepare(&self, size: (u32, u32)) -> Prepared {
        let mut link = self.link.lock().unwrap();
        let Some(server) = link.server.clone() else {
            return Prepared::Unavailable;
        };
        let mut s = server.lock().unwrap();
        if !s.is_ready() || !s.supports_avc420() {
            return Prepared::Unavailable;
        }
        if link.surface.is_some_and(|(_, at)| at == size) {
            return match self.acks.lock().unwrap().behind() {
                true => Prepared::Behind,
                false => Prepared::Ready {
                    new: false,
                    avc444: s.supports_avc444(),
                },
            };
        }
        let (Ok(w), Ok(h)) = (u16::try_from(size.0), u16::try_from(size.1)) else {
            return Prepared::Unavailable;
        };
        link.surface = None;
        s.resize_with_monitors(w, h, monitors(w, h));
        let Some(id) = s.create_surface(w, h) else {
            return Prepared::Unavailable;
        };
        s.map_surface_to_output(id, 0, 0);
        link.surface = Some((id, size));
        self.acks.lock().unwrap().unacked.clear();
        flush(&link, &mut s, Vec::new());
        Prepared::Ready {
            new: true,
            avc444: s.supports_avc444(),
        }
    }

    /// Send coded `video` that changes `regions` of the surface; false if the channel would not
    /// take it, and then the frame awaits no acknowledgement.
    pub fn send(&self, video: Video<'_>, regions: &[Rect], qp: u8) -> bool {
        let link = self.link.lock().unwrap();
        let (Some(server), Some((surface_id, _))) = (link.server.clone(), link.surface) else {
            return false;
        };
        let mut s = server.lock().unwrap();
        let supported = match video {
            Video::Avc420(_) => s.supports_avc420(),
            Video::Avc444 { .. } => s.supports_avc444(),
        };
        if !s.is_ready() || !supported || regions.is_empty() {
            return false;
        }
        let frame_id = self.acks.lock().unwrap().next();
        let pdus = frame_pdus(
            surface_id,
            frame_id,
            link.start.elapsed().as_millis() as u32,
            video,
            regions,
            qp,
        );
        let sent = flush(&link, &mut s, pdus);
        if sent {
            self.acks.lock().unwrap().begin();
        }
        sent
    }

    /// Delete the surface, so the bitmap updates show again.
    pub fn close(&self) {
        let mut link = self.link.lock().unwrap();
        let (Some(server), Some((id, _))) = (link.server.clone(), link.surface.take()) else {
            return;
        };
        let mut s = server.lock().unwrap();
        s.delete_surface(id);
        flush(&link, &mut s, Vec::new());
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

/// StartFrame, the WireToSurface1 and EndFrame. The video updates the bounds of `regions` as one
/// RDPGFX_RECT16 (exclusive right and bottom), as clients refuse overlapping rectangles.
fn frame_pdus(
    surface_id: u16,
    frame_id: u32,
    ms: u32,
    video: Video<'_>,
    regions: &[Rect],
    qp: u8,
) -> Vec<GfxPdu> {
    let edge = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
    let bounds = ExclusiveRectangle {
        left: edge(regions.iter().map(|r| r.x).min().unwrap_or(0)),
        top: edge(regions.iter().map(|r| r.y).min().unwrap_or(0)),
        right: edge(regions.iter().map(|r| r.x + r.width).max().unwrap_or(0)),
        bottom: edge(regions.iter().map(|r| r.y + r.height).max().unwrap_or(0)),
    };
    let region = Avc420Region::new(
        bounds.left,
        bounds.top,
        bounds.right,
        bounds.bottom,
        qp,
        100,
    );
    let (codec_id, bitmap_data) = match video {
        Video::Avc420(data) => (
            Codec1Type::Avc420,
            encode_avc420_bitmap_stream(&[region], data),
        ),
        Video::Avc444 { main, aux } => (Codec1Type::Avc444, avc444_stream(&region, main, aux)),
    };
    let timestamp = Timestamp {
        milliseconds: (ms % 1000) as u16,
        seconds: (ms / 1000 % 60) as u8,
        minutes: (ms / 60_000 % 60) as u8,
        hours: (ms / 3_600_000 % 24) as u16,
    };
    vec![
        GfxPdu::StartFrame(StartFramePdu {
            timestamp,
            frame_id,
        }),
        GfxPdu::WireToSurface1(WireToSurface1Pdu {
            surface_id,
            codec_id,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: bounds,
            bitmap_data,
        }),
        GfxPdu::EndFrame(EndFramePdu { frame_id }),
    ]
}

/// RFX_AVC444_BITMAP_STREAM (MS-RDPEGFX 2.2.4.5): the main stream's size in 30 bits under LC (0
/// both views, 1 main only), the main view's stream, then the auxiliary view's.
fn avc444_stream(region: &Avc420Region, main: &[u8], aux: Option<&[u8]>) -> Vec<u8> {
    let regions = std::slice::from_ref(region);
    let first = encode_avc420_bitmap_stream(regions, main);
    let lc: u32 = if aux.is_some() { 0 } else { 1 };
    let mut out = Vec::new();
    out.extend_from_slice(&((first.len() as u32 & 0x3FFF_FFFF) | lc << 30).to_le_bytes());
    out.extend_from_slice(&first);
    if let Some(aux) = aux {
        out.extend_from_slice(&encode_avc420_bitmap_stream(regions, aux));
    }
    out
}

/// Hand what the pipeline queued, then `pdus`, to the connection's event loop; false if they
/// were dropped.
fn flush(link: &Link, server: &mut GraphicsPipelineServer, pdus: Vec<GfxPdu>) -> bool {
    let mut messages = server.drain_output();
    messages.extend(pdus.iter().map(wrapped));
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

/// `pdu` in an uncompressed ZGFX segment, as the channel carries it.
fn wrapped(pdu: &GfxPdu) -> DvcMessage {
    let bytes = ironrdp_core::encode_vec(pdu).expect("a graphics PDU encodes");
    Box::new(Wrapped(ironrdp_graphics::zgfx::wrap_uncompressed(&bytes)))
}

struct Wrapped(Vec<u8>);

impl Encode for Wrapped {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RDPGFX"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for Wrapped {}

/// The one monitor ResetGraphics lists: the whole picture, primary. Clients refuse a reset
/// without monitors.
fn monitors(w: u16, h: u16) -> Vec<Monitor> {
    vec![Monitor {
        left: 0,
        top: 0,
        right: i32::from(w) - 1,
        bottom: i32::from(h) - 1,
        flags: MonitorFlags::PRIMARY,
    }]
}

struct Factory(Gfx);

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.0.link.lock().unwrap().events = Some(sender);
    }
}

impl GfxServerFactory for Factory {
    fn build_gfx_handler(&self) -> Box<dyn GraphicsPipelineHandler> {
        Box::new(Handler(self.0.acks.clone(), self.0.ready.clone()))
    }

    fn build_server_with_handle(&self) -> Option<(GfxDvcBridge, GfxServerHandle)> {
        let handler = Box::new(Handler(self.0.acks.clone(), self.0.ready.clone()));
        let server = Arc::new(Mutex::new(GraphicsPipelineServer::new(handler)));
        let mut link = self.0.link.lock().unwrap();
        link.server = Some(server.clone());
        link.surface = None;
        link.offered = Some(tokio::time::Instant::now());
        *self.0.acks.lock().unwrap() = Acks::default();
        Some((GfxDvcBridge::new(server.clone()), server))
    }
}

/// Logs the negotiation, tells the display it is done and counts the client's frame
/// acknowledgements.
struct Handler(Arc<Mutex<Acks>>, Arc<Notify>);

impl GraphicsPipelineHandler for Handler {
    fn capabilities_advertise(&mut self, _: &CapabilitiesAdvertisePdu) {}

    fn on_ready(&mut self, negotiated: &CapabilitySet) {
        log::info!("graphics pipeline ready: {negotiated:?}");
        self.1.notify_one();
    }

    fn on_frame_ack(&mut self, frame_id: u32, queue_depth: u32, _: u32) {
        self.0.lock().unwrap().acknowledged(frame_id, queue_depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reset_lists_the_picture_as_one_primary_monitor() {
        assert_eq!(
            monitors(1920, 1080),
            vec![Monitor {
                left: 0,
                top: 0,
                right: 1919,
                bottom: 1079,
                flags: MonitorFlags::PRIMARY,
            }]
        );
    }

    fn wire_to_surface(pdus: &[GfxPdu]) -> &WireToSurface1Pdu {
        assert!(matches!(
            pdus[0],
            GfxPdu::StartFrame(StartFramePdu { frame_id: 7, .. })
        ));
        assert!(matches!(
            pdus[2],
            GfxPdu::EndFrame(EndFramePdu { frame_id: 7 })
        ));
        match &pdus[1] {
            GfxPdu::WireToSurface1(w) => w,
            _ => panic!("WireToSurface1 between StartFrame and EndFrame"),
        }
    }

    #[test]
    fn a_one_pixel_column_has_exclusive_edges() {
        let column = Rect {
            x: 1047,
            y: 10,
            width: 1,
            height: 5,
        };
        let pdus = frame_pdus(1, 7, 0, Video::Avc420(&[0, 0, 0, 1, 0x65]), &[column], 22);
        let w = wire_to_surface(&pdus);
        let region = Avc420Region::new(1047, 10, 1048, 15, 22, 100);
        assert_eq!(
            w.bitmap_data,
            encode_avc420_bitmap_stream(&[region], &[0, 0, 0, 1, 0x65])
        );
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
        let au = Video::Avc420(&[0, 0, 0, 1, 0x65]);
        let pdus = frame_pdus(1, 7, 0, au, &[column, edge, column], 22);
        let w = wire_to_surface(&pdus);
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
            encode_avc420_bitmap_stream(&[region], &[0, 0, 0, 1, 0x65]),
            "one region, the bounds"
        );
    }

    #[test]
    fn an_avc444_frame_carries_lc_and_its_views() {
        use ironrdp_egfx::pdu::{Avc444BitmapStream, Encoding};
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
            let pdus = frame_pdus(1, 7, 0, video, &[block], 22);
            let w = wire_to_surface(&pdus);
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
        use ironrdp_dvc::DvcProcessor;
        use ironrdp_egfx::pdu::CapabilitiesV81Flags;
        let gfx = Gfx::default();
        let (_bridge, server) = gfx.factory().build_server_with_handle().unwrap();
        let advertise = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&[
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED,
            },
        ]));
        let payload = ironrdp_core::encode_vec(&advertise).unwrap();
        server.lock().unwrap().process(0, &payload).unwrap();
        assert!(matches!(
            gfx.prepare((64, 64)),
            Prepared::Ready { new: true, .. }
        ));
        let whole = [Rect::full((64, 64))];
        for _ in 0..MAX_IN_FLIGHT {
            let au = Video::Avc420(&[0, 0, 0, 1, 0x65]);
            assert!(!gfx.send(au, &whole, 22), "the channel is not started");
        }
        assert!(
            matches!(gfx.prepare((64, 64)), Prepared::Ready { new: false, .. }),
            "not behind"
        );
    }

    #[test]
    fn three_unacknowledged_frames_hold_the_next_until_one_is_acknowledged() {
        let mut acks = Acks::default();
        let ids: Vec<u32> = (0..3).map(|_| acks.begin()).collect();
        assert!(acks.behind());
        acks.acknowledged(ids[0], 1);
        assert!(!acks.behind());
        acks.begin();
        assert!(acks.behind());
        acks.acknowledged(99, SUSPEND_ACKS);
        assert!(
            !acks.behind(),
            "suspended acknowledgements never hold frames"
        );
    }

    #[test]
    fn frames_flow_again_when_acknowledgements_resume_after_a_suspension() {
        let mut acks = Acks::default();
        let before: Vec<u32> = (0..3).map(|_| acks.begin()).collect();
        acks.acknowledged(before[0], SUSPEND_ACKS);
        for _ in 0..1000 {
            acks.begin();
        }
        assert!(
            acks.unacked.is_empty(),
            "nothing is counted while suspended"
        );
        let last = acks.begin();
        acks.acknowledged(last, 0);
        assert!(
            !acks.behind(),
            "the first acknowledgement after a suspension frees the queue"
        );
        acks.begin();
        acks.begin();
        assert!(!acks.behind());
        acks.begin();
        assert!(acks.behind(), "counting resumes with the acknowledgements");
    }
}
