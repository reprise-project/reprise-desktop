//! Classic's task button preview panel: a cell per window with its title over a well. The host owns the pictures,
//! timers and scene; layout, hit tests and [`fit`] are the root [`crate::taskpreview`], re-exported here.
//! The media buttons a window with a media player gets under its well are drawn here for every look.

use crate::access::accesskit::{NodeId, Role};
use crate::access::{self, Tree};
use crate::art::glyph::Glyph;
use crate::classic::widgets::{CLOSE_H, CLOSE_W, draw_caption_button};
use crate::controls::volumepopup::{NowPlaying, Transport};
use crate::look::widgets::draw_media_button;
use crate::pointer::{Lit, lit_of};
use crate::raster::{Canvas, EdgeStyle, Insets, Rect};
use crate::taskpreview::{TITLE_GAP, TITLE_H, WELL_H};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

pub use crate::taskpreview::{
    CELL_H, CELL_W, Cell, Grid, PanelKey, TaskPreviewLayout, close_hit, fit, hit, key,
};

const PAD: i32 = 6;
const GAP: i32 = 6;
/// The title row's icon, 1 px in from the row's top-left, and the text
/// 4 px after it.
const ICON_PX: i32 = 16;
const TEXT_GAP: i32 = 4;

/// Classic's panel: 176x132 cells 6 px apart and 6 px in from the edge,
/// the well under the title row, the close button a caption close box at
/// the title row's right end, 2 px in. No shadow, square corners.
pub const CLASSIC: Grid = Grid {
    cell: (CELL_W, CELL_H),
    pad: PAD,
    gap: GAP,
    well: Rect::new(0, TITLE_H + TITLE_GAP, CELL_W, WELL_H),
    close: Rect::new(CELL_W - 2 - CLOSE_W, 2, CLOSE_W, CLOSE_H),
    media: None,
    shadow: Insets::ZERO,
    corner: &[],
};

/// A media button's width and the room between two, centred in the media row.
const MEDIA_W: i32 = 30;
const MEDIA_GAP: i32 = 4;

/// Cell `i`'s media button `t`, relative to the panel.
pub fn media_button(layout: &TaskPreviewLayout, i: usize, t: Transport) -> Option<Rect> {
    let row = layout.media(i)?;
    let s = layout.scale;
    let all = 3 * MEDIA_W + 2 * MEDIA_GAP;
    let x = row.x + (row.width - all * s) / 2 + t as i32 * (MEDIA_W + MEDIA_GAP) * s;
    Some(Rect::new(x, row.y, MEDIA_W * s, row.height))
}

/// The media button at `x`, `y` (relative to the panel) of a cell whose window has a player.
pub fn media_hit(
    layout: &TaskPreviewLayout,
    players: &[Option<NowPlaying>],
    x: i32,
    y: i32,
) -> Option<(usize, Transport)> {
    let i = hit(layout, x, y).filter(|&i| players.get(i).is_some_and(Option::is_some))?;
    let on = |t: &Transport| media_button(layout, i, *t).is_some_and(|r| r.contains(x, y));
    Transport::ALL.into_iter().find(on).map(|t| (i, t))
}

/// The media buttons' state: each cell's player (none for a window without one), the button the
/// pointer is on or holds, and the one with the keyboard focus.
#[derive(Clone, Copy, Debug, Default)]
pub struct Media<'a> {
    pub players: &'a [Option<NowPlaying>],
    pub lit: Lit<(usize, Transport)>,
    pub focus: Option<(usize, Transport)>,
}

