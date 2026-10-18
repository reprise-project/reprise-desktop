//! A window frame's shape and state, apart from how any one look paints it: layout around the
//! client, hit test and caption buttons. `classic::frame` and `style::frame` turn this into pixels.

use crate::raster::{Canvas, Rect};
use crate::theme::Metrics;

/// Resize edge bitflags; values match xdg_toplevel's `resize_edge` so they pass through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edges(pub u32);

impl Edges {
    pub const NONE: Edges = Edges(0);
    pub const TOP: Edges = Edges(1);
    pub const BOTTOM: Edges = Edges(2);
    pub const LEFT: Edges = Edges(4);
    pub const RIGHT: Edges = Edges(8);

    pub fn contains(self, o: Edges) -> bool {
        self.0 & o.0 == o.0
    }

    pub fn intersects(self, o: Edges) -> bool {
        self.0 & o.0 != 0
    }

    pub fn cursor_name(self) -> &'static str {
        let t = self.contains(Edges::TOP);
        let b = self.contains(Edges::BOTTOM);
        let l = self.contains(Edges::LEFT);
        let r = self.contains(Edges::RIGHT);
        match (t, b, l, r) {
            (true, _, true, _) => "nw-resize",
            (true, _, _, true) => "ne-resize",
            (_, true, true, _) => "sw-resize",
            (_, true, _, true) => "se-resize",
            (true, _, _, _) => "n-resize",
            (_, true, _, _) => "s-resize",
            (_, _, true, _) => "w-resize",
            (_, _, _, true) => "e-resize",
            _ => "default",
        }
    }
}

impl std::ops::BitOr for Edges {
    type Output = Edges;
    fn bitor(self, o: Edges) -> Edges {
        Edges(self.0 | o.0)
    }
}

impl std::ops::BitOrAssign for Edges {
    fn bitor_assign(&mut self, o: Edges) {
        self.0 |= o.0;
    }
}

pub const ICON_AREA: i32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionButton {
    Minimize,
    Maximize,
    Close,
}

