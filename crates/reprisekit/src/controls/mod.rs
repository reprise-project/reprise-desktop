//! Look-neutral controls: each has a model, layout, hit test and access part, and draws common
//! widgets through `look::widgets`.

pub mod aboutbox;
pub mod addressbar;
pub mod arrange;
pub mod browsefolder;
pub mod calendar;
pub mod captiontabs;
pub mod cliphistory;
pub mod colorbutton;
pub mod colorpick;
pub mod combo;
pub mod copyprogress;
pub mod displaypanel;
pub mod dlgtemplate;
pub mod editmenu;
pub mod emojipicker;
pub mod endsession;
pub mod explorerbar;
pub mod filedlg;
pub mod filesheet;
pub mod finddlg;
mod fit;
pub mod flip;
pub mod form;
pub mod grouplist;
pub mod hosted;
pub mod hotkey;
pub mod iconlist;
pub mod iconview;
pub mod infotip;
pub mod launcher;
pub mod listbox;
pub mod listview;
pub mod locationprompt;
pub mod menu;
pub mod menubar;
pub mod mixer;
pub mod msgbox;
pub mod multiedit;
pub mod notifyhistory;
pub mod notifysheet;
pub mod openwith;
pub mod osd;
pub mod overview;
pub mod pagelist;
pub mod placesbar;
pub mod propsheet;
pub mod rebar;
pub mod regionselect;
pub mod replacebox;
pub mod scrollbar;
mod shellgrid;
pub mod shellview;
pub mod snapassist;
pub mod snapbar;
pub mod snappicker;
pub mod statspanel;
pub mod statusbar;
pub mod switcher;
pub mod tabstrip;
pub mod taskpane;
pub mod taskpreview;
pub mod termgrid;
pub mod tilelist;
pub mod timefield;
pub mod toolbar;
pub mod tooltip;
pub mod traycustomize;
pub mod treeview;
pub mod updown;
pub mod volumepopup;
pub mod wizard;

/// The look's control sizes live in the root [`crate::metrics`]; hosts
/// and controls also reach them here.
pub use crate::metrics;

/// A label's layout rect on the device: its width was measured at the
/// device font size, so only the position and height scale.
pub(crate) fn label_rect(r: crate::raster::Rect, s: i32) -> crate::raster::Rect {
    crate::raster::Rect::new(r.x * s, r.y * s, r.width, r.height * s)
}
