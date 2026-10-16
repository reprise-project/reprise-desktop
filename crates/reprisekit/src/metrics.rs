//! The sizes a look lays its common controls out with. Layout and paint read the same values, so
//! a check box's hit area and its drawn box agree.

use crate::raster::Insets;

/// The sizes of the common controls in one look, logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlMetrics {
    /// A radio button's circle, square.
    pub radio: i32,
    /// From a radio's item left edge to its circle's; a pixel right of a check box's.
    pub radio_x: i32,
    /// From a radio button's item's left edge to its label's.
    pub radio_label: i32,
    /// A check box's box, square.
    pub check: i32,
    /// From a check box's left edge to its label's.
    pub check_label: i32,
    /// From a check box's left edge to its label's second line, for a label that runs onto one.
    pub check_wrap: i32,
    /// How far below a one-line label's row both lines of a label that runs onto a second line sit.
    pub check_wrap_dy: i32,
    /// A scrollbar's thickness, which is also its arrow buttons' side.
    pub scroll: i32,
    /// The shortest a scrollbar's thumb gets, however long the content.
    pub thumb_min: i32,
    /// A combo box's drop button width, inside the field's frame.
    pub combo_drop: i32,
    /// The clear columns between a focused combo's picked-item highlight and its drop button.
    pub combo_gap: i32,
    /// A row of a combo's open list without icons, tall. A shell list's rows are 16 px in every
    /// look.
    pub combo_row: i32,
    /// A list box's rows, tall.
    pub list_row: i32,
    /// A buddy spinner's button column width: the year field's, and the time field's.
    pub buddy_spin: i32,
    /// Whether a buddy spinner's buttons sit inside its field's frame, as a plain spinner's do,
    /// rather than beside the field.
    pub buddy_spin_inside: bool,
    /// A vertical trackbar's thumb width. The trackbar is 10 px wider: its ticks and a gap either
    /// side.
    pub vtrack_thumb: i32,
    /// A list view's column header strip, tall.
    pub list_header: i32,
    /// How far short of a right-aligned sorted column's end its sort arrow stops; the title ends
    /// its advance the arrow gap before it.
    pub header_arrow_end: i32,
    /// Whether a sorted column's arrow is centred over the header's top rows (title keeps its
    /// unsorted place) rather than after the title.
    pub header_arrow_over: bool,
    /// A status bar, tall.
    pub status_bar: i32,
    /// The border round a flyout's contents ([`crate::flyout`]): 2 for Classic's raised edge, wider
    /// for a look that frames it as a small window.
    pub flyout_border: i32,
    /// The side of the analog clock beside the clock popup's calendar; 0 for none.
    pub clock_face: i32,
    /// The task pane's group bands.
    pub task: TaskMetrics,
    pub bar: BarMetrics,
    /// A rebar's lines and the face above it.
    pub rebar: RebarMetrics,
    /// The big labelled toolbar strip.
    pub toolbar: ToolbarMetrics,
    /// A menu bar's titles.
    pub menubar: MenuBarMetrics,
    pub address: AddressMetrics,
    /// The file dialog's places bar tiles.
    pub places: PlacesMetrics,
    /// Where the file dialog's rows sit against Classic's.
    pub file_dlg: FileDlgMetrics,
    /// The Start menu's and its submenus' borders.
    pub start_menu: StartMenuMetrics,
    /// How the rows that hold a line of text grow with the text size ([`ControlMetrics::grown`]):
    /// tree, list view and menu bar rows and the calendar's cells read it.
    pub text: crate::theme::TextUnits,
}

/// How far in from the Start menu's edges its rows sit, border included, and where its separators'
/// lines stop, logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartMenuMetrics {
    /// The Start menu's own rows; the band sits `root.left` in too.
    pub root: Insets,
    /// A submenu's rows.
    pub sub: Insets,
    /// How far in from each end of its row a separator's line stops.
    pub separator_x: i32,
    /// A lit row's submenu arrow stays `menu_text`, where menu bars and context menus ink it
    /// `hilight_text`.
    pub lit_arrow_menu_text: bool,
}

