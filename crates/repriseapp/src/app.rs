//! What a program implements: the [`App`] trait and the windows it names.

use crate::{Ctx, DropForm, Event, FileDrop, Reader, ReaderForm, Reply, Waker};
use reprisebus::mutex::lock;
use reprisekit::access::Tree;
use reprisekit::controls::hosted::Hosted;
use reprisekit::raster::{Canvas, Rect};

pub trait App {
    fn title(&self) -> String;
    fn app_id(&self) -> String;
    fn initial_size(&self) -> (i32, i32);
    /// Whether the main window refuses to resize, like an About box.
    /// Its min and max size are pinned to `initial_size`, and to each later `Reply::size`.
    fn fixed(&self) -> bool {
        false
    }
    fn children(&self) -> Vec<Child> {
        Vec::new()
    }
    /// The window child `win` sits over. `None` makes it a toplevel of its own, unparented like
    /// the main window, for a program that shows several (its `modal` is then ignored).
    fn parent(&self, _win: WinId) -> Option<WinId> {
        Some(MAIN)
    }
    /// The window of this program whose caption tab group unparented child `win` joins as it
    /// maps, right after it. Read when its window is made; needs [`tab_groups`](crate::tab_groups).
    fn tab_with(&self, _win: WinId) -> Option<WinId> {
        None
    }
    /// The smallest client size resizable window `win` can be dragged
    /// to, read when its window is made; `None` keeps a child's wanted
    /// size, or `MIN_SIZE` for the main window, as the floor.
    fn min_size(&self, _win: WinId) -> Option<(i32, i32)> {
        None
    }
    /// The flyout the program has open right now, if any. The host
    /// reconciles it after every event, like `children`.
    fn flyout(&self) -> Option<Flyout> {
        None
    }
    /// Whether the main window has no surface at all (a program that lives
    /// in the tray). The program answers a `CloseRequest` on `MAIN` with
    /// `Reply::redraw()` and returns `true` again, rather than quitting.
    /// While hidden, dialogs are ordinary windows, not children of a parent.
    fn hidden(&self) -> bool {
        false
    }
    /// Another program's window this one's main window belongs to, as
    /// the handle it exported through xdg-foreign (a portal's
    /// `parent_window` without its `wayland:`). The compositor then
    /// centres the main window on that one and keeps it off the taskbar.
    /// A handle the compositor does not know leaves the window ownerless.
    fn foreign_parent(&self) -> Option<String> {
        None
    }
    /// Whether the main window, owned through `foreign_parent`, blocks
    /// its owner until it closes (xdg-dialog-v1 `set_modal`).
    fn modal(&self) -> bool {
        false
    }
    /// The caption `win` wears: a window's by default, a prompt's for a
    /// message box, a help dialog's for a sheet or chooser, a fixed dialog's
    /// for a program that is one dialog. Read after every event; the host
    /// sends it through xdg-toplevel-tag.
    fn caption(&self, _win: WinId) -> Caption {
        Caption::Window
    }
    /// The window that takes the compositor's shortcuts while it has the
    /// keyboard (Super, Alt+Tab and every bind reach it as keys), such as
    /// a dialog whose hot-key field has the focus. Read after every
    /// event; the host holds a keyboard-shortcuts-inhibit on it.
    fn shortcuts_inhibited(&self) -> Option<WinId> {
        None
    }
    /// Called once before the first dispatch, with the handle the
    /// program clones into the threads it spawns.
    fn start(&mut self, _waker: Waker) {}
    /// Whether the program sets the outputs: the host then binds
    /// wlr-output-management, sends `Event::Heads` and carries out
    /// [`Ctx::configure_head`]. Read once, at start.
    fn output_management(&self) -> bool {
        false
    }
    /// Whether [`Ctx::theme`] is at the text size: the program lays its own lists, trees, menus and
    /// bars out with [`Theme::controls`](reprisekit::theme::Theme::controls), so they grow with the
    /// text. Read at start and on every look change.
    fn text_sized(&self) -> bool {
        false
    }
    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply;
    /// Called before each draw of `win`, at `scale` device px per logical px, with
    /// [`Ctx::icons`] at that scale: a program fetches again what it holds at another one.
    fn follow_scale(&mut self, _win: WinId, _scale: i32, _ctx: &mut Ctx) {}
    /// The image of the drag window `win` started ([`Reply::start_drag`]), drawn again at
    /// `scale` for an output of another scale, with [`Ctx::icons`] at that scale. None keeps the
    /// image it has.
    fn drag_image(&mut self, _win: WinId, _scale: i32, _ctx: &mut Ctx) -> Option<Canvas> {
        None
    }
    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx);
    /// Whether `draw` gets `win`'s last frame back (until a resize or scale change) and reports
    /// what it redrew with [`Canvas::report_damage`].
    fn keeps_canvas(&self, _win: WinId) -> bool {
        false
    }
    /// Window `win`'s accessibility tree, from the state `draw` last drew:
    /// hang the controls' `access` parts under `reprisekit::access::ROOT` and
    /// set the focus. Called only while a screen reader listens. Without it
    /// the window is an empty, titled one.
    fn access(&mut self, _win: WinId, _tree: &mut Tree, _ctx: &Ctx) {}
    /// The form in window `win` whose single-line edit fields take a drag of text or files
    /// ([`fielddrop`](crate::fielddrop)); its drags then never reach `event`. None (the default)
    /// hands them to `event`.
    fn drop_form(&mut self, _win: WinId) -> Option<DropForm<'_>> {
        None
    }
    /// The menu or drop list window `win` has open, which the host shows on popups of their own
    /// ([`reprisekit::controls::hosted`]): the program keeps it and gets its input as the
    /// window's, laid out where the compositor put each level. Asked before each draw. Defaults
    /// to [`reader_popup`].
    fn popup(&mut self, win: WinId, ctx: &Ctx) -> Option<Box<dyn Hosted + '_>> {
        reader_popup(self, win, ctx)
    }
    /// The form in window `win` that a screen reader's Focus and ScrollIntoView reach without a
    /// click: a row scrolls into view, a button takes the focus unpressed. Defaults to
    /// [`App::drop_form`].
    fn reader_form(&mut self, win: WinId) -> Option<ReaderForm<'_>> {
        self.drop_form(win).map(ReaderForm::Form)
    }
    /// Carry out a screen reader's Focus or ScrollIntoView in window `win` that takes no click;
    /// true when the window changed and is drawn again. Defaults to [`reader_forms`]; a window
    /// drawn from other controls answers it here.
    fn reader(&mut self, win: WinId, act: Reader, ctx: &Ctx) -> bool {
        reader_forms(self, win, act, ctx)
    }
    /// The file dialog in window `win`, whose File name field takes a drag of text or files as
    /// a form's field does; files go in by name, or by path from another folder. None (the
    /// default) hands the drags to `event`. The host also decodes its pictures for Thumbnails
    /// and Tiles and redraws the window as they arrive.
    fn drop_file_dialog(&mut self, _win: WinId) -> Option<FileDrop<'_>> {
        None
    }
}

