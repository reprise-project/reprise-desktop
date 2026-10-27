//! Sounds and Audio Devices: the Reprise volume control.
//!
//! One process: a tray speaker that follows the default sink, the volume popup a click on it opens, and the properties sheet a double click or Adjust Audio Properties opens. Open Volume Control and the sheet's Volume... buttons start reprise-volume, the mixer window. `--tray` starts with the sheet hidden (what the autostart entry uses); closing the sheet hides it again, only Exit in the tray menu quits.

mod appicons;
mod mpris;
mod sheet;
mod tray;

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use repriseapp::keys::{dialog_key, sheet_input};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{App, BTN_LEFT, Child, Ctx, Event, PointerInput, Reply, WinId};
use repriseaudio as pipewire;
use reprisekit::access::{ROOT, Tree};
use reprisekit::controls::form;
use reprisekit::controls::propsheet;
use reprisekit::controls::volumepopup::{self, VolumePopup};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::taskbar::Edge;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

const ERROR_WIN: WinId = WinId::named("error");
const VOLUME_CONTROL: &str = "reprise-volume";
const FLYOUT_WIN: WinId = WinId::named("flyout");
/// Master's icon in the popup, as the Volume Control window shows it.
const SPEAKER: &str = "audio-volume-high";

#[derive(Default)]
struct Sound {
    tray: Option<reprisetray::TrayHandle>,
    msgs: Option<std::sync::mpsc::Receiver<reprisetray::Msg>>,
    snapshots: Option<pipewire::Watch>,
    snapshot: pipewire::Snapshot,
    tray_state: Option<(&'static str, String, bool)>,
    /// Built on first use: `Sheet::new` needs the icon cache, which does
    /// not exist until `repriseapp::run` has a `Ctx`.
    sheet: Option<sheet::Sheet>,
    /// The sheet was built before an event gave us a `&mut Ctx` to read
    /// the icons from, so it is drawing placeholders and wants rebuilding.
    iconless: bool,
    hidden: bool,
    /// `--tray` was given: the sheet starts hidden and the tray item is
    /// the only way to it, so a tray that will not start is fatal.
    tray_only: bool,
    error: Option<MsgBox>,
    /// The renderer and forms theme the box and window are sized with:
    /// `children` and `initial_size` have no `Ctx`, so they are kept from
    /// `start` and the last event.
    dialog_text: Option<Rc<TextRenderer>>,
    forms: Theme,
    /// The client size last asked for the main window.
    asked: (i32, i32),
    gate: tray::ClickGate,
    waker: Option<repriseapp::Waker>,
    flyout: Option<Open>,
    app_icons: appicons::Resolver,
    /// The icons of the programs playing now, by `App::key`: the sheet's
    /// list and the popup's columns both show them.
    icons: BTreeMap<String, appicons::AppIcon>,
    /// Master's icon in the popup.
    speaker: Option<Canvas>,
    /// The last volume the tray wheel or the popup wrote per node. The
    /// snapshot lags a write, so while it settles this stands in for the
    /// snapshot's own volume, and a wheel burst or key repeat on either
    /// path counts on from it.
    recent: pipewire::Recent<u32, f64>,
    players: Option<mpris::Watch>,
    playing: mpris::Pick,
}

struct Open {
    at: (i32, i32),
    edge: Edge,
    popup: VolumePopup,
}

fn interval(ctx: &Ctx) -> Duration {
    Duration::from_millis(ctx.theme.metrics.double_click_ms.max(0) as u64)
}

fn track_pos(volume: f64) -> usize {
    let last = (volumepopup::STEPS - 1) as f64;
    (volume * last).round().clamp(0.0, last) as usize
}

fn pick(list: &[pipewire::Device], name: Option<&str>) -> Option<usize> {
    name.and_then(|n| list.iter().position(|d| d.name == n))
        .or_else(|| (!list.is_empty()).then_some(0))
}

/// The popup's view of a snapshot: master is the tray's sink, programs
/// carry their 32 px icon from `icons` (keyed by `App::key`).
fn mix(snap: &pipewire::Snapshot, icons: &BTreeMap<String, appicons::AppIcon>) -> volumepopup::Mix {
    let output = pick(&snap.sinks, snap.default_sink.as_deref());
    let master = output.map_or_else(Default::default, |i| volumepopup::Level {
        pos: track_pos(snap.sinks[i].volume),
        mute: snap.sinks[i].mute,
        live: true,
        icon: None,
    });
    let labels = |list: &[pipewire::Device]| list.iter().map(|d| d.label.clone()).collect();
    volumepopup::Mix {
        master,
        outputs: labels(&snap.sinks),
        output,
        inputs: labels(&snap.sources),
        input: pick(&snap.sources, snap.default_source.as_deref()),
        playing: None,
        apps: snap
            .apps
            .iter()
            .map(|a| volumepopup::AppLevel {
                key: a.key.clone(),
                label: a.label.clone(),
                icon: icons.get(&a.key).and_then(|i| i.large.clone()),
                pos: track_pos(a.volume),
                mute: a.mute,
            })
            .collect(),
    }
}

/// Where the panel's edge nearest the taskbar is centred, in the flyout's
/// own coordinates: on the click point `at`, which the tray puts over the
/// icon on the taskbar's edge facing the screen.
fn anchor(at: (i32, i32), origin: (i32, i32)) -> (i32, i32) {
    (at.0 - origin.0, at.1 - origin.1)
}

/// The popup grows with the text size as the program's forms do.
fn flyout_layout(f: &Open, ctx: &Ctx) -> volumepopup::Layout {
    volumepopup::layout(
        &f.popup,
        &ctx.forms(),
        &ctx.text,
        anchor(f.at, ctx.origin),
        f.edge,
        Rect::new(0, 0, ctx.size.0, ctx.size.1),
    )
}

impl Sound {
    fn default_sink(&self) -> Option<&pipewire::Device> {
        let by_name = self
            .snapshot
            .default_sink
            .as_ref()
            .and_then(|name| self.snapshot.sinks.iter().find(|d| &d.name == name));
        by_name.or_else(|| self.snapshot.sinks.first())
    }

