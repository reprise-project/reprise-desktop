//! Space's preview window over the selected item: [`filepreview`]'s control, with the file read on a
//! listing thread, so a file on a dead mount holds that thread and never the window.

use crate::Explorer;
use crate::icons::IconSlot;
use repriseapp::decode::Decoders;
use repriseapp::listjob::Job;
use repriseapp::{BTN_LEFT, Child, Ctx, Event, MAIN, PointerInput, Reply, WinId};
use reprisefs::thumbnails::Thumbnails;
use reprisekit::controls::filepreview::{
    self, Action, Body, Facts, FilePreview, PAD, Route, SNIFF_BYTES, TEXT_BYTES,
};
use reprisekit::controls::fontsample;
use reprisekit::controls::multiedit::{Metrics, MultiEdit};
use reprisekit::controls::shellview;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use std::io::Read;
use std::path::Path;
use std::rc::Rc;

pub const PREVIEW: WinId = WinId::named("preview");

/// The text box's face size at 100%.
const MONO_PX: u32 = 13;
/// The work area's share the window may take, in percent of each side.
const SHARE: i32 = 70;

pub(crate) struct Preview {
    pub qv: FilePreview,
    /// The icon the card wears where the entry drew none at its size, worn once it lands.
    icon: Option<IconSlot>,
    /// The file's read, until it lands.
    job: Option<Job<Loaded>>,
    mono: TextRenderer,
    metrics: Metrics,
    /// The client size picked for the body, inside [`SHARE`] of the output.
    size: (i32, i32),
    within: (i32, i32),
}

/// What the read made of a file; `Nothing` keeps the card.
pub(crate) enum Loaded {
    Picture(Canvas),
    Text(String),
    Nothing,
}

/// The file at `path` for its preview, pictures fitted inside `max` device px at `scale`: decoded
/// by `decoders`, read as text, or else the largest thumbnail `cache` holds (another program's
/// thumbnailer may have made one for a PDF or a video). Anything but a regular file is never
/// opened, since the open may never return.
pub(crate) fn read(
    path: &Path,
    route: Route,
    decoders: &Decoders,
    cache: Option<&Thumbnails>,
    fonts: &FontSampler,
    max: (u32, u32),
    scale: i32,
) -> Loaded {
    let Ok(meta) = std::fs::metadata(path) else {
        return Loaded::Nothing;
    };
    if !meta.is_file() {
        return Loaded::Nothing;
    }
    let read_text = || -> Option<String> {
        let mut head = Vec::with_capacity(SNIFF_BYTES);
        let mut f = std::fs::File::open(path).ok()?;
        (&mut f)
            .take(SNIFF_BYTES as u64)
            .read_to_end(&mut head)
            .ok()?;
        if !filepreview::looks_text(&head) || (route == Route::Other && head.is_empty()) {
            return None;
        }
        f.take((TEXT_BYTES - head.len()) as u64)
            .read_to_end(&mut head)
            .ok()?;
        Some(filepreview::text_of(&head))
    };
    let picture = || {
        decoders
            .decode_file(path, max)
            .map(|d| d.canvas.at_scale(scale))
    };
    let found = match route {
        Route::Picture => picture().map(Loaded::Picture),
        Route::Font => fonts.sample(path, max, scale).map(Loaded::Picture),
        Route::Text | Route::Other => read_text().map(Loaded::Text),
    };
    found
        .or_else(|| cached(path, &meta, cache?, max, scale).map(Loaded::Picture))
        .unwrap_or(Loaded::Nothing)
}

/// How a font's sample is drawn: by `exe` [`FONT_SAMPLE`] in the thumbnailers' jail, so a font file
/// is never parsed in Explorer. No runner draws none.
pub(crate) struct FontSampler {
    pub runner: Option<reprisefs::thumbnailers::Runner>,
    pub exe: std::path::PathBuf,
}