impl CaptionButton {
    pub const ALL: [CaptionButton; 3] = [
        CaptionButton::Minimize,
        CaptionButton::Maximize,
        CaptionButton::Close,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonSet {
    MinMaxClose,
    /// A windowed dialog's: minimize stands in the ? box's slot ([`FrameGeometry::help_box`]).
    MinClose,
    Close,
    None,
}

impl ButtonSet {
    pub fn has(self, b: CaptionButton) -> bool {
        match self {
            ButtonSet::MinMaxClose => true,
            ButtonSet::MinClose => b != CaptionButton::Maximize,
            ButtonSet::Close => b == CaptionButton::Close,
            ButtonSet::None => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub buttons: ButtonSet,
    pub resizable: bool,
    /// False when the client draws its own decorations (client-side xdg-decoration, or X11 Motif
    /// hints asking for no frame): no border or caption, and the frame buffer is never drawn.
    pub decorated: bool,
    pub icon: bool,
    pub help: bool,
}

impl Chrome {
    pub const WINDOW: Chrome = Chrome {
        buttons: ButtonSet::MinMaxClose,
        resizable: true,
        decorated: true,
        icon: true,
        help: false,
    };
    pub const DIALOG: Chrome = Chrome {
        buttons: ButtonSet::Close,
        resizable: false,
        decorated: true,
        icon: true,
        help: false,
    };
    pub const HELP_DIALOG: Chrome = Chrome {
        icon: false,
        help: true,
        ..Chrome::DIALOG
    };
    pub const PROMPT: Chrome = Chrome {
        icon: false,
        ..Chrome::DIALOG
    };
    pub const WINDOWED_DIALOG: Chrome = Chrome {
        buttons: ButtonSet::MinClose,
        ..Chrome::DIALOG
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameHit {
    Title,
    Icon,
    Button(CaptionButton),
    Help,
    Resize(Edges),
    Client,
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameGeometry {
    pub frame: i32,
    pub total: (i32, i32),
    pub origin: (i32, i32),
    pub caption: Rect,
    /// In `CaptionButton::ALL` order; rects the chrome lacks are laid out but never hit or drawn.
    pub buttons: [Rect; 3],
    /// The strip the three buttons are cut from when the metrics join them
    /// ([`Metrics::caption_group`]).
    pub group: Option<Rect>,
    pub client: Rect,
    pub chrome: Chrome,
    /// How many of the client's top rows the frame's glass runs down under (the client's blur
    /// region, `ext-background-effect-v1`); 0 for none. Only a look with [`Part::ClientGlass`]
    /// draws it.
    ///
    /// [`Part::ClientGlass`]: crate::style::Part::ClientGlass
    pub glass_top: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Glow(pub [f32; 3]);

impl Glow {
    pub fn lit(b: Option<CaptionButton>) -> Glow {
        let mut g = Glow::default();
        if let Some(b) = b {
            g.0[b as usize] = 1.0;
        }
        g
    }

    pub fn of(&self, b: CaptionButton) -> f32 {
        self.0[b as usize]
    }

    pub fn single(&self) -> Option<CaptionButton> {
        let mut lit = CaptionButton::ALL.into_iter().filter(|&b| self.of(b) > 0.0);
        match (lit.next(), lit.next()) {
            (Some(b), None) if self.of(b) >= 1.0 => Some(b),
            _ => None,
        }
    }

    pub fn is_dark(&self) -> bool {
        self.0.iter().all(|&l| l <= 0.0)
    }
}

pub struct FrameState<'a> {
    pub title: &'a str,
    pub active: bool,
    pub maximized: bool,
    pub pressed: Option<CaptionButton>,
    pub glow: Glow,
    pub icon: Option<&'a Canvas>,
    pub tabbed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct CaptionTab<'a> {
    pub title: &'a str,
    pub icon: Option<&'a Canvas>,
    pub selected: bool,
    pub ptr: crate::pointer::Interaction,
    pub ghost: bool,
    pub close: Option<(Rect, crate::pointer::Interaction)>,
    pub divider: bool,
}

fn insets(m: &Metrics, maximized: bool, chrome: Chrome) -> (i32, i32) {
    if !chrome.decorated {
        return (0, 0);
    }
    let border = if maximized {
        0
    } else if chrome.resizable {
        m.frame_width
    } else {
        m.dialog_frame_width
    };
    (border, m.caption_height)
}

impl FrameGeometry {
    pub fn layout(
        m: &Metrics,
        client_w: i32,
        client_h: i32,
        maximized: bool,
        chrome: Chrome,
    ) -> Self {
        let (f, ch) = insets(m, maximized, chrome);
        let total = (client_w + 2 * f, client_h + 2 * f + ch);
        let caption = Rect::new(f, f, client_w, ch);
        let client = Rect::new(f, f + ch, client_w, client_h);
        let (group, buttons) = match m.caption_group {
            Some(g) => {
                let (strip, cells) = g.layout(total.0, maximized);
                (Some(strip), cells)
            }
            None => (None, caption_buttons(m, caption, 2)),
        };
        let mut g = FrameGeometry {
            frame: f,
            total,
            origin: (0, 0),
            caption,
            buttons,
            group,
            client,
            chrome,
            glass_top: 0,
        };
        if chrome.buttons == ButtonSet::MinClose {
            g.buttons[0] = g.help_box();
        }
        g
    }

    /// This frame with its glass run `rows` down the client's top, at most the client's height.
    pub fn with_glass_top(self, rows: i32) -> FrameGeometry {
        FrameGeometry {
            glass_top: rows.clamp(0, self.client.height),
            ..self
        }
    }

    pub fn scaled(&self, n: i32) -> FrameGeometry {
        let n = n.max(1);
        FrameGeometry {
            frame: self.frame * n,
            total: (self.total.0 * n, self.total.1 * n),
            origin: (self.origin.0 * n, self.origin.1 * n),
            caption: self.caption.scaled(n),
            buttons: [
                self.buttons[0].scaled(n),
                self.buttons[1].scaled(n),
                self.buttons[2].scaled(n),
            ],
            group: self.group.map(|r| r.scaled(n)),
            client: self.client.scaled(n),
            chrome: self.chrome,
            glass_top: self.glass_top * n,
        }
    }

    /// The frame as four scene buffers around the client hole (caption band with borders, left,
    /// right, bottom), so a huge client surface never costs a matching raster.
    pub fn strips(&self) -> [Rect; 4] {
        let (w, h) = self.total;
        let f = self.frame;
        let top = self.client.y;
        let client_h = self.client.height;
        [
            Rect::new(0, 0, w, top),
            Rect::new(0, top, f, client_h),
            Rect::new(w - f, top, f, client_h),
            Rect::new(0, h - f, w, f),
        ]
    }

    pub fn translated(&self, x: i32, y: i32) -> FrameGeometry {
        let mv = |r: Rect| Rect::new(r.x - x, r.y - y, r.width, r.height);
        FrameGeometry {
            frame: self.frame,
            total: self.total,
            origin: (self.origin.0 - x, self.origin.1 - y),
            caption: mv(self.caption),
            buttons: [
                mv(self.buttons[0]),
                mv(self.buttons[1]),
                mv(self.buttons[2]),
            ],
            group: self.group.map(mv),
            client: mv(self.client),
            chrome: self.chrome,
            glass_top: self.glass_top,
        }
    }

    pub fn client_size_for_total(
        m: &Metrics,
        total_w: i32,
        total_h: i32,
        maximized: bool,
        chrome: Chrome,
    ) -> (i32, i32) {
        let (f, ch) = insets(m, maximized, chrome);
        ((total_w - 2 * f).max(1), (total_h - 2 * f - ch).max(1))
    }

    /// Where a dialog's ? box goes: the maximize slot, or with joined buttons a cell as wide as
    /// minimize's, its right divider the one close starts on.
    pub fn help_box(&self) -> Rect {
        let b = self.buttons[1];
        match self.group {
            Some(_) => {
                let [min, max, close] = self.buttons;
                let shared = min.right() - max.x;
                Rect::new(close.x + shared - min.width, b.y, min.width, b.height)
            }
            None => b,
        }
    }

    pub fn title_end(&self) -> i32 {
        if let Some(g) = self.group {
            return g.x;
        }
        CaptionButton::ALL
            .into_iter()
            .zip(self.buttons)
            .filter(|&(b, _)| self.chrome.buttons.has(b))
            .map(|(_, r)| r.x)
            .min()
            .unwrap_or(self.caption.right())
    }

    pub fn button_rect(&self, b: CaptionButton) -> Rect {
        self.buttons[CaptionButton::ALL.iter().position(|&k| k == b).unwrap()]
    }

    pub fn hit(&self, x: i32, y: i32, m: &Metrics) -> FrameHit {
        let (w, h) = self.total;
        if x < 0 || y < 0 || x >= w || y >= h {
            return FrameHit::Outside;
        }
        if self.client.contains(x, y) {
            return FrameHit::Client;
        }
        if !self.chrome.decorated {
            return FrameHit::Outside;
        }
        for (b, r) in CaptionButton::ALL.into_iter().zip(self.buttons) {
            if self.chrome.buttons.has(b) && r.contains(x, y) {
                return FrameHit::Button(b);
            }
        }
        if self.chrome.help && self.help_box().contains(x, y) {
            return FrameHit::Help;
        }
        if self.frame > 0 && self.chrome.resizable {
            // The resize band must cover at least the frame itself, else frame pixels beyond the
            // border would classify as title.
            let rb = m.resize_border.max(self.frame);
            let corner = m.caption_height;
            let mut e = Edges::NONE;
            if y < rb {
                e |= Edges::TOP;
            } else if y >= h - rb {
                e |= Edges::BOTTOM;
            }
            if x < rb {
                e |= Edges::LEFT;
            } else if x >= w - rb {
                e |= Edges::RIGHT;
            }
            if e != Edges::NONE {
                if e.intersects(Edges::LEFT | Edges::RIGHT) {
                    if y < corner {
                        e |= Edges::TOP;
                    } else if y >= h - corner {
                        e |= Edges::BOTTOM;
                    }
                }
                if e.intersects(Edges::TOP | Edges::BOTTOM) {
                    if x < corner {
                        e |= Edges::LEFT;
                    } else if x >= w - corner {
                        e |= Edges::RIGHT;
                    }
                }
                return FrameHit::Resize(e);
            }
        }
        if self.chrome.icon && self.caption.contains(x, y) && x < self.caption.x + ICON_AREA {
            return FrameHit::Icon;
        }
        FrameHit::Title
    }
}

/// The caption button sides a frame laid out with `m` draws, normal then maximized, each once:
/// what a style's [`crate::style::Style::warm_button_sides`] builds ahead.
pub fn caption_button_sides(m: &Metrics) -> Vec<i32> {
    let mut sides: Vec<i32> = [false, true]
        .map(|max| FrameGeometry::layout(m, 400, 300, max, Chrome::WINDOW).buttons[2].height)
        .into();
    sides.dedup();
    sides
}

pub fn caption_buttons(m: &Metrics, caption: Rect, gap: i32) -> [Rect; 3] {
    let bw = m.caption_button_width;
    let bh = m.caption_button_height;
    let by = caption.y + (caption.height - bh) / 2;
    let close_x = caption.right() - m.caption_button_right_margin - bw;
    let max_x = close_x - gap - bw;
    let min_x = max_x - m.caption_button_gap - bw;
    [
        Rect::new(min_x, by, bw, bh),
        Rect::new(max_x, by, bw, bh),
        Rect::new(close_x, by, bw, bh),
    ]
}

/// Canvas for a strip of `size` device px on layout `limit` px: the client picks its surface size,
/// so the strip is cut at layout.
pub fn frame_canvas_size(size: (i32, i32), limit: (i32, i32)) -> (u32, u32) {
    let w = size.0.min(limit.0.max(1)).max(1);
    let h = size.1.min(limit.1.max(1)).max(1);
    (w as u32, h as u32)
}

pub fn held_button(hit: FrameHit, pressed: CaptionButton) -> Option<CaptionButton> {
    (hit == FrameHit::Button(pressed)).then_some(pressed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Rect;
    use crate::theme::Metrics;

    #[test]
    fn dialog_close_box_is_one_px_further_in() {
        let m = Metrics::default();
        let win = FrameGeometry::layout(&m, 300, 200, false, Chrome::WINDOW);
        let dlg = FrameGeometry::layout(&m, 300, 200, false, Chrome::DIALOG);
        assert_eq!(
            win.button_rect(CaptionButton::Close),
            Rect::new(286, 6, 16, 14)
        );
        assert_eq!(
            dlg.button_rect(CaptionButton::Close),
            Rect::new(285, 5, 16, 14)
        );
        assert!(!dlg.chrome.buttons.has(CaptionButton::Minimize));
        assert!(!dlg.chrome.buttons.has(CaptionButton::Maximize));
        assert!(win.chrome.buttons.has(CaptionButton::Minimize));
    }

    #[test]
    fn dialog_chrome_hits_title_where_a_window_hits_minimize() {
        let m = Metrics::default();
        let win = FrameGeometry::layout(&m, 300, 200, false, Chrome::WINDOW);
        let dlg = FrameGeometry::layout(&m, 300, 200, false, Chrome::DIALOG);
        let b = win.button_rect(CaptionButton::Minimize);
        let (x, y) = (b.x + 1, b.y + 1);
        assert_eq!(win.hit(x, y, &m), FrameHit::Button(CaptionButton::Minimize));
        assert_eq!(dlg.hit(x, y, &m), FrameHit::Title);
        let c = dlg.button_rect(CaptionButton::Close);
        assert_eq!(
            dlg.hit(c.x + 1, c.y + 1, &m),
            FrameHit::Button(CaptionButton::Close)
        );
        let bare = Chrome {
            buttons: ButtonSet::None,
            ..Chrome::DIALOG
        };
        let bare = FrameGeometry::layout(&m, 300, 200, false, bare);
        assert_eq!(bare.hit(c.x + 1, c.y + 1, &m), FrameHit::Title);
    }

    #[test]
    fn a_help_dialog_caption_hits_its_help_box_and_has_no_icon() {
        let m = Metrics::default();
        let g = FrameGeometry::layout(&m, 300, 200, false, Chrome::HELP_DIALOG);
        let h = g.help_box();
        assert_eq!(
            h,
            g.button_rect(CaptionButton::Maximize),
            "the maximize slot"
        );
        assert_eq!(g.hit(h.x + 1, h.y + 1, &m), FrameHit::Help);
        let c = g.button_rect(CaptionButton::Close);
        assert_eq!(
            g.hit(c.x + 1, c.y + 1, &m),
            FrameHit::Button(CaptionButton::Close)
        );
        let (x, y) = (g.caption.x + 2, g.caption.y + 5);
        assert_eq!(g.hit(x, y, &m), FrameHit::Title, "no icon to open a menu");
        let with_icon = FrameGeometry::layout(&m, 300, 200, false, Chrome::DIALOG);
        assert_eq!(with_icon.hit(x, y, &m), FrameHit::Icon);
        assert_eq!(with_icon.hit(h.x + 1, h.y + 1, &m), FrameHit::Title);
    }

    #[test]
    fn a_fixed_size_frame_has_no_resize_band() {
        let m = Metrics::default();
        let win = FrameGeometry::layout(&m, 300, 200, false, Chrome::WINDOW);
        let dlg = FrameGeometry::layout(&m, 300, 200, false, Chrome::DIALOG);
        let y = dlg.total.1 / 2;
        assert!(matches!(win.hit(0, y, &m), FrameHit::Resize(_)));
        assert_eq!(dlg.hit(0, y, &m), FrameHit::Title);
    }

    #[test]
    fn border_width_follows_resizability_not_ownership() {
        let m = Metrics::default();
        let win = FrameGeometry::layout(&m, 300, 200, false, Chrome::WINDOW);
        let dlg = FrameGeometry::layout(&m, 300, 200, false, Chrome::DIALOG);
        assert_eq!(win.frame, 4);
        assert_eq!(dlg.frame, 3);
        let resizable_dialog = FrameGeometry::layout(
            &m,
            300,
            200,
            false,
            Chrome {
                resizable: true,
                ..Chrome::DIALOG
            },
        );
        assert_eq!(resizable_dialog.frame, 4);
    }

    #[test]
    fn layout_default_metrics() {
        let m = Metrics::default();
        let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
        assert_eq!(g.frame, 4);
        assert_eq!(g.total, (208, 126));
        assert_eq!(g.caption, Rect::new(4, 4, 200, 18));
        assert_eq!(g.client, Rect::new(4, 22, 200, 100));
        assert_eq!(
            g.button_rect(CaptionButton::Close),
            Rect::new(186, 6, 16, 14)
        );
        assert_eq!(
            g.button_rect(CaptionButton::Maximize),
            Rect::new(168, 6, 16, 14)
        );
        assert_eq!(
            g.button_rect(CaptionButton::Minimize),
            Rect::new(152, 6, 16, 14)
        );
        assert_eq!(
            FrameGeometry::client_size_for_total(&m, 208, 126, false, Chrome::WINDOW),
            (200, 100)
        );
    }

    #[test]
    fn undecorated_layout_is_all_client() {
        let m = Metrics::default();
        let c = Chrome {
            decorated: false,
            ..Chrome::WINDOW
        };
        let g = FrameGeometry::layout(&m, 200, 100, false, c);
        assert_eq!(g.frame, 0);
        assert_eq!(g.total, (200, 100));
        assert_eq!(g.client, Rect::new(0, 0, 200, 100));
        assert_eq!(g.caption.height, 0);
        assert_eq!(g.hit(0, 0, &m), FrameHit::Client);
        assert_eq!(g.hit(199, 99, &m), FrameHit::Client);
        assert_eq!(g.hit(200, 0, &m), FrameHit::Outside);
        assert_eq!(
            FrameGeometry::client_size_for_total(&m, 200, 100, false, c),
            (200, 100)
        );
    }

    #[test]
    fn layout_maximized_has_no_frame() {
        let m = Metrics::default();
        let g = FrameGeometry::layout(&m, 200, 100, true, Chrome::WINDOW);
        assert_eq!(g.frame, 0);
        assert_eq!(g.total, (200, 118));
        assert_eq!(g.caption, Rect::new(0, 0, 200, 18));
        assert_eq!(g.client, Rect::new(0, 18, 200, 100));
        assert_eq!(
            FrameGeometry::client_size_for_total(&m, 200, 118, true, Chrome::WINDOW),
            (200, 100)
        );
    }

    #[test]
    fn hit_testing() {
        let m = Metrics::default();
        let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
        assert_eq!(g.hit(100, 10, &m), FrameHit::Title);
        assert_eq!(g.hit(190, 10, &m), FrameHit::Button(CaptionButton::Close));
        assert_eq!(g.hit(100, 60, &m), FrameHit::Client);
        assert_eq!(g.hit(1, 60, &m), FrameHit::Resize(Edges::LEFT));
        assert_eq!(g.hit(206, 60, &m), FrameHit::Resize(Edges::RIGHT));
        assert_eq!(g.hit(100, 1, &m), FrameHit::Resize(Edges::TOP));
        assert_eq!(g.hit(100, 124, &m), FrameHit::Resize(Edges::BOTTOM));
        assert_eq!(g.hit(1, 1, &m), FrameHit::Resize(Edges::TOP | Edges::LEFT));
        assert_eq!(
            g.hit(206, 124, &m),
            FrameHit::Resize(Edges::BOTTOM | Edges::RIGHT)
        );
        assert_eq!(g.hit(1, 15, &m), FrameHit::Resize(Edges::TOP | Edges::LEFT));
        assert_eq!(g.hit(-1, 5, &m), FrameHit::Outside);
        assert_eq!(g.hit(300, 5, &m), FrameHit::Outside);
        let gm = FrameGeometry::layout(&m, 200, 100, true, Chrome::WINDOW);
        assert_eq!(gm.hit(0, 0, &m), FrameHit::Icon);
        assert_eq!(gm.hit(ICON_AREA - 1, 0, &m), FrameHit::Icon);
        assert_eq!(gm.hit(ICON_AREA, 0, &m), FrameHit::Title);
    }

    #[test]
    fn hit_resize_band_covers_frame_wider_than_resize_border() {
        let m = Metrics {
            frame_width: 8,
            resize_border: 4,
            ..Metrics::default()
        };
        let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
        assert_eq!(g.hit(6, 60, &m), FrameHit::Resize(Edges::LEFT));
        assert_eq!(g.hit(100, 12, &m), FrameHit::Title);
    }

    #[test]
    fn edges_or_together_and_intersect() {
        let mut e = Edges::TOP | Edges::LEFT;
        assert_eq!(e, Edges(5));
        e |= Edges::RIGHT;
        assert!(e.contains(Edges::TOP | Edges::RIGHT));
        assert!(e.intersects(Edges::BOTTOM | Edges::LEFT));
        assert!(!e.intersects(Edges::BOTTOM));
    }

    #[test]
    fn edge_cursor_names() {
        assert_eq!(Edges::TOP.cursor_name(), "n-resize");
        assert_eq!((Edges::TOP | Edges::LEFT).cursor_name(), "nw-resize");
        assert_eq!((Edges::BOTTOM | Edges::RIGHT).cursor_name(), "se-resize");
        assert_eq!(Edges::RIGHT.cursor_name(), "e-resize");
    }

    #[test]
    fn scaled_geometry_doubles_everything() {
        let m = Metrics::default();
        let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
        let s = g.scaled(2);
        assert_eq!(s.frame, 8);
        assert_eq!(s.total, (416, 252));
        assert_eq!(s.caption, Rect::new(8, 8, 400, 36));
        assert_eq!(s.client, Rect::new(8, 44, 400, 200));
        assert_eq!(s.buttons[2], Rect::new(372, 12, 32, 28));
    }

    #[test]
    fn frame_canvas_is_cut_at_the_layout() {
        assert_eq!(frame_canvas_size((800, 600), (1920, 1080)), (800, 600));
        assert_eq!(
            frame_canvas_size((16384, 16384), (1920, 1080)),
            (1920, 1080)
        );
        assert_eq!(frame_canvas_size((0, -3), (1920, 1080)), (1, 1));
        assert_eq!(frame_canvas_size((800, 600), (0, 0)), (1, 1));
    }

    #[test]
    fn caption_button_gap_spaces_minimize_from_maximize() {
        let m = Metrics {
            caption_button_gap: 2,
            caption_button_width: 21,
            caption_button_height: 21,
            ..Metrics::default()
        };
        let [min, max, close] = caption_buttons(&m, Rect::new(4, 4, 792, 26), 2);
        assert_eq!(max.x - min.right(), 2);
        assert_eq!(close.x - max.right(), 2);
        assert_eq!(close.right(), 4 + 792 - m.caption_button_right_margin);
    }

    fn joined() -> Metrics {
        use crate::theme::{CaptionGroup, GroupPlace};
        Metrics {
            caption_group: Some(CaptionGroup {
                widths: [10, 8, 12],
                divider: 1,
                restored: GroupPlace {
                    right: 3,
                    top: 1,
                    height: 9,
                },
                maximized: GroupPlace {
                    right: 1,
                    top: 0,
                    height: 8,
                },
            }),
            ..Metrics::default()
        }
    }

    #[test]
    fn joined_buttons_share_their_dividers_from_the_frames_corner() {
        let m = joined();
        let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
        assert_eq!(g.group, Some(Rect::new(177, 1, 28, 9)));
        assert_eq!(
            g.buttons,
            [
                Rect::new(177, 1, 10, 9),
                Rect::new(186, 1, 8, 9),
                Rect::new(193, 1, 12, 9),
            ]
        );
        let mx = FrameGeometry::layout(&m, 200, 100, true, Chrome::WINDOW);
        assert_eq!(mx.group, Some(Rect::new(171, 0, 28, 8)), "maximized");
        assert_eq!(
            mx.buttons[2],
            Rect::new(187, 0, 13, 8),
            "maximized, close reaches the frame's right edge"
        );
        let dlg = FrameGeometry::layout(&m, 200, 100, false, Chrome::DIALOG);
        assert_eq!(dlg.buttons[2], Rect::new(dlg.total.0 - 15, 1, 12, 9));
        assert_eq!(
            dlg.help_box(),
            Rect::new(dlg.total.0 - 24, 1, 10, 9),
            "the ? box as wide as minimize, sharing close's divider"
        );
        let plain = FrameGeometry::layout(&Metrics::default(), 200, 100, false, Chrome::DIALOG);
        assert_eq!(
            plain.help_box(),
            plain.buttons[1],
            "standalone: the maximize slot"
        );
        assert_eq!(
            FrameGeometry::layout(&Metrics::default(), 200, 100, false, Chrome::WINDOW).group,
            None,
            "buttons that stand alone have no strip"
        );
    }

    #[test]
    fn a_shared_divider_belongs_to_the_button_on_its_left() {
        let m = joined();
        let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
        let hit = |x| g.hit(x, 5, &m);
        assert_eq!(hit(177), FrameHit::Button(CaptionButton::Minimize));
        assert_eq!(
            hit(186),
            FrameHit::Button(CaptionButton::Minimize),
            "divider"
        );
        assert_eq!(hit(187), FrameHit::Button(CaptionButton::Maximize));
        assert_eq!(
            hit(193),
            FrameHit::Button(CaptionButton::Maximize),
            "divider"
        );
        assert_eq!(hit(194), FrameHit::Button(CaptionButton::Close));
        assert_eq!(hit(204), FrameHit::Button(CaptionButton::Close));
        assert_eq!(hit(176), FrameHit::Title);
        assert_eq!(g.hit(180, 0, &m), FrameHit::Resize(Edges::TOP));
        assert_eq!(g.hit(180, 10, &m), FrameHit::Title);
    }

    #[test]
    fn a_windowed_dialogs_minimize_box_takes_the_help_box_slot() {
        for m in [Metrics::default(), joined()] {
            let help = FrameGeometry::layout(&m, 200, 100, false, Chrome::HELP_DIALOG);
            let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOWED_DIALOG);
            let min = g.button_rect(CaptionButton::Minimize);
            assert_eq!(min, help.help_box(), "minimize where the ? box would be");
            assert_eq!(g.buttons[2], help.buttons[2], "close stays put");
            assert_eq!(
                g.hit(min.x, min.y, &m),
                FrameHit::Button(CaptionButton::Minimize)
            );
            assert_eq!(
                g.hit(g.buttons[2].right() - 1, g.buttons[2].y, &m),
                FrameHit::Button(CaptionButton::Close)
            );
            assert!(!g.chrome.buttons.has(CaptionButton::Maximize));
            assert_eq!(g.scaled(2).buttons[0], min.scaled(2));
        }
    }

    #[test]
    fn a_joined_strip_scales_and_moves_with_its_geometry() {
        let g = FrameGeometry::layout(&joined(), 200, 100, false, Chrome::WINDOW);
        assert_eq!(g.scaled(2).group, Some(Rect::new(354, 2, 56, 18)));
        assert_eq!(g.translated(0, 1).group, Some(Rect::new(177, 0, 28, 9)));
        assert_eq!(
            joined().scaled(2).caption_group.unwrap().divider,
            2,
            "the divider scales too"
        );
    }

    #[test]
    fn a_held_button_pops_up_off_it_and_back_in_on_it() {
        let close = CaptionButton::Close;
        assert_eq!(held_button(FrameHit::Button(close), close), Some(close));
        assert_eq!(held_button(FrameHit::Title, close), None);
        assert_eq!(
            held_button(FrameHit::Button(CaptionButton::Maximize), close),
            None
        );
        assert_eq!(held_button(FrameHit::Outside, close), None);
    }
}