/// Draw the media buttons of `media`'s cells on a panel drawn into `canvas` (sized to
/// `layout.canvas()`), greyed where the player can't do what they do.
pub fn draw_media(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &TaskPreviewLayout,
    media: &Media,
) {
    let p = theme.paint();
    let area = layout.canvas();
    for (i, np) in media.players.iter().enumerate() {
        let Some(np) = np else {
            continue;
        };
        for t in Transport::ALL {
            let Some(r) = media_button(layout, i, t) else {
                break;
            };
            let r = Rect::new(r.x - area.x, r.y - area.y, r.width, r.height);
            let (glyph, _) = np.glyph(t);
            let ptr = lit_of(media.lit, (i, t));
            let focus = media.focus == Some((i, t));
            let px = theme.fonts.size_px;
            draw_media_button(canvas, text, &p, r, glyph, np.can(t), ptr, focus, px);
        }
    }
}

/// The media button Tab (Shift+Tab with `back`) moves the keyboard to from `from` (None: the
/// cell): the next one `np` can press, None past the last, back on the cell.
pub fn media_step(np: &NowPlaying, from: Option<Transport>, back: bool) -> Option<Transport> {
    let order: Vec<Transport> = Transport::ALL.into_iter().filter(|&t| np.can(t)).collect();
    let at = from.and_then(|f| order.iter().position(|&t| t == f));
    match (at, back) {
        (None, false) => order.first().copied(),
        (None, true) => order.last().copied(),
        (Some(k), false) => order.get(k + 1).copied(),
        (Some(k), true) => k.checked_sub(1).map(|k| order[k]),
    }
}

/// Draw a sunken well with `picture` (an icon standing in for the live
/// one) centred in it.
pub fn draw_well(canvas: &mut Canvas, theme: &Theme, well: Rect, picture: Option<&Canvas>) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    canvas.fill_rect(well, cs.button_shadow);
    canvas.edge_t(well, EdgeStyle::ButtonSunken, cs, s);
    if let Some(p) = picture {
        let (w, h) = (p.width as i32, p.height as i32);
        canvas.blit_canvas(
            well.x + (well.width - w) / 2,
            well.y + (well.height - h) / 2,
            p,
        );
    }
}

/// Draw Classic's panel into `canvas` (sized to `layout.rect`, which is
/// in device pixels at `theme.scale`): a raised window edge, and per cell
/// the icon and title over its well; the `hot` cell's title row in the
/// highlight colours, a close box at its right end.
pub fn draw(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &TaskPreviewLayout,
    cells: &[Cell],
    hot: Option<usize>,
) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let all = Rect::new(0, 0, layout.rect.width, layout.rect.height);
    canvas.fill_rect(all, cs.button_face);
    canvas.edge_t(all, EdgeStyle::WindowRaised, cs, s);
    for (i, cell) in cells.iter().enumerate() {
        let (Some(row), Some(well)) = (layout.title_row(i), layout.well(i)) else {
            break;
        };
        let close = layout.close(i).filter(|_| hot == Some(i));
        let fg = if close.is_some() {
            canvas.fill_rect(row, cs.hilight);
            cs.hilight_text
        } else {
            cs.button_text
        };
        if let Some(icon) = cell.icon {
            canvas.blit_canvas(row.x + s, row.y + s, icon);
        }
        let tx = row.x + s + (ICON_PX + TEXT_GAP) * s;
        let end = close.map_or(row.right(), |c| c.x);
        let tw = end - 2 * s - tx;
        let title = text.ellipsize(cell.title, tw, cell.active, size);
        text.draw(
            canvas,
            Rect::new(tx, row.y, tw, row.height),
            &title,
            cell.active,
            fg,
            Align::Left,
            size,
        );
        draw_well(canvas, theme, well, cell.picture);
        if let Some(c) = close {
            draw_caption_button(canvas, cs, c, Glyph::Close, false, s);
        }
    }
}