    fn push_state(&mut self) {
        let (volume, mute) = self
            .default_sink()
            .map(|d| (d.volume, d.mute))
            .unwrap_or((0.0, true));
        let next = (
            pipewire::icon_for(volume, mute),
            tray::tooltip_text(volume, mute),
            mute,
        );
        // `pw-dump -m` reports every property change, most of which move
        // neither the icon nor the tooltip; re-sending would emit a
        // NewIcon / NewToolTip the tray then redraws for nothing.
        if self.tray_state.as_ref() == Some(&next) {
            return;
        }
        if let Some(tray) = &self.tray {
            tray.set_state(tray::shown(next.0, next.1.clone(), next.2));
        }
        self.tray_state = Some(next);
    }

    /// Build the sheet, or rebuild one built without icons; `icons` is None when `draw` beat the first event.
    fn build(&mut self, icons: Option<sheet::Icons>) -> bool {
        if self.sheet.is_some() && !(self.iconless && icons.is_some()) {
            return false;
        }
        self.iconless = icons.is_none();
        let tab = self.sheet.as_ref().map(|s| s.sheet.tabs.open);
        let mut s = sheet::Sheet::new(&self.snapshot, icons.unwrap_or_default());
        // A rebuild keeps the tab the tray menu picked.
        if let Some(tab) = tab {
            s.open(tab);
        }
        self.sheet = Some(s);
        true
    }

    fn push_app_icons(&mut self, ctx: &mut Ctx) {
        self.icons = self.app_icons.icons(&self.snapshot.apps, &ctx.icons);
        self.speaker = ctx.icons.get(SPEAKER, 32);
        if let Some(s) = &mut self.sheet {
            s.set_app_icons(self.icons.clone());
        }
    }

    fn written(&self, ids: &[u32]) -> Option<f64> {
        ids.iter().find_map(|id| self.recent.get(id))
    }

    /// The mix the popup is built and synced from: the snapshot's, with
    /// each channel's unsettled write over its volume, and the player's
    /// track.
    fn popup_mix(&self) -> volumepopup::Mix {
        let mut m = mix(&self.snapshot, &self.icons);
        m.master.icon = self.speaker.clone();
        m.playing = self.playing.as_ref().map(|(_, np)| np.clone());
        if m.master.live
            && let Some(v) = self.written(&self.nodes(&volumepopup::Channel::Master))
        {
            m.master.pos = track_pos(v);
        }
        for a in &mut m.apps {
            if let Some(v) = self.written(&self.nodes(&volumepopup::Channel::App(a.key.clone()))) {
                a.pos = track_pos(v);
            }
        }
        m
    }

