//! The font viewer's window: Install, a face picker for a collection, the face's names and its
//! sample ([`fontsample`]) in a well under them. The host reads the file and installs it.

use crate::access::{Tree, accesskit::NodeId};
use crate::classic::widgets::BUTTON_H;
use crate::controls::fontsample;
use crate::controls::form::{Form, Item};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::fontfile::FaceNames;
use crate::text::{TextRenderer, escape_mnemonic};
use crate::theme::TextUnits;

pub const INSTALL: &str = "install";
pub const FACE: &str = "face";
pub const NAME: &str = "name";
pub const VERSION: &str = "version";
pub const COPYRIGHT: &str = "copyright";

pub const CLIENT: (i32, i32) = (600, 512);

const MARGIN: i32 = 11;
const INSTALL_W: i32 = 112;
const FACE_LABEL_X: i32 = MARGIN + INSTALL_W + 16;
const FACE_X: i32 = FACE_LABEL_X + 36;
const RULE_Y: i32 = 44;
const NAME_Y: i32 = 54;
const LINE: i32 = 18;
/// The sample's well at 100%.
const WELL: Rect = Rect {
    x: MARGIN,
    y: NAME_Y + 3 * LINE + 8,
    width: CLIENT.0 - 2 * MARGIN,
    height: CLIENT.1 - (NAME_Y + 3 * LINE + 8) - MARGIN,
};

/// Where the shown face stands with the user's fonts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// Not installed: Install is live.
    No,
    /// Being copied and cached.
    Running,
    /// In the user's fonts folder.
    User,
    /// Installed for every user, under a system fonts folder.
    System,
    /// The copy failed, for this reason; Install is live again.
    Failed(String),
}

impl Install {
    fn label(&self) -> &'static str {
        match self {
            Install::No | Install::Failed(_) => "&Install",
            Install::Running => "Installing...",
            Install::User => "Installed",
            Install::System => "Installed (system)",
        }
    }
}

/// The window's form for face `picked` of `faces` (none: a file that is no font), Install as
/// `install` sets it.
pub fn form(faces: &[FaceNames], picked: usize, install: &Install) -> Form {
    let live = matches!(install, Install::No | Install::Failed(_)) && !faces.is_empty();
    let mut items = vec![Item::button(
        INSTALL,
        Rect::new(MARGIN, MARGIN, INSTALL_W, BUTTON_H),
        install.label(),
        live,
        live,
    )];
    if faces.len() > 1 {
        let titles: Vec<String> = faces.iter().map(FaceNames::title).collect();
        let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
        items.push(Item::label_at(
            "facelabel",
            FACE_LABEL_X,
            MARGIN + 5,
            34,
            "&Face:",
        ));
        let mut combo = Item::combo(
            FACE,
            FACE_X,
            MARGIN + 1,
            CLIENT.0 - MARGIN - FACE_X,
            &titles,
            8,
        );
        if let crate::controls::form::Control::Combo(c) = &mut combo.control {
            c.selected = picked.min(faces.len() - 1);
        }
        items.push(combo);
    }
    items.push(Item::new(
        "rule",
        Rect::new(0, RULE_Y, CLIENT.0, 2),
        crate::controls::form::Control::Separator,
    ));
    let w = CLIENT.0 - 2 * MARGIN;
    let line = |id, row: i32, text: String| {
        Item::label_at(id, MARGIN, NAME_Y + row * LINE, w, &escape_mnemonic(&text))
    };
    match faces.get(picked) {
        Some(f) => {
            items.push(line(NAME, 0, f.title()));
            items.push(line(VERSION, 1, f.version.clone()));
            let second = match install {
                Install::Failed(why) => format!("Could not install the font: {why}"),
                _ => f.copyright.clone(),
            };
            items.push(line(COPYRIGHT, 2, second));
        }
        None => items.push(line(
            NAME,
            0,
            "This file is not a font Reprise can read.".into(),
        )),
    }
    Form::new(items)
}

/// The sample's well in a form grown by `u`, at `s` device px per unit.
pub fn well(u: TextUnits, s: i32) -> Rect {
    let r = u.rect(WELL);
    Rect::new(
        r.x,
        r.y,
        u.len(CLIENT.0) - 2 * r.x,
        u.len(CLIENT.1) - r.y - r.x,
    )
    .scaled(s)
}

/// Draw the window: its face, `form`, and `sample` in the well.
pub fn draw(
    c: &mut Canvas,
    p: &Paint,
    text: &TextRenderer,
    px: u32,
    f: &Form,
    sample: Option<&TextRenderer>,
) {
    let s = p.s.max(1);
    c.fill_rect(c.bounds(), p.cs.button_face);
    crate::controls::form::draw(c, text, p, f, true, px);
    if let Some(sample) = sample {
        fontsample::draw(c, p, text, px, sample, well(p.text, s));
    }
}

