use super::*;
use crate::raster::Canvas;
use crate::text::{Align, split_mnemonic};
use crate::theme::TextUnits;
use reprisekit::blue::powerpanel::TileState;
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, draw_label};
use reprisekit::controls::form::Key;
use reprisekit::cues::Cues;
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_button;
use reprisekit::pointer::{Interaction, Lit};
use reprisekit::powerpanel::{self, Event, Part, Updates};

impl DialogLayout {
    /// The Blue panel: `powerpanel`'s layout, with its cells, Cancel, Restart Into... and the Install
    /// updates check box as the buttons.
    pub(super) fn compute_welcome(w: &Welcome, u: TextUnits) -> DialogLayout {
        let lay = powerpanel::layout(&w.panel, u);
        let mut buttons = lay.cells.clone();
        buttons.push(lay.cancel);
        buttons.extend(lay.restart_into);
        buttons.extend(lay.updates);
        DialogLayout {
            size: lay.size,
            caption: lay.title,
            buttons,
            power: Some(lay),
            ..Default::default()
        }
    }

    /// The Welcome panel under a look with glass (`GLASS_WELCOME_W` on):
    /// the icon buttons in a centred row, the rest (Cancel, Restart Into...) from the right of the
    /// row under them, the Install updates check box from its left up to them.
    pub(super) fn compute_welcome_glass(d: &Dialog) -> DialogLayout {
        let (x0, y0) = (DIALOG_FRAME, DIALOG_FRAME + CAPTION_H);
        let (cw, ch) = GLASS_WELCOME_CELL;
        let icons = d.buttons.iter().filter(|x| x.choice.is_some()).count() as i32;
        let row = icons * cw + (icons - 1).max(0) * GLASS_WELCOME_GAP;
        let left = x0 + (GLASS_WELCOME_W - row) / 2;
        let plain_y = y0 + GLASS_WELCOME_TOP + ch + GLASS_WELCOME_TOP;
        let mut next_icon = 0;
        let mut next_plain = 0;
        let plains = (d.buttons.iter())
            .filter(|x| x.choice.is_none() && x.action != DialogAction::ToggleUpdates)
            .count() as i32;
        let buttons = d
            .buttons
            .iter()
            .map(|x| {
                if x.action == DialogAction::ToggleUpdates {
                    let left = x0 + GLASS_WELCOME_MARGIN;
                    let right = x0 + GLASS_WELCOME_W
                        - GLASS_WELCOME_MARGIN
                        - plains * (BUTTON_W + BUTTON_GAP);
                    Rect::new(left, plain_y, right - left, BUTTON_H)
                } else if x.choice.is_some() {
                    let r = Rect::new(
                        left + next_icon * (cw + GLASS_WELCOME_GAP),
                        y0 + GLASS_WELCOME_TOP,
                        cw,
                        ch,
                    );
                    next_icon += 1;
                    r
                } else {
                    let r = Rect::new(
                        x0 + GLASS_WELCOME_W
                            - GLASS_WELCOME_MARGIN
                            - BUTTON_W
                            - next_plain * (BUTTON_W + BUTTON_GAP),
                        plain_y,
                        BUTTON_W,
                        BUTTON_H,
                    );
                    next_plain += 1;
                    r
                }
            })
            .collect();
        let client_h = plain_y - y0 + BUTTON_H + GLASS_WELCOME_MARGIN;
        DialogLayout {
            size: (
                GLASS_WELCOME_W + 2 * DIALOG_FRAME,
                client_h + 2 * DIALOG_FRAME + CAPTION_H,
            ),
            buttons,
            ..Default::default()
        }
    }
}

/// The icon and label offsets in a framed panel's icon button, at 100%.
pub(super) const WELCOME_ICON_Y: i32 = 8;

const WELCOME_LABEL_Y: i32 = 44;