    fn note_volume(&mut self, ch: &volumepopup::Channel, v: f64) -> Vec<u32> {
        let ids = self.nodes(ch);
        let now = Instant::now();
        for &id in &ids {
            self.recent.note(id, v, now);
        }
        ids
    }

    /// The default sink and the volume a wheel burst of `steps` puts it
    /// at, 5 % a step, counted from its last write while that settles.
    fn wheel_volume(&self, steps: i32) -> Option<(u32, f64)> {
        let d = self.default_sink()?;
        let base = self.recent.get(&d.id).unwrap_or(d.volume);
        Some((d.id, (base + 0.05 * f64::from(steps)).clamp(0.0, 1.0)))
    }

    /// The node ids a channel's writes go to: the default sink for master,
    /// every stream of a program.
    fn nodes(&self, ch: &volumepopup::Channel) -> Vec<u32> {
        match ch {
            volumepopup::Channel::Master => self.default_sink().map(|d| d.id).into_iter().collect(),
            volumepopup::Channel::App(k) => self
                .snapshot
                .apps
                .iter()
                .find(|a| &a.key == k)
                .map(pipewire::App::ids)
                .unwrap_or_default(),
        }
    }

    fn ensure(&mut self, ctx: &mut Ctx) {
        let icons = sheet::Icons {
            speaker: ctx.icons.get("audio-speakers", 32),
            mic: ctx.icons.get("audio-input-microphone", 32),
            speaker_small: ctx.icons.get("audio-speakers", 16),
            mic_small: ctx.icons.get("audio-input-microphone", 16),
        };
        if self.build(Some(icons)) {
            self.push_app_icons(ctx);
        }
    }

    fn outcome(&mut self, out: sheet::Outcome) -> Reply {
        match out {
            sheet::Outcome::None => Reply::none(),
            sheet::Outcome::Redraw => Reply::redraw(),
            sheet::Outcome::Apply(changes) => {
                self.apply(&changes);
                Reply::redraw()
            }
            sheet::Outcome::ApplyAndClose(changes) => {
                // A failed OK keeps the sheet up with the picks pending.
                if self.apply(&changes) {
                    self.hidden = true;
                }
                Reply::redraw()
            }
            sheet::Outcome::Close => {
                self.hidden = true;
                Reply::redraw()
            }
            sheet::Outcome::VolumeControl { record } => {
                let mut argv = vec![VOLUME_CONTROL.into()];
                if record {
                    argv.push("--record".into());
                }
                Reply {
                    spawn: vec![argv],
                    ..Reply::none()
                }
            }
        }
    }