impl FontSampler {
    /// Explorer's own program on `PATH`, which an upgrade replaces under a running Explorer,
    /// jailed as the thumbnailers are; no runner without bubblewrap.
    pub(crate) fn installed() -> FontSampler {
        FontSampler {
            runner: reprisefs::thumbnailers::Runner::jailed(),
            exe: reprisefs::thumbpipe::PROGRAM.into(),
        }
    }

    /// The sample of the font file at `path` inside `max` device px at `scale`.
    fn sample(&self, path: &Path, max: (u32, u32), scale: i32) -> Option<Canvas> {
        use reprisefs::thumbnailers::{MAX_SIDE, Thumbnailer};
        let runner = self.runner.as_ref()?;
        let (w, h) = (max.0.min(MAX_SIDE), max.1.min(MAX_SIDE));
        let exe = reprisefs::apps::exec_quote(self.exe.to_str()?);
        let t = Thumbnailer::command(
            "font-sample",
            format!("{exe} {FONT_SAMPLE} {w} {h} {scale} %i %o"),
        );
        let png = (runner.run(&t, path, 0))
            .map_err(|e| log::debug!("font sample {}: {e:?}", path.display()))
            .ok()?;
        let c = reprisekit::icontheme::png_to_canvas(&png, MAX_SIDE).ok()?;
        Some(c.at_scale(scale))
    }
}

/// Explorer's helper mode `--font-sample W H SCALE FONT OUT`: [`font_sample`] as a PNG at OUT.
pub const FONT_SAMPLE: &str = "--font-sample";

/// [`FONT_SAMPLE`] with the arguments after the flag.
pub fn serve_font_sample(args: &[std::ffi::OsString]) -> Result<(), String> {
    let [w, h, scale, font, out] = args else {
        return Err("usage: --font-sample W H SCALE FONT OUT".into());
    };
    let num = |a: &std::ffi::OsString| a.to_str().and_then(|s| s.parse::<u32>().ok());
    let (Some(w), Some(h), Some(scale)) = (num(w), num(h), num(scale)) else {
        return Err("W, H and SCALE are numbers".into());
    };
    let c = font_sample(Path::new(font), (w, h), scale.clamp(1, 4) as i32)
        .ok_or_else(|| format!("{}: no face", Path::new(font).display()))?;
    std::fs::write(out, reprisekit::raster::to_png(&c)).map_err(|e| e.to_string())
}

/// The font file at `path`'s first face set as a sample, inside `max` device px, in Classic's
/// window colours: a sample is paper whatever the look.
fn font_sample(path: &Path, max: (u32, u32), scale: i32) -> Option<Canvas> {
    let sample = TextRenderer::from_file(path, 0)?;
    let theme = reprisekit::theme::Theme::default();
    let ui = TextRenderer::load(&theme.fonts);
    let cs = &theme.colors;
    let ink = (cs.window_text, cs.window, cs.button_shadow);
    Some(fontsample::picture(
        &ui,
        theme.fonts.size_px,
        &sample,
        ink,
        max,
        scale,
    ))
}

/// The largest thumbnail of `path` in the cache, fitted inside `max` device px.
fn cached(
    path: &Path,
    meta: &std::fs::Metadata,
    cache: &Thumbnails,
    max: (u32, u32),
    scale: i32,
) -> Option<Canvas> {
    [1024, 512, 256, 128].into_iter().find_map(|edge| {
        let hit = cache.get(path, meta, edge)?;
        let c = reprisekit::icontheme::png_to_canvas(&hit.png, edge).ok()?;
        let (w, h) = reprisekit::raster::fit_size(c.width, c.height, max);
        Some(c.resample_box(w, h).at_scale(scale))
    })
}

impl Explorer {
    /// Space in the view: the preview of the one selected item, or closes the one up.
    pub(crate) fn toggle_preview(&mut self, ctx: &mut Ctx) -> Reply {
        if self.preview.take().is_some() {
            return Reply::redraw();
        }
        match self.sv.selection.single() {
            Some(i) => self.preview_entry(i, ctx),
            None => Reply::none(),
        }
    }

