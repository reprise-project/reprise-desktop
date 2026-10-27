//! PickColor's picker, run as our own child (`--pickcolor`, see
//! `dialog`). It freezes every output first (ext-image-copy-capture, no
//! cursor painted), then shows the still of the output under the pointer
//! on an overlay flyout over it, drawn by `controls::colorpick` with its
//! loupe beside the pointer. A pick prints the pixel as `r g b` (0-255)
//! on stdout; a cancel prints nothing, which the parent reads as one.
//!
//! What is picked is what the still shows, so the overlay's own drawing
//! is never sampled. An output's rotation is not undone, as with
//! Screenshot's PNG.

use crate::wayland::{Target, Wayland};
use repriseapp::{App, Ctx, Event, Flyout, PointerInput, Reply, WinId};
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::colorpick::{self, ColorPick, Still};
use reprisekit::raster::{Canvas, Rect};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use wayland_client::protocol::wl_shm::Format;

const FLYOUT: WinId = "pickcolor";
const TITLE: &str = "Pick a colour";

pub(crate) struct Output {
    pub(crate) layout: Rect,
    pub(crate) still: Still,
}

/// Ask the user for a colour on screen. Blocks the calling thread; None
/// when they cancel, the capture fails, or `cancel` is set (a frontend
/// Close). The channels are 0-1, as the portal answers them.
pub fn pick(cancel: &AtomicBool) -> Option<(f64, f64, f64)> {
    parse(&crate::dialog::run(&["--pickcolor"], "", cancel)?)
}

fn parse(out: &str) -> Option<(f64, f64, f64)> {
    let mut it = out.split_whitespace().map(|v| v.parse::<u8>().ok());
    let (r, g, b) = (it.next()??, it.next()??, it.next()??);
    let unit = |v: u8| f64::from(v) / 255.0;
    Some((unit(r), unit(g), unit(b)))
}

fn answer(argb: u32) -> String {
    let [_, r, g, b] = argb.to_be_bytes();
    format!("{r} {g} {b}")
}

pub fn run_dialog() {
    let stills = match capture(false) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => return log::warn!("PickColor: no output to capture"),
        Err(e) => return log::warn!("PickColor: {e}"),
    };
    let picked = Arc::new(Mutex::new(None));
    repriseapp::run(Box::new(Picker {
        stills,
        pick: ColorPick::default(),
        picked: picked.clone(),
        done: false,
        backdrop: None,
    }));
    if let Some(argb) = *picked.lock().unwrap() {
        println!("{}", answer(argb));
    }
}

/// A still of every output with its place in the layout; `cursor` paints
/// the pointer into them.
pub(crate) fn capture(cursor: bool) -> Result<Vec<Output>, String> {
    let mut wl = Wayland::connect()?;
    let mut stills = Vec::new();
    for out in wl.outputs() {
        let still = grab(&mut wl, &Target::Output(out.name.clone()), cursor)
            .map_err(|e| format!("{}: {e}", out.name))?;
        stills.push(Output {
            layout: Rect::new(out.x, out.y, out.width, out.height),
            still: Still {
                logical: (out.width, out.height),
                ..still
            },
        });
    }
    Ok(stills)
}

/// One frame of `target` as ARGB at its buffer size; `logical` is left
/// at the buffer size for the caller to set.
pub(crate) fn grab(wl: &mut Wayland, target: &Target, cursor: bool) -> Result<Still, String> {
    let session = wl.open_session(target, cursor)?;
    let c = wl.constraints();
    let format = c.pick_format().ok_or("no 32 bpp shm format")?;
    let buf = wl.shm_buffer(c.width, c.height, format)?;
    // A source always has a first frame to give; two seconds is for a
    // compositor busy with something else.
    wl.capture(&session, &buf, std::time::Duration::from_secs(2))
        .map_err(|e| format!("capture failed: {e:?}"))?;
    let bytes = buf.read()?;
    let row = buf.stride as usize;
    let mut pixels = Vec::with_capacity((c.width * c.height) as usize);
    for y in 0..c.height as usize {
        let (words, _) = bytes[y * row..].as_chunks::<4>();
        pixels.extend(
            words[..c.width as usize]
                .iter()
                .map(|&w| to_argb(w, format)),
        );
    }
    Ok(Still {
        logical: (c.width as i32, c.height as i32),
        width: c.width,
        height: c.height,
        pixels,
    })
}

fn to_argb(px: [u8; 4], format: Format) -> u32 {
    let (r, g, b) = match format {
        Format::Argb8888 | Format::Xrgb8888 => (px[2], px[1], px[0]),
        _ => (px[0], px[1], px[2]),
    };
    u32::from_be_bytes([0xff, r, g, b])
}

/// The still for the output at `origin` of `size` (logical), else the
/// first of `size`, else the first. Until the flyout enters its output
/// the origin reads (0, 0), so it alone decides nothing.
fn still_for(stills: &[Output], origin: (i32, i32), size: (i32, i32)) -> usize {
    stills
        .iter()
        .position(|s| s.layout == Rect::new(origin.0, origin.1, size.0, size.1))
        .or_else(|| {
            stills
                .iter()
                .position(|s| (s.layout.width, s.layout.height) == size)
        })
        .unwrap_or(0)
}