    /// Write `changes` through `wpctl`. When it cannot run, the sheet
    /// gets the last snapshot back as its baseline and a box says so.
    fn apply(&mut self, changes: &[sheet::Change]) -> bool {
        if write(&self.snapshot, changes) {
            return true;
        }
        if let Some(s) = &mut self.sheet {
            s.unapplied(&self.snapshot);
        }
        self.error = Some(MsgBox::new(
            "Sounds and Audio Devices",
            reprisekit::controls::msgbox::Fields {
                text: "wpctl could not be run, so nothing was changed.".into(),
                icon: None,
                buttons: vec!["OK".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        ));
        false
    }

    fn error_event(&mut self, ev: Event) -> Reply {
        let (Some(b), Some(text)) = (&mut self.error, &self.dialog_text) else {
            return Reply::none();
        };
        match b.event(&ev, text, &self.forms) {
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn flyout_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        use volumepopup::Event as E;
        let Some(f) = &mut self.flyout else {
            return Reply::none();
        };
        let lay = flyout_layout(f, ctx);
        let out = match ev {
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            }) => f.popup.press(&lay, x, y),
            // Any other button outside the panel closes it too.
            Event::Pointer(PointerInput::Press { x, y, .. }) => {
                match volumepopup::hit(&f.popup, &lay, x, y) {
                    volumepopup::Hit::Outside => E::Close,
                    _ => E::None,
                }
            }
            Event::Pointer(PointerInput::Motion { x, y }) => f.popup.motion(&lay, x, y),
            Event::Pointer(PointerInput::Scroll { x, y, rows }) => f.popup.scroll(&lay, x, y, rows),
            Event::Pointer(PointerInput::Release { .. }) => f.popup.release(),
            Event::Pointer(PointerInput::Leave) => f.popup.leave(),
            Event::Key(k) => match dialog_key(&k) {
                Some(form::Key::Edit(e)) => match form::flat_key(&e) {
                    Some(k) => f.popup.key(&lay, k),
                    None => E::None,
                },
                Some(k) => f.popup.key(&lay, k),
                None => E::None,
            },
            Event::CloseRequest => E::Close,
            _ => E::None,
        };
        self.flyout_outcome(out)
    }

    /// Every write goes out at once: the popup has no Apply. Master is
    /// the default sink, a program is each of its streams.
    fn flyout_outcome(&mut self, ev: volumepopup::Event) -> Reply {
        use volumepopup::{Dir, Event as E};
        match ev {
            E::None => Reply::none(),
            E::Redraw => Reply::redraw(),
            E::Close => {
                self.flyout = None;
                Reply::redraw()
            }
            E::Volume(ch, pos) => {
                let v = pos as f64 / (volumepopup::STEPS - 1) as f64;
                for id in self.note_volume(&ch, v) {
                    pipewire::set_volume(&self.snapshot, id, v);
                }
                Reply::redraw()
            }
            E::Mute(ch, on) => {
                for id in self.nodes(&ch) {
                    pipewire::set_mute(id, on);
                }
                Reply::redraw()
            }
            E::Default(dir, i) => {
                let list = match dir {
                    Dir::Output => &self.snapshot.sinks,
                    Dir::Input => &self.snapshot.sources,
                };
                if let Some(d) = list.get(i) {
                    pipewire::set_default(d.id);
                }
                Reply::redraw()
            }
            E::Media(t) => {
                if let (Some(w), Some((bus, _))) = (&self.players, &self.playing) {
                    w.send(bus, t);
                }
                Reply::redraw()
            }
        }
    }

    fn wake(&mut self, ctx: &mut Ctx) -> Reply {
        let mut fresh = None;
        if let Some(rx) = &self.snapshots {
            while let Ok(snapshot) = rx.try_recv() {
                fresh = Some(snapshot);
            }
        }
        let mut player = None;
        if let Some(w) = &self.players {
            while let Ok(p) = w.picks.try_recv() {
                player = Some(p);
            }
        }
        let landed = self.app_icons.take_landed();
        let resync = fresh.is_some() || player.is_some() || landed;
        if let Some(p) = player {
            self.playing = p;
        }
        if landed && fresh.is_none() {
            self.push_app_icons(ctx);
        }
        if let Some(snapshot) = fresh {
            self.snapshot = snapshot;
            if let Some(s) = &mut self.sheet {
                s.refresh(&self.snapshot);
            }
            self.push_app_icons(ctx);
            self.push_state();
        }
        if resync {
            let mix = self.popup_mix();
            if let Some(f) = &mut self.flyout {
                // The wake redraws everything, so the Redraw a thumb that
                // turned hot or cold asks for needs no reply of its own.
                let lay = flyout_layout(f, ctx);
                f.popup.sync(&lay, &mix);
                // That sync found the part under the pointer on the layout
                // from before it. A program that came or went changes the
                // panel's width and moves every column, so sync again on
                // the new layout; the second sync changes nothing else.
                // The strip coming or going moves the panel's top the same
                // way.
                let lay = flyout_layout(f, ctx);
                f.popup.sync(&lay, &mix);
            }
        }
        let msgs: Vec<tray::TrayMsg> = self
            .msgs
            .as_ref()
            .map(|rx| rx.try_iter().filter_map(tray::translate).collect())
            .unwrap_or_default();
        // A wheel step never comes with a handoff.
        let raised = msgs.iter().any(|m| !matches!(m, tray::TrayMsg::Scroll(_)));
        let mut show = false;
        // A fast wheel burst arrives as several messages in one wake. Each
        // one recomputing from the same snapshot would move the volume a
        // single step, so the steps are summed and written once.
        let mut steps = 0i32;
        let mut spawn = Vec::new();
        for msg in msgs {
            match msg {
                tray::TrayMsg::Exit => {
                    // The process ends here; the pw-dump child should not
                    // run on until its next write fails.
                    if let Some(w) = &self.snapshots {
                        w.stop();
                    }
                    return Reply {
                        quit: true,
                        ..Reply::none()
                    };
                }
                tray::TrayMsg::ToggleMute => {
                    if let Some(d) = self.default_sink() {
                        pipewire::set_mute(d.id, !d.mute);
                    }
                }
                tray::TrayMsg::Scroll(delta) => steps = steps.saturating_add(delta),
                tray::TrayMsg::Activate(x, y) => {
                    let interval = interval(ctx);
                    match self.gate.press(Instant::now(), (x, y), interval) {
                        tray::Press::Double => {
                            self.flyout = None;
                            show = true;
                        }
                        tray::Press::First => {
                            if let Some(w) = self.waker.clone() {
                                std::thread::spawn(move || {
                                    std::thread::sleep(interval + Duration::from_millis(10));
                                    w.wake();
                                });
                            }
                        }
                    }
                }
                // The sheet opens on its Audio page, whichever one it was
                // last closed on.
                tray::TrayMsg::Properties => {
                    self.ensure(ctx);
                    if let Some(s) = &mut self.sheet {
                        s.open(sheet::AUDIO);
                    }
                    self.flyout = None;
                    show = true;
                }
                tray::TrayMsg::VolumeControl => {
                    self.flyout = None;
                    spawn.push(vec![VOLUME_CONTROL.into()]);
                }
            }
        }
        // `pw-dump` needs a moment to report a write back, so a burst
        // spread over several wakes would otherwise keep recomputing from
        // the same stale snapshot and move one step in total.
        if steps != 0
            && let Some((id, v)) = self.wheel_volume(steps)
        {
            pipewire::set_volume(&self.snapshot, id, v);
            self.recent.note(id, v, Instant::now());
        }
        if let Some(at) = self.gate.due(Instant::now(), interval(ctx)) {
            // A program installed since it started playing finds its icon when the popup opens.
            self.push_app_icons(ctx);
            let mix = self.popup_mix();
            self.flyout = Some(Open {
                at,
                edge: Edge::from_name(&repriseconf::taskbar_position()).unwrap_or_default(),
                popup: VolumePopup::new(&mix),
            });
        }
        if show {
            self.hidden = false;
        }
        Reply {
            activate: reprisetray::handoff_for(
                self.tray.as_ref(),
                raised,
                show.then_some(repriseapp::MAIN),
            ),
            spawn,
            ..Reply::redraw_all()
        }
    }
}

fn write(snap: &pipewire::Snapshot, changes: &[sheet::Change]) -> bool {
    changes.iter().all(|change| match *change {
        sheet::Change::Profile(card, index) => pipewire::set_profile(card, index),
        sheet::Change::Default(id) => pipewire::set_default(id),
        sheet::Change::Volume(id, v) => pipewire::set_volume(snap, id, v),
        sheet::Change::Mute(id, on) => pipewire::set_mute(id, on),
    })
}

impl App for Sound {
    fn title(&self) -> String {
        "Sounds and Audio Devices Properties".into()
    }

