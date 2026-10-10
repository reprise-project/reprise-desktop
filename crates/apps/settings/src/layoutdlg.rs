//! The Add Layout dialog the Keyboard page's Add... opens: a layout and one of
//! its variants from the xkb registry. Laid out like the Shortcut dialog.

use reprisefs::xkb::Layout;
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W};
use reprisekit::controls::combo::{Combo, ListItem};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::raster::Rect;

pub const CLIENT: (i32, i32) = (320, 104);

const MARGIN: i32 = 12;
const LABEL_W: i32 = 64;
const FIELD_X: i32 = MARGIN + LABEL_W + 4;
const FIELD_W: i32 = CLIENT.0 - MARGIN - FIELD_X;
/// The two rows' combo tops; a 13 px label sits 4 px into the 21 px combo.
const ROW_Y: [i32; 2] = [MARGIN, MARGIN + 28];
const LABEL_DY: i32 = 4;
const DROP_ROWS: usize = 12;

const BTN_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const CANCEL_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const OK_X: i32 = CANCEL_X - BUTTON_GAP - BUTTON_W;

pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
const LAYOUT: &str = "layout";
const VARIANT: &str = "variant";

const FIRST: &str = "us";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// OK: the (layout, variant) codes, the variant empty for none.
    Accept(String, String),
    Cancel,
}

pub struct LayoutDialog {
    pub form: Form,
    /// The registry's layouts in the Layout combo's order, by description.
    layouts: Vec<Layout>,
}

fn label(id: &'static str, y: i32, text: &str) -> Item {
    Item::label(id, Rect::new(MARGIN, y + LABEL_DY, LABEL_W, 13), text)
}

fn combo(id: &'static str, y: i32, items: Vec<ListItem>, selected: usize) -> Item {
    Item::new(
        id,
        Rect::new(FIELD_X, y, FIELD_W, 21),
        Control::Combo(Combo::with_items(items, selected, DROP_ROWS)),
    )
}

fn button(id: &'static str, x: i32, text: &str, default: bool) -> Item {
    Item::button(
        id,
        Rect::new(x, BTN_Y, BUTTON_W, BUTTON_H),
        text,
        true,
        default,
    )
}

/// The Variant combo's rows for `l`: "None", then its variants.
fn variant_items(l: Option<&Layout>) -> Vec<ListItem> {
    std::iter::once(ListItem::new("None"))
        .chain(
            l.into_iter()
                .flat_map(|l| &l.variants)
                .map(|v| ListItem::new(v.description.as_str())),
        )
        .collect()
}

impl LayoutDialog {
    pub fn new(reg: &[Layout]) -> LayoutDialog {
        let mut layouts = reg.to_vec();
        layouts.sort_by(|a, b| a.description.cmp(&b.description));
        let first = layouts.iter().position(|l| l.name == FIRST).unwrap_or(0);
        let names = layouts
            .iter()
            .map(|l| ListItem::new(l.description.as_str()))
            .collect();
        let items = vec![
            label("layout_l", ROW_Y[0], "&Layout:"),
            combo(LAYOUT, ROW_Y[0], names, first),
            label("variant_l", ROW_Y[1], "&Variant:"),
            combo(VARIANT, ROW_Y[1], variant_items(layouts.get(first)), 0),
            button(OK, OK_X, "OK", true),
            button(CANCEL, CANCEL_X, "Cancel", false),
        ];
        let mut form = Form::new(items);
        form.set_enabled(OK, !layouts.is_empty());
        LayoutDialog { form, layouts }
    }

    /// A key or a click the host has already given the form, as what it
    /// should do about it.
    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(LAYOUT) => {
                let items = variant_items(self.layouts.get(self.picked(LAYOUT)));
                if let Some(Control::Combo(c)) = self.form.item_mut(VARIANT).map(|i| &mut i.control)
                {
                    *c = Combo::with_items(items, 0, DROP_ROWS);
                }
                Outcome::Redraw
            }
            form::Event::Pressed(OK) | form::Event::Default => match self.value() {
                Some((l, v)) => Outcome::Accept(l, v),
                None => Outcome::None,
            },
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Cancel,
            form::Event::Changed(_) | form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    /// The picked layout and variant codes; None with an empty registry.
    pub fn value(&self) -> Option<(String, String)> {
        let l = self.layouts.get(self.picked(LAYOUT))?;
        let v = match self.picked(VARIANT) {
            0 => String::new(),
            i => l.variants.get(i - 1)?.name.clone(),
        };
        Some((l.name.clone(), v))
    }

    fn picked(&self, id: &str) -> usize {
        match self.form.item(id).map(|i| &i.control) {
            Some(Control::Combo(c)) => c.selected,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Vec<Layout> {
        reprisefs::xkb::parse(
            "<layoutList>\
             <layout><configItem><name>us</name><description>English (US)</description>\
             </configItem><variantList><variant><configItem><name>intl</name>\
             <description>English (US, intl.)</description></configItem></variant>\
             </variantList></layout>\
             <layout><configItem><name>hu</name><description>Hungarian</description>\
             </configItem><variantList><variant><configItem><name>qwerty</name>\
             <description>Hungarian (QWERTY)</description></configItem></variant>\
             </variantList></layout>\
             </layoutList>",
        )
    }

    fn pick(d: &mut LayoutDialog, id: &'static str, i: usize) -> Outcome {
        if let Some(Control::Combo(c)) = d.form.item_mut(id).map(|i| &mut i.control) {
            c.selected = i;
        }
        d.event(form::Event::Changed(id))
    }

    fn variants(d: &LayoutDialog) -> Vec<String> {
        match d.form.item(VARIANT).map(|i| &i.control) {
            Some(Control::Combo(c)) => c.items.iter().map(|i| i.text.clone()).collect(),
            _ => panic!("a combo"),
        }
    }

    #[test]
    fn it_opens_on_us_with_no_variant_and_lists_layouts_by_name() {
        let d = LayoutDialog::new(&reg());
        assert_eq!(d.value(), Some(("us".into(), String::new())));
        assert_eq!(variants(&d), vec!["None", "English (US, intl.)"]);
        match d.form.item(LAYOUT).map(|i| &i.control) {
            Some(Control::Combo(c)) => {
                let names: Vec<&str> = c.items.iter().map(|i| i.text.as_str()).collect();
                assert_eq!(names, vec!["English (US)", "Hungarian"]);
            }
            _ => panic!("a combo"),
        }
    }

    #[test]
    fn a_new_layout_brings_its_own_variants() {
        let mut d = LayoutDialog::new(&reg());
        pick(&mut d, LAYOUT, 1);
        assert_eq!(variants(&d), vec!["None", "Hungarian (QWERTY)"]);
        pick(&mut d, VARIANT, 1);
        assert_eq!(
            d.event(form::Event::Pressed(OK)),
            Outcome::Accept("hu".into(), "qwerty".into())
        );
        assert_eq!(d.event(form::Event::Cancel), Outcome::Cancel);
    }

    #[test]
    fn an_empty_registry_greys_ok() {
        let mut d = LayoutDialog::new(&[]);
        assert_eq!(d.event(form::Event::Default), Outcome::None);
    }

    #[test]
    fn everything_sits_inside_the_client() {
        let d = LayoutDialog::new(&reg());
        for i in &d.form.items {
            let r = i.rect;
            assert!(
                r.x >= 0 && r.y >= 0 && r.right() <= CLIENT.0 && r.bottom() <= CLIENT.1,
                "{} at {r:?}",
                i.id
            );
        }
        assert_eq!(d.form.item(OK).unwrap().rect, Rect::new(152, 69, 75, 23));
    }
}