struct Picker {
    stills: Vec<Output>,
    pick: ColorPick,
    picked: Arc<Mutex<Option<u32>>>,
    done: bool,
    /// The still drawn at the canvas's device size, for the still index
    /// and size it was made for.
    backdrop: Option<((usize, u32, u32), Vec<u32>)>,
}

impl Picker {
    fn finish(&mut self, pick: Option<u32>) -> Reply {
        *self.picked.lock().unwrap() = pick;
        self.done = true;
        Reply {
            quit: true,
            ..Default::default()
        }
    }

    fn still(&self, ctx: &Ctx) -> &Still {
        &self.stills[still_for(&self.stills, ctx.origin, ctx.size)].still
    }

    fn reply(&mut self, ev: colorpick::Event, ctx: &Ctx) -> Reply {
        match ev {
            colorpick::Event::None => Reply::none(),
            colorpick::Event::Redraw => Reply::redraw(),
            colorpick::Event::Cancel => self.finish(None),
            colorpick::Event::Pick(p) => {
                let argb = self.still(ctx).sample(p);
                self.finish(Some(argb))
            }
        }
    }
}

impl App for Picker {
    fn title(&self) -> String {
        TITLE.into()
    }
    fn app_id(&self) -> String {
        "reprise-portal".into()
    }
    fn initial_size(&self) -> (i32, i32) {
        (1, 1)
    }
    fn hidden(&self) -> bool {
        true
    }
    fn flyout(&self) -> Option<Flyout> {
        (!self.done).then_some(Flyout {
            id: FLYOUT,
            at: Flyout::UNDER_POINTER,
            title: TITLE,
        })
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if win != FLYOUT {
            return match ev {
                Event::CloseRequest => self.finish(None),
                _ => Reply::none(),
            };
        }
        let ev = match ev {
            Event::Pointer(PointerInput::Motion { x, y }) => self.pick.motion((x, y)),
            Event::Pointer(PointerInput::Leave) => self.pick.leave(),
            Event::Pointer(PointerInput::Press { x, y, button, .. }) => match button {
                repriseapp::BTN_LEFT => self.pick.press((x, y), true),
                repriseapp::BTN_RIGHT => self.pick.press((x, y), false),
                _ => colorpick::Event::None,
            },
            Event::Key(k) => self.pick.key(k.sym, ctx.size),
            Event::CloseRequest => colorpick::Event::Cancel,
            _ => colorpick::Event::None,
        };
        self.reply(ev, ctx)
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        if win != FLYOUT {
            return;
        }
        let idx = still_for(&self.stills, ctx.origin, ctx.size);
        let (dw, dh) = c.device_size();
        let key = (idx, dw, dh);
        if self.backdrop.as_ref().is_none_or(|(k, _)| *k != key) {
            self.backdrop = Some((key, self.stills[idx].still.backdrop(dw, dh)));
        }
        let backdrop = self.backdrop.as_ref().map_or(&[][..], |(_, px)| px);
        colorpick::draw(
            c,
            &ctx.text,
            ctx.theme.fonts.size_px,
            &self.pick,
            &self.stills[idx].still,
            backdrop,
        );
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        if win == FLYOUT {
            colorpick::access(&self.pick, self.still(ctx), ctx.size, tree, ROOT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(layout: Rect) -> Output {
        Output {
            layout,
            still: Still {
                logical: (layout.width, layout.height),
                width: 1,
                height: 1,
                pixels: vec![0],
            },
        }
    }

    #[test]
    fn the_answer_survives_the_pipe_as_unit_channels() {
        let out = answer(0xff80_00ff);
        assert_eq!(out, "128 0 255");
        let (r, g, b) = parse(&format!("{out}\n")).unwrap();
        assert!((r - 128.0 / 255.0).abs() < 1e-9);
        assert_eq!((g, b), (0.0, 1.0));
    }

    #[test]
    fn nothing_or_junk_on_stdout_is_a_cancel() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("1 2"), None);
        assert_eq!(parse("1 2 300"), None);
    }

    #[test]
    fn pixels_read_back_in_either_byte_order() {
        assert_eq!(to_argb([3, 2, 1, 0], Format::Xrgb8888), 0xff01_0203);
        assert_eq!(to_argb([1, 2, 3, 0], Format::Xbgr8888), 0xff01_0203);
    }

    #[test]
    fn the_still_is_the_one_for_the_flyouts_output() {
        let stills = [
            output(Rect::new(0, 0, 1920, 1080)),
            output(Rect::new(1920, 0, 1280, 1024)),
        ];
        assert_eq!(still_for(&stills, (1920, 0), (1280, 1024)), 1);
        // Before the surface entered an output, the size decides.
        assert_eq!(still_for(&stills, (0, 0), (1280, 1024)), 1);
        assert_eq!(still_for(&stills, (0, 0), (1920, 1080)), 0);
        assert_eq!(still_for(&stills, (7, 7), (1, 1)), 0);
    }
}