/// The panel's screen reader nodes under `parent`, laid out from the
/// panel's top-left: a button per cell named for its window's title, in
/// the `hot` cell its close button, named "Close", and in a cell with a
/// player its media buttons. With `keys` the panel has the keyboard and
/// the hot cell, or `media`'s focused button, the focus.
pub fn access(
    layout: &TaskPreviewLayout,
    titles: &[&str],
    hot: Option<usize>,
    keys: bool,
    media: &Media,
    tree: &mut Tree,
    parent: NodeId,
) {
    for (i, (cell, title)) in layout.cells.iter().zip(titles).enumerate() {
        let id = tree.push(parent, i, access::named(Role::Button, *cell, title));
        if keys && hot == Some(i) && media.focus.is_none() {
            tree.set_focus(id);
        }
        if let Some(close) = layout.close(i).filter(|_| hot == Some(i)) {
            tree.push(
                id,
                "close",
                access::named(Role::Button, close, &tr!("close")),
            );
        }
        let Some(Some(np)) = media.players.get(i) else {
            continue;
        };
        for t in Transport::ALL {
            let Some(r) = media_button(layout, i, t) else {
                break;
            };
            let (_, name) = np.glyph(t);
            let b = tree.push(
                id,
                ("media", t as usize),
                access::button(r, &name, np.can(t), false),
            );
            if keys && media.focus == Some((i, t)) {
                tree.set_focus(b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taskbar::Edge;
    use crate::theme::FontConfig;

    #[test]
    fn access_names_each_cell_and_the_hot_cells_close_button() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        let mut tree = Tree::window("Previews", (l.rect.width, l.rect.height));
        access(
            &l,
            &["Notes", "Mail"],
            Some(1),
            false,
            &Media::default(),
            &mut tree,
            crate::access::ROOT,
        );
        let bounds = |label| {
            let id = tree.find(Role::Button, label).expect(label);
            tree.get(id).unwrap().bounds().unwrap()
        };
        assert_eq!(
            bounds("Notes"),
            access::bounds(l.cells[0]),
            "cell 0 is a button"
        );
        assert_eq!(
            bounds("Mail"),
            access::bounds(l.cells[1]),
            "cell 1 is a button"
        );
        assert_eq!(
            bounds("Close"),
            access::bounds(l.close(1).unwrap()),
            "the hot cell's close button"
        );
        let closes = tree
            .nodes()
            .filter(|(_, n)| n.label() == Some("Close"))
            .count();
        assert_eq!(closes, 1, "only the hot cell has a close button");
    }

    #[test]
    fn with_the_keyboard_the_hot_cell_holds_the_focus() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        let titles = ["Notes", "Mail"];
        let mut tree = Tree::window("Previews", (l.rect.width, l.rect.height));
        access(
            &l,
            &titles,
            Some(1),
            true,
            &Media::default(),
            &mut tree,
            crate::access::ROOT,
        );
        assert_eq!(tree.find(Role::Button, "Mail"), Some(tree.focus()));
        let mut tree = Tree::window("Previews", (l.rect.width, l.rect.height));
        access(
            &l,
            &titles,
            Some(1),
            false,
            &Media::default(),
            &mut tree,
            crate::access::ROOT,
        );
        assert_eq!(
            tree.focus(),
            crate::access::ROOT,
            "a hovered cell takes no focus"
        );
    }

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1052);

    fn playing() -> NowPlaying {
        NowPlaying {
            playing: true,
            can_previous: true,
            can_next: true,
            can_play: true,
            can_pause: true,
            ..NowPlaying::default()
        }
    }

    fn with_media(n: usize) -> TaskPreviewLayout {
        let button = Rect::new(300, 1054, 160, 22);
        TaskPreviewLayout::compute(CLASSIC.with_media(), n, button, SCREEN, Edge::Bottom)
    }

    #[test]
    fn the_media_row_puts_three_30_px_buttons_4_px_under_the_well() {
        let l = with_media(1);
        assert_eq!(l.cells[0].height, 132 + 28, "the cell is 28 px taller");
        assert_eq!(
            l.well(0),
            Some(Rect::new(6, 28, 176, 110)),
            "the well stays"
        );
        assert_eq!(l.media(0), Some(Rect::new(6, 142, 176, 24)));
        assert_eq!(
            Transport::ALL.map(|t| media_button(&l, 0, t).unwrap()),
            [
                Rect::new(45, 142, 30, 24),
                Rect::new(79, 142, 30, 24),
                Rect::new(113, 142, 30, 24),
            ],
            "98 px of buttons centred in the 176 px row"
        );
        assert_eq!(
            media_button(&l.scaled(2), 0, Transport::Next),
            Some(Rect::new(226, 284, 60, 48))
        );
        let plain = TaskPreviewLayout::compute(
            CLASSIC,
            1,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(plain.media(0), None, "no row without a player");
    }

    #[test]
    fn media_hit_takes_the_buttons_of_a_cell_with_a_player_only() {
        let l = with_media(2);
        let players = [None, Some(playing())];
        let next = media_button(&l, 1, Transport::Next).unwrap();
        assert_eq!(
            media_hit(&l, &players, next.x, next.y),
            Some((1, Transport::Next))
        );
        assert_eq!(
            media_hit(&l, &players, next.right(), next.y),
            None,
            "the gap after it"
        );
        let first = media_button(&l, 0, Transport::Next).unwrap();
        assert_eq!(
            media_hit(&l, &players, first.x, first.y),
            None,
            "a window without a player has no buttons"
        );
    }

    #[test]
    fn tab_steps_through_the_buttons_the_player_can_press_and_back_to_the_cell() {
        let np = NowPlaying {
            can_previous: false,
            ..playing()
        };
        use Transport::*;
        assert_eq!(media_step(&np, None, false), Some(PlayPause));
        assert_eq!(media_step(&np, Some(PlayPause), false), Some(Next));
        assert_eq!(media_step(&np, Some(Next), false), None, "back on the cell");
        assert_eq!(media_step(&np, None, true), Some(Next));
        assert_eq!(media_step(&np, Some(PlayPause), true), None);
        assert_eq!(media_step(&NowPlaying::default(), None, false), None);
    }

    #[test]
    fn access_names_the_media_buttons_by_what_they_do() {
        let l = with_media(2);
        let paused = NowPlaying {
            playing: false,
            can_next: false,
            ..playing()
        };
        let players = [Some(paused), None];
        let media = Media {
            players: &players,
            lit: None,
            focus: Some((0, Transport::PlayPause)),
        };
        let mut tree = Tree::window("Previews", (l.rect.width, l.rect.height));
        access(
            &l,
            &["Player", "Notes"],
            Some(0),
            true,
            &media,
            &mut tree,
            crate::access::ROOT,
        );
        let play = tree
            .find(Role::Button, "Play")
            .expect("Play, as it is paused");
        assert_eq!(tree.focus(), play, "the keyboard's button has the focus");
        let next = tree.find(Role::Button, "Next").unwrap();
        assert!(tree.get(next).unwrap().is_disabled(), "Next greys");
        assert_eq!(
            tree.nodes()
                .filter(|(_, n)| n.label() == Some("Previous"))
                .count(),
            1,
            "only the cell with a player has buttons"
        );
    }

    /// A playing window with the pointer on Next beside a paused one with Play focused, in each
    /// look at scale 1 and 2. The titles are empty so no system font draws.
    #[test]
    fn the_media_row_pins_in_each_look_at_scale_1_and_2() {
        use crate::interface::Interface;
        use crate::pointer::Interaction;
        crate::pins::fonts();
        let paused = NowPlaying {
            playing: false,
            can_previous: false,
            ..playing()
        };
        let players = [Some(playing()), Some(paused)];
        let media = Media {
            players: &players,
            lit: Some(((0, Transport::Next), Interaction::Hot)),
            focus: Some((1, Transport::PlayPause)),
        };
        let blank = || Cell {
            title: "",
            icon: None,
            active: false,
            picture: None,
        };
        let cells = [blank(), blank()];
        let mut got = Vec::new();
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let base = crate::interfaces::theme_for(look);
            let text = TextRenderer::load(&base.fonts);
            let grid =
                crate::interfaces::task_preview_grid(&base, &Default::default(), None).with_media();
            let button = Rect::new(300, 1040, 60, 40);
            let l = TaskPreviewLayout::compute(grid, 2, button, SCREEN, Edge::Bottom);
            for s in [1, 2] {
                let (theme, l) = (base.scaled(s), l.scaled(s));
                let a = l.canvas();
                let mut c = Canvas::new(a.width as u32, a.height as u32);
                crate::interfaces::draw_task_preview(
                    &mut c,
                    &theme,
                    &Default::default(),
                    &text,
                    &l,
                    &cells,
                    Some(0),
                    &media,
                );
                got.push((format!("{look:?} {s}"), crate::pins::fnv(&c)));
            }
        }
        crate::pins::check("controls/task_preview_media", &got);
    }

    #[test]
    fn one_window_centres_a_single_cell_over_its_button() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            1,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.rect, Rect::new(286, 908, 188, 144));
        assert_eq!(l.cells, [Rect::new(6, 6, 176, 132)]);
        assert_eq!(l.well(0), Some(Rect::new(6, 28, 176, 110)));
    }

    #[test]
    fn a_group_lays_cells_side_by_side() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            3,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.rect.width, 12 + 3 * 176 + 2 * 6);
        assert_eq!(l.cells[1].x, 6 + 182);
        assert_eq!(l.cells[2], Rect::new(6 + 2 * 182, 6, 176, 132));
    }

    #[test]
    fn a_panel_near_the_edge_is_pushed_inside() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            3,
            Rect::new(4, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.rect.x, 0);
        let r = TaskPreviewLayout::compute(
            CLASSIC,
            1,
            Rect::new(1850, 1054, 60, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(r.rect.right(), 1920);
    }

    #[test]
    fn more_cells_than_fit_wrap_into_rows() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            11,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.rect.width, 12 + 10 * 176 + 9 * 6);
        assert_eq!(l.rect.height, 12 + 2 * 132 + 6);
        assert_eq!(l.cells[10], Rect::new(6, 6 + 138, 176, 132));
    }

    #[test]
    fn hit_finds_the_cell_and_misses_the_gaps() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(hit(&l, 10, 10), Some(0));
        assert_eq!(hit(&l, 6 + 182 + 100, 100), Some(1));
        assert_eq!(hit(&l, 6 + 177, 50), None);
        assert_eq!(hit(&l, 2, 2), None);
    }

    #[test]
    fn draw_highlights_the_hot_title_and_sinks_the_wells() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        let mut c = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        let red = crate::theme::Color::rgb(255, 0, 0);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), red);
        let cells = [
            Cell {
                title: "one",
                icon: None,
                active: true,
                picture: None,
            },
            Cell {
                title: "two",
                icon: None,
                active: false,
                picture: Some(&icon),
            },
        ];
        draw(&mut c, &theme, &text, &l, &cells, Some(1));
        let cs = &theme.colors;
        assert_eq!(c.get(0, 10), cs.button_face.argb());
        assert_eq!(c.get(1, 10), cs.button_hilight.argb());
        assert_eq!(c.get(l.rect.width - 1, 10), cs.button_dk_shadow.argb());
        let (a, b) = (l.cells[0], l.cells[1]);
        assert_eq!(c.get(b.right() - 2, b.y + 2), cs.hilight.argb());
        assert_eq!(c.get(a.right() - 2, a.y + 2), cs.button_face.argb());
        let w = l.well(0).unwrap();
        assert_eq!(c.get(w.x, w.y + 20), cs.button_shadow.argb());
        assert_eq!(c.get(w.x + 1, w.y + 20), cs.button_dk_shadow.argb());
        assert_eq!(c.get(w.right() - 1, w.y + 20), cs.button_hilight.argb());
        assert_eq!(c.get(w.x + 10, w.y + 10), cs.button_shadow.argb());
        let w1 = l.well(1).unwrap();
        assert_eq!(c.get(w1.x + 88, w1.y + 55), red.argb());
        assert_eq!(c.get(w1.x + 88 - 17, w1.y + 55), cs.button_shadow.argb());
        let x = l.close(1).unwrap();
        assert_eq!(
            c.get(x.right() - 1, x.y + 5),
            cs.button_dk_shadow.argb(),
            "the close box's right edge is dark shadow"
        );
        let x0 = l.close(0).unwrap();
        assert_eq!(
            c.get(x0.right() - 1, x0.y + 5),
            cs.button_face.argb(),
            "a cold cell's title row stays face where a close box would be"
        );
    }

    #[test]
    fn the_close_box_sits_at_the_title_rows_right_end() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.close(0), Some(Rect::new(6 + 176 - 18, 8, 16, 14)));
        assert_eq!(l.close(1), Some(Rect::new(6 + 182 + 176 - 18, 8, 16, 14)));
        assert_eq!(l.scaled(2).close(0), Some(Rect::new(2 * 164, 16, 32, 28)));
    }

    #[test]
    fn at_200_percent_text_the_title_row_grows_by_the_line_and_the_well_moves_down() {
        use crate::interface::Interface;
        use crate::interfaces::{task_preview_grid, theme_for};
        for look in [Interface::Classic, Interface::Blue] {
            let theme = theme_for(look).with_text_size(200);
            let g = theme.line_growth();
            assert!(g > 0, "{look:?}");
            let button = Rect::new(300, 1054, 160, 22);
            let l = TaskPreviewLayout::compute(
                task_preview_grid(&theme, &Default::default(), None),
                1,
                button,
                SCREEN,
                Edge::Bottom,
            );
            let (row, well, close) = (
                l.title_row(0).unwrap(),
                l.well(0).unwrap(),
                l.close(0).unwrap(),
            );
            assert_eq!(row.height, TITLE_H + g, "{look:?}");
            assert_eq!(well.y, row.bottom() + TITLE_GAP, "{look:?}");
            assert_eq!(well.height, WELL_H, "{look:?}");
            assert_eq!(l.cells[0].height, CELL_H + g, "{look:?}");
            let (above, below) = (close.y - row.y, row.bottom() - close.bottom());
            assert!(
                (above - below).abs() <= 1,
                "{look:?}: the close box centred in the row"
            );
            let base = theme_for(look);
            assert_eq!(
                task_preview_grid(&base, &Default::default(), None),
                CLASSIC,
                "{look:?} at 100%"
            );
        }
        let glass = theme_for(Interface::Glass).with_text_size(200);
        assert_eq!(
            task_preview_grid(&glass, &Default::default(), None),
            crate::glass::preview::GRID,
            "Glass keeps its own grid"
        );
    }

    #[test]
    fn a_classic_panel_takes_the_pointer_everywhere_on_it() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.input_area(), Rect::new(0, 0, l.rect.width, l.rect.height));
    }

    #[test]
    fn close_hit_finds_the_close_box_only() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        let x = l.close(1).unwrap();
        let hot = Some(1);
        assert_eq!(close_hit(&l, hot, x.x, x.y), Some(1), "its top-left pixel");
        assert_eq!(close_hit(&l, hot, x.right() - 1, x.bottom() - 1), Some(1));
        assert_eq!(
            close_hit(&l, None, x.x, x.y),
            None,
            "no cell hot: no close box drawn, none hit"
        );
        assert_eq!(
            close_hit(&l, Some(0), x.x, x.y),
            None,
            "another cell hot: this cell's close box is not drawn"
        );
        assert_eq!(
            close_hit(&l, hot, x.x - 1, x.y + 5),
            None,
            "the title left of it"
        );
        assert_eq!(
            close_hit(&l, hot, x.x + 5, x.bottom()),
            None,
            "the well under it"
        );
    }
}