/// The Welcome panel under a look with glass, a dialog like any other
/// in its frame: a 312 px wide client, the icon buttons 80x64 push
/// buttons 12 px apart, centred, 14 px under the client's top, each with
/// its icon 8 px and its label 44 px into it; Cancel a standard button
/// 11 px in from the client's right and bottom edges, 14 px under them.
pub(super) const GLASS_WELCOME_W: i32 = 312;

pub(super) const GLASS_WELCOME_TOP: i32 = 14;

pub(super) const GLASS_WELCOME_CELL: (i32, i32) = (80, 64);

const GLASS_WELCOME_GAP: i32 = 12;

pub(super) const GLASS_WELCOME_MARGIN: i32 = 11;

/// What a Welcome panel's button `i` is: its cells, Cancel, Restart Into... when offered, which the
/// framed layout puts left of Cancel, then the check box.
fn part_of(w: &Welcome, i: usize) -> Part {
    let n = w.panel.bigs.len();
    let into = usize::from(w.panel.restart_into);
    match i {
        i if i < n => Part::Big(i),
        i if i == n => Part::Cancel,
        i if i == n + into => Part::RestartInto,
        _ => Part::Updates,
    }
}

fn index_of(w: &Welcome, part: Part) -> usize {
    match part {
        Part::Big(i) => i,
        Part::Cancel => w.panel.bigs.len(),
        Part::RestartInto => w.panel.bigs.len() + 1,
        Part::Updates => w.panel.bigs.len() + 1 + usize::from(w.panel.restart_into),
    }
}

/// What a big button does with the Install updates check box shown: Turn Off and Restart install
/// the update, or take back one set elsewhere, as the box says
/// ([`Panel::updates`](powerpanel::Panel::updates)).
fn with_updates(action: DialogAction, updates: Updates) -> DialogAction {
    match (action, updates) {
        (a, Updates::Leave) => a,
        (DialogAction::EndSession(goal, cmd), updates) if goal != Goal::LogOff => {
            DialogAction::EndSessionUpdates { goal, cmd, updates }
        }
        (a, _) => a,
    }
}

/// The buttons `w`'s panel shows: each big button as it shows now, Cancel, Restart Into... when
/// offered, greyed while the box is checked, then the Install updates check box while it shows.
/// The focused one is the default.
fn buttons_of(w: &Welcome) -> Vec<DialogButton> {
    let p = &w.panel;
    let mut out: Vec<DialogButton> = (0..p.bigs.len())
        .filter_map(|i| p.shown(i).map(|shown| (i, shown)))
        .map(|(i, (choice, on))| DialogButton {
            label: choice.label(),
            action: w
                .actions
                .iter()
                .find(|(c, _)| *c == choice)
                .map_or(DialogAction::Cancel, |(_, a)| {
                    with_updates(a.clone(), p.updates())
                }),
            default: p.focus == Part::Big(i),
            choice: Some(choice),
            enabled: on,
            tip: on.then(|| choice.tip()),
        })
        .collect();
    out.push(super::build::button(
        &reprisekit::kit_tr!("cancel"),
        DialogAction::Cancel,
        p.focus == Part::Cancel,
    ));
    if p.restart_into {
        out.push(DialogButton {
            enabled: p.live(Part::RestartInto),
            ..super::build::button(
                &powerpanel::restart_into_label(),
                DialogAction::RestartInto,
                p.focus == Part::RestartInto,
            )
        });
    }
    if p.updates.is_some() {
        out.push(super::build::button(
            &powerpanel::updates_label(),
            DialogAction::ToggleUpdates,
            p.focus == Part::Updates,
        ));
    }
    out
}

impl Dialog {
    /// Lay `d.buttons` out again from the Welcome panel's model.
    pub(super) fn sync_welcome(&mut self) {
        if let Style::Welcome(w) = &self.style {
            self.buttons = buttons_of(w);
        }
    }