/// The window's nodes: the form's, then the sample named for the face `title`. Returns the
/// focused node.
pub fn access(
    tree: &mut Tree,
    parent: NodeId,
    f: &Form,
    text: &TextRenderer,
    px: u32,
    title: &str,
) -> Option<NodeId> {
    let focus = f.access(tree, parent, text, px);
    fontsample::access(tree, parent, title, well(f.units(), 1));
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::form::Control;
    use crate::interface::Interface;
    use crate::interfaces;
    use crate::theme::Theme;

    fn faces() -> Vec<FaceNames> {
        ["Regular", "Bold"]
            .iter()
            .enumerate()
            .map(|(i, style)| FaceNames {
                index: i as u32,
                family: "Tahoma".into(),
                style: style.to_string(),
                full_name: format!("Tahoma {style}"),
                postscript: format!("Tahoma-{style}"),
                version: "Version 5.00".into(),
                copyright: "Copyright & 2024 Sample".into(),
            })
            .collect()
    }

    fn button(f: &Form) -> (String, bool) {
        match &f.item(INSTALL).expect("Install").control {
            Control::Button { label, enabled, .. } => (label.clone(), *enabled),
            _ => unreachable!(),
        }
    }

    #[test]
    fn install_reads_where_the_face_stands() {
        let all = faces();
        let one = &all[..1];
        assert_eq!(
            button(&form(one, 0, &Install::No)),
            ("&Install".into(), true)
        );
        assert!(!button(&form(one, 0, &Install::Running)).1);
        assert_eq!(
            button(&form(one, 0, &Install::User)),
            ("Installed".into(), false)
        );
        assert_eq!(
            button(&form(one, 0, &Install::System)),
            ("Installed (system)".into(), false)
        );
        let failed = form(one, 0, &Install::Failed("disk full".into()));
        assert!(button(&failed).1);
        assert!(matches!(
            &failed.item(COPYRIGHT).unwrap().control,
            Control::Label { text, .. } if text.ends_with("disk full")
        ));
        assert!(
            !button(&form(&[], 0, &Install::No)).1,
            "no font, no Install"
        );
    }

    #[test]
    fn a_collection_picks_its_face_and_a_single_face_has_no_picker() {
        let all = faces();
        let f = form(&all, 1, &Install::No);
        assert_eq!(f.combo_selected(FACE), Some(1));
        assert!(matches!(
            &f.item(NAME).unwrap().control,
            Control::Label { text, .. } if text == "Tahoma Bold"
        ));
        assert!(form(&all[..1], 0, &Install::No).item(FACE).is_none());
        assert!(matches!(
            &f.item(COPYRIGHT).unwrap().control,
            Control::Label { text, .. } if text == "Copyright && 2024 Sample"
        ));
    }

    #[test]
    fn the_window_fits_its_text_under_every_interface() {
        let over = interfaces::form_overflows(|_, _, _| {
            vec![("Font".into(), form(&faces(), 0, &Install::System))]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    fn looks() -> Vec<(String, Theme)> {
        let mut out = vec![
            ("Classic".to_string(), Theme::default()),
            (
                "Blue".to_string(),
                interfaces::blue_theme(crate::blue::Variant::ALL[0]),
            ),
        ];
        for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
            let g = crate::glass::Settings {
                mode,
                ..Default::default()
            };
            let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
            out.push((format!("Glass {mode:?}"), t));
        }
        out
    }

    #[test]
    fn font_viewer_pins_at_scale_1_and_2() {
        let dir = crate::pins::fonts();
        let sample = TextRenderer::from_file(&dir.join("tahoma.ttf"), 0).expect("Tahoma");
        let all = faces();
        let states = [
            ("installable", form(&all[..1], 0, &Install::No)),
            ("collection system", form(&all, 1, &Install::System)),
        ];
        let mut got = Vec::new();
        for (name, base) in looks() {
            for (state, f) in &states {
                for s in [1, 2] {
                    let theme = base.scaled(s);
                    let text = TextRenderer::load(&theme.fonts);
                    let mut c = Canvas::new((CLIENT.0 * s) as u32, (CLIENT.1 * s) as u32);
                    draw(
                        &mut c,
                        &theme.paint(),
                        &text,
                        theme.fonts.size_px,
                        f,
                        Some(&sample),
                    );
                    got.push((format!("{name} {state} {s}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("font_viewer", &got);
    }
}