    fn app_id(&self) -> String {
        "reprise.sound".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.forms.text.size(sheet::CLIENT)
    }

    fn children(&self) -> Vec<Child> {
        match (&self.error, &self.dialog_text) {
            (Some(b), Some(text)) => vec![b.child(ERROR_WIN, text, &self.forms)],
            _ => Vec::new(),
        }
    }

    fn start(&mut self, waker: repriseapp::Waker) -> repriseapp::Start {
        self.waker = Some(waker.clone());
        self.app_icons.set_waker(waker.clone());
        self.dialog_text = Some(repriseapp::start_text());
        self.asked = sheet::CLIENT;
        // The tray goes first: it takes the `reprise.sound` bus name and a
        // second instance exits here once it has raised the first. Spawning
        // `pw-dump` before that would leave the loser's child behind.
        match tray::start(waker.clone()).or_exit(self.tray_only) {
            Ok(Some((tray, msgs))) => {
                self.tray = Some(tray);
                self.msgs = Some(msgs);
            }
            Ok(None) => {}
            Err(code) => return repriseapp::Start::Exit(code),
        }
        self.players = Some(mpris::watch(waker.clone()));
        self.snapshots = Some(pipewire::watch(move || waker.wake()));
        repriseapp::Start::Continue
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        self.dialog_text = Some(ctx.text.clone());
        self.forms = ctx.forms();
        let mut reply = self.dispatch(win, ev, ctx);
        let size = self.forms.text.size(sheet::CLIENT);
        if size != self.asked {
            self.asked = size;
            reply.size = Some(size);
            reply.redraw = true;
        }
        reply
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        self.dialog_text = Some(ctx.text.clone());
        if win == FLYOUT_WIN {
            // The canvas arrives transparent; only the panel is drawn.
            if let Some(f) = &self.flyout {
                let lay = flyout_layout(f, ctx);
                volumepopup::draw(c, &ctx.forms(), &ctx.text, &f.popup, &lay);
            }
            return;
        }
        let forms = ctx.forms();
        if win == ERROR_WIN {
            if let (Some(b), Some(text)) = (&self.error, &self.dialog_text) {
                b.draw(c, &forms, text);
            }
            return;
        }
        self.build(None);
        c.fill_rect(c.bounds(), forms.colors.button_face);
        if let Some(s) = &self.sheet {
            propsheet::draw(
                c,
                &ctx.text,
                &Paint {
                    s: 1,
                    ..forms.paint()
                },
                &s.sheet,
                true,
                forms.fonts.size_px,
            );
        }
    }
}

impl repriseapp::Extended for Sound {
    fn fixed(&self) -> bool {
        true
    }