    /// A key for a Welcome panel, which its model answers.
    pub(super) fn welcome_key(&mut self, key: &Key) -> Outcome {
        use crate::textedit::EditKey;
        let Style::Welcome(w) = &mut self.style else {
            return Outcome::Nothing;
        };
        let k = match key {
            Key::Left | Key::Edit(EditKey::Left { .. }) => powerpanel::Key::Left,
            Key::Right | Key::Edit(EditKey::Right { .. }) => powerpanel::Key::Right,
            Key::Up => powerpanel::Key::Up,
            Key::Down => powerpanel::Key::Down,
            Key::Tab { back } => powerpanel::Key::Tab { back: *back },
            Key::Enter => powerpanel::Key::Enter,
            Key::Escape => powerpanel::Key::Escape,
            Key::Space => powerpanel::Key::Space,
            Key::Char(c) | Key::Access(c) => powerpanel::Key::Char(*c),
            Key::Edit(EditKey::Insert(s)) => {
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(' '), None) => powerpanel::Key::Space,
                    (Some(c), None) => powerpanel::Key::Char(c),
                    _ => return Outcome::Nothing,
                }
            }
            _ => return Outcome::Nothing,
        };
        match w.panel.key(k) {
            Event::None => Outcome::Nothing,
            Event::Redraw => {
                self.sync_welcome();
                Outcome::Redraw
            }
            Event::Press(part) => {
                let i = index_of(w, part);
                match self.press(i) {
                    Outcome::RestartInto { .. } => Outcome::RestartInto { keys: true },
                    out => out,
                }
            }
        }
    }

    /// Button `i` of a Welcome panel takes the focus.
    pub(super) fn welcome_focus(&mut self, i: usize) -> Outcome {
        let Style::Welcome(w) = &mut self.style else {
            return Outcome::Nothing;
        };
        let part = part_of(w, i);
        if w.panel.focus_on(part) == Event::None {
            return Outcome::Nothing;
        }
        self.sync_welcome();
        Outcome::Redraw
    }

    /// Show or drop a Turn off computer panel's Restart Into... (`on`); true when it changed. Its
    /// focus moves to Cancel when it goes.
    pub fn offer_restart_into(&mut self, on: bool) -> bool {
        let Style::Welcome(w) = &mut self.style else {
            return false;
        };
        let restarts = (w.actions.iter()).any(|(c, a)| {
            *c == Choice::Restart && matches!(a, DialogAction::EndSession(Goal::Restart, _))
        });
        if !restarts || w.panel.restart_into == on {
            return false;
        }
        w.panel.restart_into = on;
        if !w.panel.live(w.panel.focus) {
            w.panel.focus = Part::Cancel;
        }
        self.sync_welcome();
        true
    }

    /// Whether an offline update is `prepared` changed what a Turn off computer panel shows: the
    /// Install updates check box came or went, checked only when it shows from the panel's first
    /// frame (`late` false). The dialog then wants laying out again.
    pub fn offer_updates(&mut self, prepared: bool, late: bool) -> bool {
        let Style::Welcome(w) = &mut self.style else {
            return false;
        };
        if !w.updates || w.panel.offer_updates(prepared, !late) == Event::None {
            return false;
        }
        self.sync_welcome();
        true
    }

    /// What the Install updates check box of a Turn off computer panel says to do; Leave for any
    /// other dialog.
    pub fn updates(&self) -> Updates {
        match &self.style {
            Style::Welcome(w) => w.panel.updates(),
            _ => Updates::Leave,
        }
    }

    /// The Install updates check box was clicked.
    pub(super) fn welcome_toggle_updates(&mut self) -> Outcome {
        let Style::Welcome(w) = &mut self.style else {
            return Outcome::Nothing;
        };
        if w.panel.toggle_updates() == Event::None {
            return Outcome::Nothing;
        }
        self.sync_welcome();
        Outcome::Redraw
    }

    /// Shift went down or up while a Welcome panel is open: Stand By turns into Hibernate and back.
    pub fn welcome_shift(&mut self, on: bool) -> Outcome {
        let Style::Welcome(w) = &mut self.style else {
            return Outcome::Nothing;
        };
        if w.panel.set_shift(on) == Event::None {
            return Outcome::Nothing;
        }
        self.sync_welcome();
        Outcome::Redraw
    }
}