impl StartMenuMetrics {
    /// Classic's: 3 px all round (the raised edge and a pixel of face), the separators a pixel
    /// short of each end of the row.
    pub const CLASSIC: StartMenuMetrics = StartMenuMetrics {
        root: Insets::new(3, 3, 3, 3),
        sub: Insets::new(3, 3, 3, 3),
        separator_x: 1,
        lit_arrow_menu_text: false,
    };
}

/// The file dialog's places bar tiles, logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacesMetrics {
    /// A tile's height, and so the pitch from one icon to the next.
    pub tile: i32,
    /// An icon's top, from its tile's.
    pub icon_top: i32,
    /// From the icon's bottom to the label's first line.
    pub label_gap: i32,
    /// From one wrapped label line's top to the next, past the text size.
    pub line_lead: i32,
}

impl PlacesMetrics {
    /// Classic's: 65 px tiles, each icon 3 px down and its label 3 px under it.
    pub const CLASSIC: PlacesMetrics = PlacesMetrics {
        tile: 65,
        icon_top: 3,
        label_gap: 3,
        line_lead: 2,
    };

    /// Tiles whose label, under the 32 px icon, grows by `u`.
    fn grown(self, u: crate::theme::TextUnits) -> PlacesMetrics {
        let icon = self.icon_top + 32;
        PlacesMetrics {
            tile: icon + u.len(self.tile - icon),
            ..self
        }
    }
}

/// Where the file dialog lays out its rows, logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileDlgMetrics {
    /// How far below Classic's place every row sits: the Look in row and toolbar, the places bar,
    /// the shell view, the File name and Files of type rows, their buttons and a host's extra rows.
    pub dy: i32,
}

impl FileDlgMetrics {
    /// Classic's: the rows where the file dialog's consts put them.
    pub const CLASSIC: FileDlgMetrics = FileDlgMetrics { dy: 0 };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddressMetrics {
    /// The Go button, wide: from the field's end to the band's right stripe.
    pub go_w: i32,
    /// How far in from each side of the Go button its lit frame sits.
    pub go_frame_x: i32,
}

impl AddressMetrics {
    /// Classic's: a 57 px Go button lit edge to edge.
    pub const CLASSIC: AddressMetrics = AddressMetrics {
        go_w: 57,
        go_frame_x: 0,
    };
}

/// A rebar's lines and the face above it, logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RebarMetrics {
    /// Face rows between the client's top and the rebar's.
    pub top: i32,
    /// Whether the first band wears a line along its top too; every band after it does.
    pub first_line: bool,
    /// The line under the last band that closes the rebar off.
    pub closing: i32,
    /// Rows between a band's line (or the rebar's top, for a first band without one) and its
    /// content; the band grows by as many.
    pub band_top: i32,
}

impl RebarMetrics {
    /// Classic's: one face row above, a line on every band, and a 2 px closing line.
    pub const CLASSIC: RebarMetrics = RebarMetrics {
        top: 1,
        first_line: true,
        closing: 2,
        band_top: 0,
    };
}

/// Where the big labelled toolbar strip puts its buttons, logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolbarMetrics {
    /// The strip's height.
    pub height: i32,
    /// The first item's left edge, from the strip's.
    pub lead: i32,
    /// A labelled button without a drop part (Search, Folders).
    pub labelled: ButtonPads,
    /// An icon-only button (Up).
    pub bare: ButtonPads,
    /// A labelled drop button's main part (Back).
    pub split: ButtonPads,
    /// A drop button's main part whose label is empty (Forward).
    pub split_empty: ButtonPads,
    /// A labelled drop button's drop part, wide.
    pub split_drop: i32,
    /// How far below the strip's middle a labelled drop button's arrow sits. An unlabelled one's
    /// (Views) sits a pixel below in every look.
    pub split_arrow_dy: i32,
}

/// Where a big toolbar button puts its icon and label: the icon box's inset, the gap from the box
/// to the label (none without label text), and the space after the label (or after the box when
/// there is no text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonPads {
    pub inset: i32,
    pub gap: i32,
    pub trail: i32,
}