    fn hidden(&self) -> bool {
        self.hidden
    }

    fn flyout(&self) -> Option<repriseapp::Flyout> {
        self.flyout.as_ref().map(|f| repriseapp::Flyout {
            id: FLYOUT_WIN,
            at: f.at,
            title: volumepopup::VOLUME,
        })
    }

    fn reader_form(&mut self, win: WinId) -> Option<repriseapp::ReaderForm<'_>> {
        let s = self
            .sheet
            .as_mut()
            .filter(|_| win != ERROR_WIN && win != FLYOUT_WIN)?;
        Some(repriseapp::ReaderForm::Sheet(&mut s.sheet))
    }

    fn reader(&mut self, win: WinId, act: repriseapp::Reader, ctx: &Ctx) -> bool {
        match (win, act, &mut self.flyout) {
            (FLYOUT_WIN, repriseapp::Reader::Focus((x, y)), Some(f)) => {
                let lay = flyout_layout(f, ctx);
                f.popup.focus_at(&lay, x, y) != volumepopup::Event::None
            }
            _ => repriseapp::reader_forms(self, win, act, ctx),
        }
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        match win {
            FLYOUT_WIN => {
                if let Some(f) = &self.flyout {
                    let lay = flyout_layout(f, ctx);
                    let px = ctx.forms().fonts.size_px;
                    let focus = f.popup.access(&lay, &ctx.text, px, tree, ROOT);
                    tree.set_focus(focus);
                }
            }
            ERROR_WIN => {
                if let (Some(b), Some(text)) = (&self.error, &self.dialog_text) {
                    b.access(tree, text, &forms);
                }
            }
            _ => {
                if let Some(s) = &self.sheet {
                    oneshot::access_sheet(&s.sheet, tree, &ctx.text, forms.fonts.size_px);
                }
            }
        }
    }
}

impl Sound {
    fn dispatch(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::Wake) {
            return self.wake(ctx);
        }
        if win == ERROR_WIN {
            return self.error_event(ev);
        }
        if win == FLYOUT_WIN {
            return self.flyout_event(ev, ctx);
        }
        self.ensure(ctx);
        let Some(s) = &mut self.sheet else {
            return Reply::none();
        };
        let out = match ev {
            // The X hides the sheet; the tray item keeps the process up.
            Event::CloseRequest => {
                self.hidden = true;
                return Reply::redraw();
            }
            ev => match sheet_input(&mut s.sheet, &ev, ctx) {
                Some(ev) => s.event(ev),
                None => return Reply::none(),
            },
        };
        self.outcome(out)
    }
}