/// The Blue panel in `theme`'s Blue scheme (navy outside Blue), on a canvas at `theme.scale`, with
/// Cancel in `theme`'s look unless the panel is always Blue.
pub(super) fn draw_welcome(
    theme: &Theme,
    text: &TextRenderer,
    lay: &powerpanel::Layout,
    w: &Welcome,
    lit: Lit<usize>,
    cues: Cues,
) -> Canvas {
    let s = theme.scale.max(1);
    let (cw, ch) = (lay.size.0 * s, lay.size.1 * s);
    let mut c = Canvas::new(cw.max(1) as u32, ch.max(1) as u32);
    let lit = lit.map(|(i, how)| (part_of(w, i), how));
    let cancel = (!w.always_blue).then(|| theme.paint());
    reprisekit::interfaces::draw_power_panel(
        &mut c,
        text,
        reprisekit::interfaces::power_panel_variant(&theme.look),
        cancel.as_ref(),
        &w.panel,
        lay,
        lit,
        cues,
        theme.fonts.size_px,
        s,
    );
    c
}

/// A Welcome icon cell `r` (scaled) laid out `cell_h` tall at 100% with a `label_h` label line
/// `WELCOME_LABEL_Y` down: the icon's top and the label's box at the text size `u`. The label
/// keeps its 100% gap to the cell's bottom, grown; the icon keeps its px and moves down half
/// as far as the label did.
fn welcome_cell(r: Rect, cell_h: i32, label_h: i32, u: TextUnits, s: i32) -> (i32, Rect) {
    let h = r.height / s;
    let label_y = (h - u.len(cell_h - WELCOME_LABEL_Y)).max(WELCOME_LABEL_Y);
    let icon_y = WELCOME_ICON_Y + (label_y - WELCOME_LABEL_Y) / 2;
    let label = Rect::new(r.x, r.y + label_y * s, r.width, u.len(label_h) * s);
    (r.y + icon_y * s, label)
}

/// How a framed Welcome panel's icon button shows: wearing the default
/// ring, what the pointer does to it, and the dotted focus rectangle.
#[derive(Clone, Copy)]
pub(super) struct IconButton {
    pub(super) default: bool,
    pub(super) ptr: Interaction,
    pub(super) focus: bool,
}

/// A framed Welcome panel's icon button `b` across `r` (scaled): a push
/// button with its choice's tile `WELCOME_ICON_Y` into it, faded when greyed, and the label `WELCOME_LABEL_Y` in,
/// centred, its access key underlined while `cues` shows the keys.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_icon_button(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    b: &DialogButton,
    look: IconButton,
    cues: Cues,
) {
    let s = p.s;
    let px = theme.fonts.size_px;
    let icon_px = ICON_PX * s;
    draw_button(
        c,
        text,
        p,
        r,
        "",
        look.default,
        look.ptr,
        b.enabled,
        look.focus,
        px,
    );
    let Some(choice) = b.choice else {
        return;
    };
    let icon = &reprisekit::blue::powerpanel::icon(choice, TileState::Normal, powerpanel::ICON, s);
    let cell_h = GLASS_WELCOME_CELL.1;
    let (icon_y, label) = welcome_cell(r, cell_h, cell_h - WELCOME_LABEL_Y - 4, theme.text, s);
    let at = (r.x + (r.width - icon_px) / 2, icon_y);
    if b.enabled {
        c.blit_over(at.0, at.1, icon);
    } else {
        let mut faded = icon.clone();
        faded.fade(1, 3);
        c.blit_over(at.0, at.1, &faded);
    }
    let ink = if b.enabled {
        p.cs.button_text
    } else {
        p.cs.gray_text
    };
    if cues.keys {
        draw_label(c, text, label, &b.label, false, ink, Align::Center, px, s);
    } else {
        let plain = split_mnemonic(&b.label).0;
        text.draw(c, label, &plain, false, ink, Align::Center, px);
    }
}