    /// Show entry `i` in the preview, opening the window if it is not up; waits for the read as
    /// long as a listing is waited for, so a quick read opens at its own size.
    fn preview_entry(&mut self, i: usize, ctx: &mut Ctx) -> Reply {
        let (Some(it), Some(e)) = (self.items.get(i), self.sv.entries.get(i)) else {
            return Reply::none();
        };
        let facts = Facts {
            name: it.label().to_string(),
            kind: e.kind.clone(),
            size: (!e.is_folder()).then(|| reprisefs::props::format_size(it.size)),
            modified: e.modified.clone(),
        };
        let route = filepreview::route(it.mime.as_deref());
        let folder = e.is_folder();
        let slot = e.icon48.is_none().then(|| {
            let pick = crate::icons::file_pick(&self.globs, &it.path, folder);
            IconSlot::new(&mut ctx.icons, pick, filepreview::ICON_PX)
        });
        let icon = match &slot {
            Some(slot) => slot.icon().map(Rc::new),
            None => e.icon48.clone(),
        };
        let (w, h) = ctx.output_size.unwrap_or((1280, 800));
        let within = (w * SHARE / 100, h * SHARE / 100);
        let scale = self.drawn_scale.max(1);
        let max = (
            ((within.0 - 2 * PAD).max(1) * scale) as u32,
            ((within.1 - 2 * PAD).max(1) * scale) as u32,
        );
        let job = (!folder).then(|| {
            let p = it.path.clone();
            Job::spawn(Some(&ctx.wake), move || {
                let cache = repriseapp::thumbs::cache_for(repriseconf::thumbnail_cache());
                read(
                    &p,
                    route,
                    &Decoders::default_set(),
                    cache.as_ref(),
                    &FontSampler::installed(),
                    max,
                    scale,
                )
            })
        });
        let landed = job.as_ref().and_then(|j| j.wait(self.list_wait()));
        let (mono, metrics) = match self.preview.take() {
            Some(p) => (p.mono, p.metrics),
            None => {
                let mut m = Metrics {
                    font_px: MONO_PX,
                    line_h: MONO_PX as i32,
                    bars_always: false,
                    ..Metrics::default()
                };
                m.follow_look(&ctx.theme.controls());
                (TextRenderer::monospace(MONO_PX), m)
            }
        };
        let mut p = Preview {
            qv: FilePreview::new(facts, icon),
            icon: slot,
            job: if landed.is_some() { None } else { job },
            mono,
            metrics,
            size: filepreview::CARD,
            within,
        };
        if let Some(l) = landed {
            p.land(l);
        }
        p.size = p.qv.size(&p.mono, &p.metrics, within);
        self.preview = Some(p);
        Reply {
            redraw_wins: vec![PREVIEW],
            ..Reply::redraw()
        }
    }

    /// A read that landed after the wait.
    pub(crate) fn take_preview(&mut self) -> Option<Reply> {
        let p = self.preview.as_mut()?;
        let landed = p.job.as_ref()?.take()?;
        p.job = None;
        p.land(landed);
        p.size = p.qv.size(&p.mono, &p.metrics, p.within);
        Some(Reply {
            redraw_wins: vec![PREVIEW],
            ..Reply::none()
        })
    }

    /// The card wears its icon once it landed: true when it did.
    pub(crate) fn land_preview_icon(&mut self, ctx: &mut Ctx) -> bool {
        let Some(p) = self.preview.as_mut() else {
            return false;
        };
        let Some(slot) = p.icon.as_mut() else {
            return false;
        };
        let landed = slot.land(&mut ctx.icons);
        if landed {
            p.qv.icon = slot.icon().map(Rc::new);
        }
        landed
    }

    pub(crate) fn preview_child(&self) -> Option<Child> {
        let p = self.preview.as_ref()?;
        Some(Child {
            id: PREVIEW,
            title: p.qv.facts.name.clone(),
            size: p.size,
            fixed: false,
            modal: false,
        })
    }