/// Runs the program; `args` are the command line after the program's name.
pub fn main(args: Vec<std::ffi::OsString>) {
    repriseapp::applet_main(&args, |tray_only| Sound {
        hidden: tray_only,
        tray_only,
        ..Sound::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: u32, name: &str) -> pipewire::Device {
        pipewire::Device {
            id,
            name: name.into(),
            label: name.to_uppercase(),
            volume: 0.5,
            mute: false,
            card: None,
            channels: Default::default(),
            route: None,
        }
    }

    fn app(key: &str, streams: &[u32], volume: f64) -> pipewire::App {
        pipewire::App {
            key: key.into(),
            label: key.into(),
            icon_hints: vec![],
            pids: Vec::new(),
            binary: None,
            streams: streams
                .iter()
                .map(|&id| pipewire::Stream {
                    id,
                    channels: Default::default(),
                })
                .collect(),
            volume,
            mute: false,
        }
    }

    fn snap() -> pipewire::Snapshot {
        pipewire::Snapshot {
            sinks: vec![device(40, "hdmi"), device(41, "speakers")],
            sources: vec![device(50, "mic")],
            default_sink: Some("speakers".into()),
            default_source: Some("mic".into()),
            cards: vec![],
            apps: vec![app("firefox", &[90, 91], 0.8), app("mpv", &[92], 0.5)],
        }
    }

    #[test]
    fn mix_takes_the_defaults_and_each_program() {
        let m = mix(&snap(), &BTreeMap::new());
        assert_eq!(m.outputs, ["HDMI", "SPEAKERS"]);
        assert_eq!(m.output, Some(1));
        assert_eq!(m.input, Some(0));
        assert_eq!(
            m.master,
            volumepopup::Level {
                pos: 10,
                mute: false,
                live: true,
                icon: None,
            }
        );
        let keys: Vec<_> = m.apps.iter().map(|a| (a.key.as_str(), a.pos)).collect();
        assert_eq!(keys, [("firefox", 16), ("mpv", 10)]);
    }

    #[test]
    fn output_falls_back_to_the_first_sink() {
        let mut s = snap();
        s.default_sink = Some("gone".into());
        let m = mix(&s, &BTreeMap::new());
        assert_eq!(m.output, Some(0));
        assert!(m.master.live);
    }

    #[test]
    fn no_sinks_greys_master_and_output() {
        let mut s = snap();
        s.sinks.clear();
        let m = mix(&s, &BTreeMap::new());
        assert_eq!(m.output, None);
        assert!(!m.master.live);
    }

    #[test]
    fn master_carries_the_sinks_mute_and_clamps_its_volume() {
        let mut s = snap();
        s.sinks[1].mute = true;
        s.sinks[1].volume = 1.4;
        let m = mix(&s, &BTreeMap::new());
        assert!(m.master.mute);
        assert_eq!(m.master.pos, volumepopup::STEPS - 1, "clamped to the top");
    }

    #[test]
    fn a_program_gets_its_large_icon() {
        let icon = Canvas::new(32, 32);
        let icons = BTreeMap::from([(
            "mpv".to_string(),
            appicons::AppIcon {
                small: Some(Canvas::new(16, 16)),
                large: Some(icon.clone()),
            },
        )]);
        let m = mix(&snap(), &icons);
        assert_eq!(m.apps[0].icon, None);
        assert_eq!(m.apps[1].icon, Some(icon));
    }

    #[test]
    fn every_sink_in_the_dump_is_an_output() {
        let mut store = pipewire::Store::new();
        store.apply(
            &serde_json::from_str(include_str!(
                "../../../repriseaudio/tests/fixtures/pw-dump.json"
            ))
            .unwrap(),
        );
        let snap = store.snapshot();
        let m = mix(&snap, &BTreeMap::new());
        let labels: Vec<_> = snap.sinks.iter().map(|d| d.label.clone()).collect();
        assert!(!labels.is_empty(), "the dump has a sink");
        assert_eq!(m.outputs, labels);
        assert!(m.output.is_some() && m.master.live);
    }

    #[test]
    fn a_recent_write_stands_in_for_a_lagging_snapshot() {
        let mut s = Sound {
            snapshot: snap(),
            ..Default::default()
        };
        s.recent.note(92, 0.7, Instant::now());
        assert_eq!(s.popup_mix().apps[1].pos, 14);
        s.recent = Default::default();
        s.recent.note(92, 0.7, Instant::now() - pipewire::SETTLE);
        assert_eq!(s.popup_mix().apps[1].pos, 10, "a stale write is forgotten");
    }

    #[test]
    fn a_popup_write_notes_each_node_of_its_channel() {
        use volumepopup::Channel;
        let mut s = Sound {
            snapshot: snap(),
            ..Default::default()
        };
        assert_eq!(
            s.note_volume(&Channel::App("firefox".into()), 0.3),
            [90, 91]
        );
        assert_eq!(s.note_volume(&Channel::Master, 0.15), [41]);
        let m = s.popup_mix();
        assert_eq!((m.master.pos, m.apps[0].pos), (3, 6));
    }

    fn percent(w: Option<(u32, f64)>) -> Option<(u32, i32)> {
        w.map(|(id, v)| (id, (v * 100.0).round() as i32))
    }

    #[test]
    fn the_wheel_and_the_popup_count_on_from_each_others_writes() {
        use volumepopup::Channel;
        let mut s = Sound {
            snapshot: snap(),
            ..Default::default()
        };
        s.note_volume(&Channel::Master, 0.7);
        assert_eq!(
            percent(s.wheel_volume(1)),
            Some((41, 75)),
            "the wheel starts from the popup's write, not the sink's 50 %"
        );
        s.recent.note(41, 0.75, Instant::now());
        assert_eq!(s.popup_mix().master.pos, 15, "the popup shows the wheel's");
    }

    #[test]
    fn a_new_output_shows_its_own_volume() {
        use volumepopup::Channel;
        let mut s = Sound {
            snapshot: snap(),
            ..Default::default()
        };
        s.note_volume(&Channel::Master, 0.15);
        s.snapshot.default_sink = Some("hdmi".into());
        assert_eq!(s.popup_mix().master.pos, 10, "hdmi's own volume");
        assert_eq!(percent(s.wheel_volume(-2)), Some((40, 40)));
    }

    #[test]
    fn the_panel_sits_on_the_taskbar_over_the_icon() {
        // A 30 px taskbar on a 1080-row output: the click on its top edge,
        // at 1050.
        assert_eq!(anchor((1900, 1050), (0, 0)), (1900, 1050));
        // The same on a second output to the right.
        assert_eq!(anchor((1920 + 100, 1050), (1920, 0)), (100, 1050));
    }

    #[test]
    fn under_a_top_taskbar_the_panel_hangs_from_its_bottom() {
        // The same bar along the top: the click on its foot, at 30.
        assert_eq!(anchor((1900, 30), (0, 0)), (1900, 30));
        // An output under another: its own top is the origin.
        assert_eq!(anchor((100, 1080 + 30), (0, 1080)), (100, 30));
    }

    fn test_ctx() -> Ctx {
        Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default())
                .into(),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (0, 0),
            maximized: false,
            origin: (0, 0),
            wake: repriseapp::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            output_size: None,
            at_pick: false,
            text_size: 100,
        }
    }

    /// A wake that shows no window leaves a second instance's handoff for the wake whose raise does.
    #[test]
    fn the_handoff_waits_for_the_wake_that_shows_the_window() {
        let handoff = reprisebus::instance::Handoff {
            token: Some("t".into()),
            by_key: true,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = Sound {
            tray: Some(reprisetray::TrayHandle::holding(handoff.clone())),
            msgs: Some(rx),
            hidden: true,
            ..Sound::default()
        };
        let mut ctx = test_ctx();
        assert_eq!(app.wake(&mut ctx).activate, None);
        tx.send(reprisetray::Rows::clicked(&tray::TrayMsg::Properties))
            .unwrap();
        assert_eq!(
            app.wake(&mut ctx).activate,
            Some((repriseapp::MAIN, handoff))
        );
    }

    #[test]
    fn a_first_click_that_opens_only_the_flyout_drops_its_handoff() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = Sound {
            tray: Some(reprisetray::TrayHandle::holding(
                reprisebus::instance::Handoff {
                    token: Some("t".into()),
                    by_key: false,
                },
            )),
            msgs: Some(rx),
            hidden: true,
            ..Sound::default()
        };
        let mut ctx = test_ctx();
        tx.send(reprisetray::Msg::Activate(0, 0)).unwrap();
        assert_eq!(app.wake(&mut ctx).activate, None);
        assert_eq!(app.tray.as_ref().unwrap().take_handoff(), None);
    }

    #[test]
    fn a_wheel_step_leaves_the_handoff_for_the_raise() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = Sound {
            tray: Some(reprisetray::TrayHandle::holding(
                reprisebus::instance::Handoff::default(),
            )),
            msgs: Some(rx),
            hidden: true,
            ..Sound::default()
        };
        let mut ctx = test_ctx();
        tx.send(reprisetray::Msg::Scroll(1)).unwrap();
        app.wake(&mut ctx);
        assert!(app.tray.as_ref().unwrap().take_handoff().is_some());
    }
}
