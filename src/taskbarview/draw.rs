use std::{borrow::Cow, collections::HashSet, sync::Arc, time::Instant};

use reprisekit::taskbar::{Attention, BarSpan, ClockText};
use reprisescene::{Input, NodeId, TreeId};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use super::{
    Bar, DrawnBar, SPOT_TICK, StartMix, TaskWindows, app_icon, empty_layout, glow_colour,
    quick_icon, stands_over_menu, start_icon,
};
use crate::{
    compositor::{DeskScene, State},
    glass::{self, Pane, PaneShape},
    input::Interaction,
    output::upload_scaled,
    quicklaunch::QuickItem,
    raster::{Canvas, Rect},
    taskbar::{PopupCell, TaskEntry, TaskbarCounts, TaskbarHit, TaskbarLayout, TaskbarState},
    taskslide,
    window::WindowId,
};

impl State {
    /// The pointer's x inside task button `i` of `bar`, logical px from the
    /// button's left edge, as the last draw laid the row out; None if `i`
    /// is stale.
    pub(super) fn task_pointer_x(&self, bar: &Bar, i: usize) -> Option<i32> {
        let r = bar.layout.tasks.get(i)?;
        let s = self.bar_drawn(&bar.output).shown;
        Some(self.pointer_location().x as i32 - s.x - r.x)
    }

    pub(super) fn spot_moved(&mut self, tree: TreeId) {
        let spot = (self.config.theme.look.style()).is_some_and(|s| s.task_spot().is_some());
        let Some(bar) = self.bar(tree).filter(|_| spot) else {
            return;
        };
        let armed = self.bar_shared.spot_timer.is_some();
        if crate::taskbar::spot_due(bar.task_hot_x_drawn, bar.task_hot_x, armed)
            != crate::taskbar::SpotStep::Arm
        {
            return;
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(SPOT_TICK), |_, _, state| {
                state.spot_tick();
                TimeoutAction::Drop
            });
        match token {
            Ok(t) => self.bar_shared.spot_timer = Some(t),
            Err(e) => log::warn!("spot timer: {}", e.error),
        }
    }

    fn spot_tick(&mut self) {
        self.bar_shared.spot_timer = None;
        let due: Vec<TreeId> = (self.bars.iter())
            .filter(|b| {
                crate::taskbar::spot_due(b.task_hot_x_drawn, b.task_hot_x, false)
                    == crate::taskbar::SpotStep::Arm
            })
            .map(|b| b.tree)
            .collect();
        for t in due {
            self.redraw_taskbar(t);
        }
    }

    pub(super) fn held_task(&self, bar: &Bar) -> Option<usize> {
        let Interaction::TaskPress {
            bar: tree,
            key,
            dragging: false,
            ..
        } = self.interaction
        else {
            return None;
        };
        if tree != bar.tree {
            return None;
        }
        let i = crate::taskbar::find_task(&bar.buttons, &bar.pins, &bar.dialogs, &key)?;
        let p = self.pointer_location();
        (self.bar_hit(bar, p.x, p.y) == TaskbarHit::Task(i)).then_some(i)
    }