impl ToolbarMetrics {
    /// Classic's: a 30 px strip from 1 px in, icons 3 px in, a label 5 px past its icon box and 7
    /// px short of the button's end (or 12 px drop part), 3 px after an icon-only button's icon and
    /// 7 after an empty-labelled one's, the arrow a pixel below the middle.
    pub const CLASSIC: ToolbarMetrics = ToolbarMetrics {
        height: 30,
        lead: 1,
        labelled: ButtonPads {
            inset: 3,
            gap: 5,
            trail: 7,
        },
        bare: ButtonPads {
            inset: 3,
            gap: 0,
            trail: 3,
        },
        split: ButtonPads {
            inset: 3,
            gap: 5,
            trail: 7,
        },
        split_empty: ButtonPads {
            inset: 3,
            gap: 0,
            trail: 7,
        },
        split_drop: 12,
        split_arrow_dy: 1,
    };

    /// A strip whose labels grow by `u`: their 13 px line grows, the 24 px icon box and pads stay.
    fn grown(self, u: crate::theme::TextUnits) -> ToolbarMetrics {
        const LINE: i32 = 13;
        ToolbarMetrics {
            height: self.height + u.len(LINE) - LINE,
            ..self
        }
    }
}

/// Where a menu bar puts its titles, logical px: [`MenuTitles`] on a window's own bar and on one in
/// a rebar band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuBarMetrics {
    pub plain: MenuTitles,
    pub banded: MenuTitles,
    /// Rows at the bottom of a window's own bar in the `window` colour.
    pub line: i32,
}

impl MenuBarMetrics {
    /// Classic's: the same titles in both bars, and no line.
    pub const CLASSIC: MenuBarMetrics = MenuBarMetrics {
        plain: MenuTitles::CLASSIC,
        banded: MenuTitles::CLASSIC,
        line: 0,
    };

    /// The titles of a bar in a rebar band when `banded`, else of a window's own.
    pub fn titles(&self, banded: bool) -> &MenuTitles {
        if banded { &self.banded } else { &self.plain }
    }
}

/// A menu bar's title frames, logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuTitles {
    /// The bar, tall.
    pub bar_h: i32,
    /// The first frame's left edge, from the bar's.
    pub first_x: i32,
    /// Each frame's top, from the bar's, and its height.
    pub frame_y: i32,
    pub frame_h: i32,
    /// From a frame's left edge to its text; the frame is its text plus this much either side.
    pub pad: i32,
    /// How far below the frame's middle the text sits.
    pub text_dy: i32,
}

impl MenuTitles {
    /// The bar and its frames `by` px taller; the frames keep their place from the bar's top.
    fn taller(self, by: i32) -> MenuTitles {
        MenuTitles {
            bar_h: self.bar_h + by,
            frame_h: self.frame_h + by,
            ..self
        }
    }

    /// Classic's: a 20 px bar, 18 px frames a pixel down, from 2 px in, the text 7 px in and
    /// centred down the frame.
    pub const CLASSIC: MenuTitles = MenuTitles {
        bar_h: 20,
        first_x: 2,
        frame_y: 1,
        frame_h: 18,
        pad: 7,
        text_dy: 0,
    };
}

/// Where a task pane lays its groups out, logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskMetrics {
    /// Columns at the pane's left edge left in the window colour before its background starts.
    pub lead: i32,
    /// Background above the first group's title band.
    pub top: i32,
    /// A group's title band.
    pub header: i32,
    /// A group's left edge, from the pane's.
    pub section_x: i32,
    /// What a group's width gives up to the pane's, both sides together.
    pub section_inset: i32,
    /// How far right of Classic's place a body's text sits: a link's past its icon, a Details
    /// line's.
    pub text_dx: i32,
    /// Rows at the pane's bottom edge left in the window colour under its background.
    pub foot: i32,
    /// A line of a link's or a Details line's text, tall.
    pub line: i32,
}