/// The open popup of [`App::reader_form`]'s form or sheet, else the open list or Views menu of
/// [`App::drop_file_dialog`].
pub fn reader_popup<'a, A: App + ?Sized>(
    app: &'a mut A,
    win: WinId,
    ctx: &Ctx,
) -> Option<Box<dyn Hosted + 'a>> {
    if app.reader_form(win).is_some() {
        return match app.reader_form(win)? {
            ReaderForm::Form(d) => Some(Box::new(d.form)),
            ReaderForm::Sheet(s) => Some(Box::new(s)),
        };
    }
    let FileDrop { dlg, .. } = app.drop_file_dialog(win)?;
    Some(Box::new(reprisekit::controls::filedlg::Popped {
        dlg,
        text: ctx.text.clone(),
        px: ctx.form_units().px(ctx.base_px()),
    }))
}

/// A screen reader's `act` carried out on window `win`'s [`App::reader_form`], else its
/// [`App::drop_file_dialog`]; what [`App::reader`] does unless a program answers it.
pub fn reader_forms<A: App + ?Sized>(app: &mut A, win: WinId, act: Reader, ctx: &Ctx) -> bool {
    let base = ctx.base_px();
    if let Some(f) = app.reader_form(win) {
        return f.act(act, &ctx.text, base);
    }
    let px = ctx.form_units().px(base);
    let Some(FileDrop { dlg, .. }) = app.drop_file_dialog(win) else {
        return false;
    };
    let (w, h) = dlg.client();
    let r = Rect::new(0, 0, w, h);
    let ev = match act {
        Reader::Reveal { at, row } => dlg.reveal(&ctx.text, px, r, at, row),
        Reader::Focus(at) => dlg.focus_at(&ctx.text, px, r, at),
    };
    !matches!(ev, reprisekit::controls::filedlg::Event::None)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Caption {
    #[default]
    Window,
    Prompt,
    Help,
    Fixed,
}

/// Ids are fixed strings, so a program has at most one window per id.
pub type WinId = &'static str;

pub const MAIN: WinId = "main";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    pub id: WinId,
    pub title: String,
    pub size: (i32, i32),
    /// Fixed size: min and max are pinned to `size`, so the compositor
    /// gives it no resize band.
    pub fixed: bool,
    pub modal: bool,
}

/// A panel popped up over everything at a point on screen: the tray's
/// flyout. `at` is in global layout coordinates (what an SNI `Activate`
/// carries). The host gives it an overlay layer surface covering the
/// whole output that holds `at`, taskbar included, with the keyboard; the
/// program draws its panel into that canvas (`Ctx::origin` is the
/// output's top-left) and leaves the rest transparent, so a press there
/// is a click outside. An `at` on no output, such as
/// [`Flyout::UNDER_POINTER`], leaves the output to the compositor, which
/// takes the one under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flyout {
    pub id: WinId,
    pub at: (i32, i32),
    /// What a screen reader calls the flyout's window.
    pub title: &'static str,
}

impl Flyout {
    pub const UNDER_POINTER: (i32, i32) = (i32::MIN, i32::MIN);
}

/// The id `window-<n>`, one `'static` str per `n` for the life of the process: for a program
/// that opens windows of its own as it runs ([`App::parent`]).
pub fn window_id(n: usize) -> WinId {
    use std::sync::Mutex;
    static IDS: Mutex<Vec<WinId>> = Mutex::new(Vec::new());
    let mut ids = lock(&IDS);
    while ids.len() <= n {
        let next = format!("window-{}", ids.len());
        ids.push(Box::leak(next.into_boxed_str()));
    }
    ids[n]
}