    /// Draw `bar` without asking for a frame; whether anything it shows changed. An output out
    /// of the layout has an empty box: its bar hides and forgets its buttons, so neither a stale
    /// bar nor a closed window can be hit.
    pub(super) fn redraw_taskbar_with(&mut self, bar: &mut Bar, tasks: &TaskWindows) -> bool {
        #[cfg(test)]
        {
            bar.draws += 1;
        }
        let before = BarLook::of(&self.scene, bar);
        let b = self.layout_box(Some(&bar.output));
        if b.width <= 0 {
            self.scene.set_enabled(bar.tree, false);
            self.scene.set_enabled(bar.orb_tree, false);
            bar.glass
                .set(&mut self.scene, &self.glass, bar.glass_tree, None);
            bar.buttons.clear();
            bar.pins.clear();
            bar.runs.clear();
            bar.dialogs.clear();
            bar.start_mix = None;
            bar.drawn = None;
            bar.orb_up = None;
            bar.layout = empty_layout();
            return BarLook::of(&self.scene, bar) != before;
        }
        let scale = self.draw_scale_at(Some(&bar.output));
        let p = self.bar_placement(&bar.output);
        let chevron = crate::tray::chevron_shown(self);
        let now = Instant::now();
        let reveal = bar.tray_reveal.x(now).max(0);
        let hidden = crate::trayreveal::hidden(self);
        let revealed = crate::trayreveal::cells(
            p.edge.is_side(),
            &self.config.theme.taskbar_shape(),
            hidden,
            reveal,
        );
        let mut tray_icons =
            crate::tray::bar_icons(self, reprisekit::icon::ICON_SIZE * scale, revealed);
        let base = &self.config.theme;
        let m = &base.metrics;
        let (locked, bands) = (self.bar_shared.locked, self.bar_shared.bands);
        let show_desktop = self.config.taskbar.show_desktop_button;
        let both = crate::taskbar::bar_clock_text(true);
        let (time, date) = both.split_once('\n').unwrap_or((both.as_str(), ""));
        let measure = |s: &str| self.text.measure(s, false, base.fonts.size_px);
        let clock_text = ClockText {
            time: measure(time),
            date: measure(date),
        };
        let time = time.to_string();
        let theme = if scale == 1 {
            Cow::Borrowed(base)
        } else {
            Cow::Owned(base.scaled(scale))
        };
        let px = theme.icon_size();
        let icons = &mut self.icons;
        let pinned = self.config.taskbar.pinned;
        // No band with Toolbars > Quick Launch unchecked, nor while
        // `pinned` puts the Quick Launch items on the task row, which
        // shows them whatever the band would.
        let quick_icons: Vec<Option<Arc<Canvas>>> = if pinned || !self.bar_shared.quick_shown {
            Vec::new()
        } else {
            self.quicklaunch
                .iter()
                .map(|q| quick_icon(icons, q, px))
                .collect()
        };
        let TaskWindows {
            windows,
            apps,
            places,
            dialogs,
            pager,
        } = tasks;
        let (pin_count, pin_of, pin_icons): (usize, Vec<Option<usize>>, Vec<Option<String>>) =
            if pinned {
                let pins: &[QuickItem] = &self.quicklaunch;
                let index = &self.app_index;
                let pin_of = apps
                    .iter()
                    .map(|a| {
                        let id = reprisefs::apps::entry_for_app_id(index, a, |_| true);
                        crate::taskbar::pin_for(pins, a, id.map(|e| e.id.as_str()))
                    })
                    .collect();
                let icons = pins.iter().map(|q| q.entry.icon.clone()).collect();
                (pins.len(), pin_of, icons)
            } else {
                (0, Vec::new(), Vec::new())
            };
        let shape = base.taskbar_shape();
        let span = BarSpan {
            edge: p.edge,
            length: if p.edge.is_side() { b.height } else { b.width },
            thickness: p.thickness,
        };
        let mut layout = TaskbarLayout::compute_on(
            m,
            &shape,
            span,
            locked,
            TaskbarCounts {
                quick: quick_icons.len(),
                pager: pager.len(),
                tasks: apps.len() + pin_count + dialogs.len(),
                tray: tray_icons.len() - revealed,
                show_desktop,
                chevron,
                hidden,
                reveal,
            },
            clock_text,
            bands,
        );
        crate::tray::keep_run(&mut tray_icons, revealed, layout.tray_hidden);
        let clock = if layout.clock_lines.is_some() {
            both
        } else {
            time
        };
        let mode = self.config.taskbar.buttons;
        let flash_for = std::time::Duration::from_secs(self.config.taskbar.attention_flash.into());
        let now = Instant::now();
        let dialog_tasks: Vec<crate::taskbar::DialogTask> = dialogs
            .iter()
            .map(|d| crate::taskbar::DialogTask {
                id: d.id,
                place: d.place,
            })
            .collect();
        let slots = crate::taskbar::lay_slots(
            &mut layout,
            m,
            &shape,
            apps,
            places,
            &pin_of,
            pin_count,
            &dialog_tasks,
            mode,
        );
        let task_px = shape.task_icon * scale;
        let task_glow = base.look.style().and_then(|s| s.task_glow);
        let glow_of = |icon: &Option<Arc<Canvas>>| {
            task_glow
                .zip(icon.as_deref())
                .and_then(|(g, ic)| glow_colour(g, ic))
        };
        let index = &self.app_index;
        let focused = self.focused;
        let wins = &self.windows;
        let win = |i: usize| wins.iter().find(|w| w.id == windows[i]);
        let live = |i: &usize| win(*i).is_some_and(|w| focused == Some(w.id) && !w.minimized);
        let entries: Vec<TaskEntry> = slots
            .iter()
            .map(|slot| {
                let group = &slot.windows;
                if let Some(d) = slot
                    .dialog
                    .and_then(|id| dialogs.iter().find(|d| d.id == id))
                {
                    let icon = (d.icon.as_deref()).and_then(|n| icons.get_shared(n, task_px));
                    return TaskEntry {
                        glow: glow_of(&icon),
                        icon,
                        ..crate::taskbar::dialog_entry(&d.title, d.active)
                    };
                }
                if group.is_empty() {
                    let name = slot.pin.and_then(|k| pin_icons.get(k).cloned().flatten());
                    return TaskEntry {
                        title: String::new(),
                        active: false,
                        attention: Attention::None,
                        icon: name.and_then(|n| crate::icon::unread(icons, &n, task_px)),
                        glow: None,
                    };
                }
                let icon = match win(group[0]) {
                    Some(w) => crate::icon::window_icon(icons, index, w, task_px),
                    None => app_icon(icons, index, &apps[group[0]], task_px),
                };
                let title = if group.len() == 1 {
                    win(group[0]).map(|w| w.title()).unwrap_or_default()
                } else {
                    let name = crate::taskbar::group_name(&apps[group[0]]);
                    format!("{name} ({})", group.len())
                };
                TaskEntry {
                    title,
                    active: group.iter().any(live),
                    attention: (group.iter().filter_map(|&i| win(i)))
                        .filter(|w| w.attention.is_some())
                        .max_by_key(|w| w.attention)
                        .map_or(Attention::None, |w| {
                            crate::attention::phase(w.attention, now, flash_for)
                        }),
                    glow: glow_of(&icon),
                    icon,
                }
            })
            .collect();
        bar.buttons = slots
            .iter()
            .map(|s| s.windows.iter().map(|&i| windows[i]).collect())
            .collect();
        bar.pins = slots.iter().map(|s| s.pin).collect();
        bar.runs = if mode == crate::config::TaskbarButtons::Group {
            slots
                .iter()
                .map(|s| match (s.pin, s.windows.first()) {
                    (None, Some(&w)) => apps.get(w).filter(|a| !a.is_empty()).cloned(),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        bar.dialogs = slots.iter().map(|s| s.dialog).collect();
        bar.layout = layout;
        let start_icon = start_icon(&self.config, icons, scale);
        let task_hot = bar
            .task_hot
            .and_then(|key| crate::taskbar::find_task(&bar.buttons, &bar.pins, &bar.dialogs, &key));
        bar.task_held = self.held_task(bar);
        // An idle pin's slot light has no spot, so moves over it redraw nothing.
        let spot = bar.task_held.or(task_hot).is_some_and(|i| {
            (base.look.style()).is_some_and(|s| {
                reprisekit::style::taskbar::task_draws_spot(s, bar.layout.pinned_idle(i))
            })
        });
        bar.task_hot_x_drawn = bar.task_hot_x.filter(|_| spot);
        let (task_shift, task_float) = taskslide::draw_state(bar, Instant::now());
        let pid = |w: WindowId| self.client_pid(w).and_then(|p| u32::try_from(p).ok());
        let any_shown = self.launcher_entries.any_shown();
        let owners: HashSet<u32> = match any_shown {
            true => self.windows.iter().filter_map(|w| pid(w.id)).collect(),
            false => HashSet::new(),
        };
        let task_progress = (bar.buttons.iter().zip(&bar.dialogs))
            .map(|(wins, dialog)| {
                if let Some(id) = dialog {
                    return (dialogs.iter().find(|d| d.id == *id)).and_then(|d| d.progress);
                }
                if !any_shown {
                    return None;
                }
                wins.iter().find_map(|id| {
                    let w = self.windows.iter().find(|w| w.id == *id)?;
                    let entries = &self.launcher_entries;
                    entries.progress(pid(w.id), &w.app_id(), &self.app_index, |p| {
                        owners.contains(&p)
                    })
                })
            })
            .collect();
        let state = TaskbarState {
            start_pressed: bar.start_pressed,
            start_focus: bar.start_pressed && self.menu_keys,
            focus: self.bar_focus(bar),
            start_icon,
            orb_glow: self.config.glass.orb_glow,
            quick_icons,
            quick_hot: bar.quick_hot,
            quick_pressed: bar.quick_pressed,
            quick_drop: bar.quick_drop,
            quick_more: PopupCell {
                hot: bar.more_hot == Some(TaskbarHit::QuickMore),
                open: self.quick_more_open(bar.tree),
                held: false,
            },
            pager: pager.clone(),
            pager_hot: bar.pager_hot,
            pager_mark: self.config.pager_mark,
            start_hot: bar.start_glow.lit(),
            task_hot,
            task_pressed: bar.task_held,
            task_hot_x: bar.task_hot_x_drawn,
            task_shift,
            task_float,
            task_progress,
            show_desktop_hot: bar.show_desktop_hot,
            more: PopupCell {
                hot: bar.more_hot == Some(TaskbarHit::More),
                open: self.more_open(bar.tree),
                held: false,
            },
            chevron: PopupCell {
                hot: bar.chevron_hot,
                open: self.chevron_open(bar),
                held: bar.chevron_held,
            },
            tasks: entries,
            tray_icons,
            clock,
        };
        let layout = bar.layout.scaled(scale);
        let (text, start_text) = (
            self.text.for_output(scale),
            self.start_text.for_output(scale),
        );
        // The orb's own buffer: its rows past the bar, and held the whole
        // orb, which then stands over the menu unless a fullscreen window
        // covers the bar.
        let draw_orb = |state: &TaskbarState| {
            layout.start_orb(state.start_pressed).map(|r| {
                let mut c = Canvas::new(r.width as u32, r.height as u32);
                reprisekit::look::draw_start_above(&mut c, &theme, &layout, state);
                c
            })
        };
        let draw_bar = |state: &TaskbarState| {
            let mut canvas = Canvas::new(layout.width.max(1) as u32, layout.height as u32);
            reprisekit::look::draw_taskbar(&mut canvas, &theme, text, start_text, &layout, state);
            canvas
        };
        let draw = |state: &TaskbarState| (draw_bar(state), draw_orb(state));
        let held = state.start_pressed;
        let fading = !bar.start_glow.settled(Instant::now()) && base.look.style().is_some();
        let painted = if fading {
            let mut state = state;
            state.start_hot = false;
            let dark = draw(&state);
            state.start_hot = true;
            let lit = draw(&state);
            let changed = dark.0.differing_rect(&lit.0);
            bar.start_mix = Some(StartMix {
                dark,
                lit,
                held,
                scale,
                changed,
            });
            bar.drawn = None;
            self.redraw_start(bar);
            true
        } else {
            bar.start_mix = None;
            let canvas = match bar.drawn.take() {
                Some(mut d)
                    if d.scale == scale && d.layout == layout && d.state.same_but_start(&state) =>
                {
                    #[cfg(test)]
                    {
                        bar.start_only_draws += 1;
                    }
                    reprisekit::look::redraw_taskbar_start(
                        &mut d.canvas,
                        &theme,
                        text,
                        start_text,
                        &layout,
                        &state,
                    );
                    d.canvas
                }
                _ => draw_bar(&state),
            };
            let orb = draw_orb(&state);
            let up = self.upload_bar(bar, &canvas, orb, held, scale);
            bar.orb_up = up.orb_up;
            bar.drawn = Some(DrawnBar {
                state,
                layout,
                scale,
                canvas,
            });
            up.painted
        };
        crate::trayreveal::follow_layout(self, bar, now);
        bar.tray_reveal_drawn = bar.tray_reveal.x(now);
        self.scene.set_enabled(bar.tree, true);
        self.lay_bar(bar);
        painted || BarLook::of(&self.scene, bar) != before
    }

    /// `lay_bar`, then a frame on the bar's output.
    pub(crate) fn place_bar(&mut self, bar: &mut Bar) {
        self.lay_bar(bar);
        crate::backend::schedule(self, &bar.output);
    }

    /// Place `bar`'s nodes and glass for its slide and hide state.
    fn lay_bar(&mut self, bar: &mut Bar) {
        let p = self.bar_drawn(&bar.output);
        let rows = crate::barplace::on_output(&p, self.layout_box(Some(&bar.output)));
        let orb = crate::barplace::orb_opacity(self.bar_hide.drawn(&bar.output));
        let scale = bar.buffer_scale;
        let wanted = glass::wanted(&self.config);
        let scene = &mut self.scene;
        scene.set_position(bar.tree, p.shown.x, p.shown.y);
        scene.set_position(bar.orb_tree, p.shown.x, p.shown.y);
        scene.set_position(bar.buffer, rows.x, rows.y);
        let whole = rows == Rect::new(0, 0, bar.layout.width, bar.layout.height);
        let source = (!whole).then(|| {
            smithay::utils::Rectangle::new(
                (f64::from(rows.x * scale), f64::from(rows.y * scale)).into(),
                (
                    f64::from(rows.width * scale),
                    f64::from(rows.height * scale),
                )
                    .into(),
            )
        });
        scene.buffer_set_source_box(bar.buffer, source);
        scene.buffer_set_dest_size(bar.buffer, Some((rows.width, rows.height).into()));
        scene.set_enabled(bar.buffer, !rows.is_empty());
        let up = bar.orb_up.filter(|_| orb.is_some());
        scene.set_enabled(bar.start_above, up.is_some());
        scene.set_opacity(bar.start_above, orb.unwrap_or(0.0));
        scene.set_enabled(bar.orb_tree, up == Some(true));
        let pane = (wanted && !rows.is_empty()).then(|| Pane {
            shapes: vec![PaneShape {
                rect: rows,
                corner: &[],
                square_bottom: false,
                square_top: false,
            }],
            holes: Vec::new(),
        });
        bar.glass
            .set_surface(reprisekit::glass::tint::Surface::Taskbar);
        bar.glass
            .set(&mut self.scene, &self.glass, bar.glass_tree, pane);
    }

    fn redraw_start(&mut self, bar: &mut Bar) {
        let Some(m) = &bar.start_mix else { return };
        let now = Instant::now();
        let l = bar.start_glow.level(now);
        let mut canvas = m.dark.0.clone();
        canvas.mix(&m.lit.0, m.changed, l);
        let orb = m.dark.1.as_ref().zip(m.lit.1.as_ref()).map(|(d, h)| {
            let mut c = d.clone();
            c.mix(h, c.bounds(), l);
            c
        });
        let (held, scale) = (m.held, m.scale);
        bar.orb_up = self.upload_bar(bar, &canvas, orb, held, scale).orb_up;
        self.place_bar(bar);
        if bar.start_glow.settled(now) {
            bar.start_mix = None;
        }
    }

    pub fn start_fade_tick(&mut self, tree: TreeId) -> bool {
        self.with_bar(tree, |s, bar| {
            s.redraw_start(bar);
            bar.start_mix.is_some()
        })
        .unwrap_or(false)
    }

    fn upload_bar(
        &mut self,
        bar: &mut Bar,
        canvas: &Canvas,
        orb: Option<Canvas>,
        held: bool,
        scale: i32,
    ) -> Upload {
        let covered = self.under_fullscreen(bar);
        let scene = &mut self.scene;
        let mut painted = upload_scaled(scene, bar.buffer, canvas, scale);
        bar.buffer_scale = scale;
        let l = &bar.layout;
        let over_menu = stands_over_menu(l, held, covered);
        let parent = if over_menu { bar.orb_tree } else { bar.tree };
        if scene.parent(bar.start_above) != Some(parent) {
            scene.reparent(bar.start_above, parent);
        }
        let pictured = l.start_orb(held).is_some() && orb.is_some();
        match (l.start_orb(held), orb) {
            (Some(r), Some(c)) => {
                painted |= upload_scaled(scene, bar.start_above, &c, scale);
                scene.set_position(bar.start_above, r.x, r.y);
                scene.set_enabled(bar.start_above, true);
                // Only Start takes the pointer, so a click beside the disc or on its
                // glow reaches what lies beneath.
                let layout = l.clone();
                scene.set_input(
                    bar.start_above,
                    Input::Test(Box::new(move |p| {
                        let (x, y) = (p.x.floor() as i32 + r.x, p.y.floor() as i32 + r.y);
                        layout.hit(x, y) == TaskbarHit::Start
                    })),
                );
            }
            _ => scene.set_enabled(bar.start_above, false),
        }
        scene.set_enabled(bar.orb_tree, over_menu);
        Upload {
            orb_up: pictured.then_some(over_menu),
            painted,
        }
    }
}

/// What `upload_bar` did: where the orb went (`Bar::orb_up`) and whether the bar's or the
/// orb's pixels changed.
struct Upload {
    orb_up: Option<bool>,
    painted: bool,
}

/// What a bar's nodes show apart from their pixels: places, flags, opacity, the orb's parent,
/// the buffer's size and scale, and the glass node. The glass pane is the buffer's rows.
#[derive(PartialEq)]
struct BarLook {
    nodes: Vec<NodeLook>,
    orb_parent: Option<TreeId>,
    sizes: [Option<(i32, i32)>; 2],
    buffer_scale: i32,
    glass: Vec<NodeId>,
}

/// A node's place, enabled flag and opacity.
type NodeLook = (Option<(i32, i32)>, bool, Option<f32>);

impl BarLook {
    fn of(scene: &DeskScene, bar: &Bar) -> BarLook {
        let nodes: [NodeId; 5] = [
            bar.tree.into(),
            bar.orb_tree.into(),
            bar.glass_tree.into(),
            bar.buffer.into(),
            bar.start_above.into(),
        ];
        let size = |b| scene.buffer_size(b).map(|s| (s.w, s.h));
        BarLook {
            nodes: (nodes.iter())
                .map(|&n| {
                    let at = scene.position(n).map(|p| (p.x, p.y));
                    (at, scene.enabled(n), scene.opacity(n))
                })
                .collect(),
            orb_parent: scene.parent(bar.start_above),
            sizes: [size(bar.buffer), size(bar.start_above)],
            buffer_scale: bar.buffer_scale,
            glass: scene.children(bar.glass_tree).to_vec(),
        }
    }
}