impl TaskMetrics {
    /// Classic's: 23 px title bands 13 px down, each group 12 px in from the pane's left edge and 2
    /// px in from its right.
    pub const CLASSIC: TaskMetrics = TaskMetrics {
        lead: 0,
        top: 13,
        header: 23,
        section_x: 12,
        section_inset: 14,
        text_dx: 0,
        foot: 0,
        line: 13,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarMetrics {
    /// The ring round the whole bar, each side.
    pub ring: i32,
    /// The title strip, inside the ring.
    pub header: i32,
    /// The line under the strip.
    pub line: i32,
    /// The title's text box, from the strip's left edge, and how far below the strip's middle its
    /// line sits.
    pub title_x: i32,
    pub title_dy: i32,
    /// The close box: its size, how far its left edge sits from the strip's right edge, and how far
    /// down the strip its top sits.
    pub close_w: i32,
    pub close_h: i32,
    pub close_right: i32,
    pub close_top: i32,
}

impl BarMetrics {
    /// Classic's: a 2 px etched ring, a 20 px strip over a 2 px etched line, the title 8 px in and
    /// a 16x14 close box 21 px from the strip's right edge, 2 px down.
    pub const CLASSIC: BarMetrics = BarMetrics {
        ring: 2,
        header: 20,
        line: 2,
        title_x: 8,
        title_dy: 0,
        close_w: 16,
        close_h: 14,
        close_right: 21,
        close_top: 2,
    };
}

impl ControlMetrics {
    /// Classic's controls: a 12 px radio and 13 px check box, each with its label 5 px past the box
    /// (a check box label's second line a pixel further right), a 16 px scrollbar with thumb at
    /// least 8 px, a 16 px combo drop button against the highlight, 13 px rows in an open combo
    /// list, a buddy spinner's 16 px button column, a vertical trackbar's 21 px thumb, a 17 px list
    /// header and a 22 px status bar.
    pub const CLASSIC: ControlMetrics = ControlMetrics {
        radio: 12,
        radio_x: 0,
        radio_label: 17,
        check: 13,
        check_label: 18,
        check_wrap: 19,
        check_wrap_dy: 0,
        scroll: 16,
        thumb_min: 8,
        combo_drop: 16,
        combo_gap: 0,
        combo_row: 13,
        list_row: 17,
        buddy_spin: 16,
        buddy_spin_inside: false,
        vtrack_thumb: 21,
        list_header: 17,
        header_arrow_end: 7,
        header_arrow_over: false,
        status_bar: 22,
        flyout_border: 2,
        clock_face: 0,
        task: TaskMetrics::CLASSIC,
        bar: BarMetrics::CLASSIC,
        rebar: RebarMetrics::CLASSIC,
        toolbar: ToolbarMetrics::CLASSIC,
        menubar: MenuBarMetrics::CLASSIC,
        address: AddressMetrics::CLASSIC,
        places: PlacesMetrics::CLASSIC,
        file_dlg: FileDlgMetrics::CLASSIC,
        start_menu: StartMenuMetrics::CLASSIC,
        text: crate::theme::TextUnits::ONE,
    };
}

impl ControlMetrics {
    /// These sizes for text grown by `u`: the rows that hold a line of text grow; boxes, glyphs,
    /// icons, scrollbars and drop buttons keep their size.
    pub fn grown(self, u: crate::theme::TextUnits) -> ControlMetrics {
        ControlMetrics {
            combo_row: u.len(self.combo_row),
            list_row: u.len(self.list_row),
            list_header: u.len(self.list_header),
            menubar: MenuBarMetrics {
                plain: self
                    .menubar
                    .plain
                    .taller(u.len(self.menubar.plain.bar_h) - self.menubar.plain.bar_h),
                banded: self
                    .menubar
                    .banded
                    .taller(u.len(self.menubar.banded.bar_h) - self.menubar.banded.bar_h),
                ..self.menubar
            },
            places: self.places.grown(u),
            toolbar: self.toolbar.grown(u),
            status_bar: u.len(self.status_bar),
            task: TaskMetrics {
                header: u.len(self.task.header),
                line: u.len(self.task.line),
                ..self.task
            },
            bar: BarMetrics {
                header: u.len(self.bar.header),
                ..self.bar
            },
            text: u,
            ..self
        }
    }
}

impl Default for ControlMetrics {
    fn default() -> ControlMetrics {
        ControlMetrics::CLASSIC
    }
}