    pub(crate) fn preview_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let r = Rect::new(0, 0, ctx.size.0, ctx.size.1);
        match ev {
            Event::Key(k) => match filepreview::key(k.sym, k.mods.ctrl, k.mods.alt) {
                Action::Close => self.close_preview(),
                Action::Open => {
                    self.preview = None;
                    let opened = self.as_main(ctx, |ex, ctx| ex.open_selected(ctx));
                    Reply {
                        redraw_wins: vec![MAIN],
                        ..opened
                    }
                }
                Action::Step(dir) => self.as_main(ctx, |ex, ctx| ex.step_preview(dir, ctx)),
                Action::Page(n) => self.with_preview(|p| p.qv.page(n, &p.mono, &p.metrics, r)),
                Action::None => Reply::none(),
            },
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => self.with_preview(|p| p.qv.press((x, y), &p.mono, &p.metrics, r)),
            Event::Pointer(PointerInput::Motion { x, y }) => {
                self.with_preview(|p| p.qv.motion((x, y), &p.mono, &p.metrics, r))
            }
            Event::Pointer(PointerInput::Release { .. }) => self.with_preview(|p| {
                p.qv.release();
                false
            }),
            Event::Pointer(PointerInput::Scroll { rows, .. }) => {
                self.with_preview(|p| p.qv.scroll(rows as i64, &p.mono, &p.metrics, r))
            }
            Event::CloseRequest => self.close_preview(),
            Event::Resized => Reply::redraw(),
            _ => Reply::none(),
        }
    }

    pub(crate) fn draw_preview(&self, c: &mut Canvas, ctx: &Ctx) {
        let Some(p) = &self.preview else {
            c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
            return;
        };
        let r = Rect::new(0, 0, ctx.size.0, ctx.size.1);
        let px = ctx.theme.fonts.size_px;
        let paint = ctx.theme.paint();
        filepreview::draw(c, &paint, &ctx.text, px, &p.mono, &p.metrics, &p.qv, r);
    }

    pub(crate) fn access_preview(&self, tree: &mut reprisekit::access::Tree, ctx: &Ctx) {
        if let Some(p) = &self.preview {
            let r = Rect::new(0, 0, ctx.size.0, ctx.size.1);
            let root = reprisekit::access::ROOT;
            filepreview::access(tree, root, &p.qv, &p.mono, &p.metrics, r);
        }
    }

    fn close_preview(&mut self) -> Reply {
        self.preview = None;
        Reply::redraw()
    }

    /// An arrow in the preview: the view's selection moves `by` items along its order, scrolled
    /// into view, and the preview shows what it lands on.
    fn step_preview(&mut self, by: i64, ctx: &mut Ctx) -> Reply {
        let from = self.sv.selection.focus;
        let to = reprisekit::selection::step_index(from, by, self.sv.entries.len());
        let Some(i) = to.filter(|&i| Some(i) != from) else {
            return Reply::none();
        };
        self.sv.selection.set_only(Some(i));
        let frame = self.layout(ctx).view;
        let sl = self.view_layout(frame, ctx);
        shellview::follow(&mut self.sv, &sl);
        self.refresh_chrome(ctx);
        let shown = self.preview_entry(i, ctx);
        Reply {
            redraw_wins: vec![MAIN, PREVIEW],
            ..shown
        }
    }

    /// Run `f` with `ctx` sized as the main window, for an event of the preview's that lays the
    /// view out.
    fn as_main<T>(&mut self, ctx: &mut Ctx, f: impl FnOnce(&mut Explorer, &mut Ctx) -> T) -> T {
        let keep = std::mem::replace(&mut ctx.size, self.main_size);
        let out = f(self, ctx);
        ctx.size = keep;
        out
    }

    fn with_preview(&mut self, f: impl FnOnce(&mut Preview) -> bool) -> Reply {
        match self.preview.as_mut().map(f) {
            Some(true) => Reply::redraw(),
            _ => Reply::none(),
        }
    }
}

impl Preview {
    fn land(&mut self, l: Loaded) {
        match l {
            Loaded::Picture(c) => self.qv.set_body(Body::Picture(c)),
            Loaded::Text(t) => self.qv.set_body(Body::Text(MultiEdit::new(&t))),
            Loaded::Nothing => {}
        }
    }
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
