use crate::classic::widgets::{ScrollHit, ScrollParts};
use crate::controls::columns::{Hover, Strip, StripEvent, notches};
use crate::controls::combo::{self, Combo};
use crate::controls::form;
use crate::pointer::Pointer;
use crate::trackbar::{self, TrackBar};

use super::layout::{Hit, Layout, bar_parts, hit};
use super::{
    ColumnLayout, Dir, Event, Focus, MAX_SHOWN, Part, Pick, Transport, VolumePopup, combo_focus,
};

impl StripEvent for Event {
    const NONE: Event = Event::None;
    const REDRAW: Event = Event::Redraw;
}

impl Strip for VolumePopup {
    type Part = Part;
    type Focus = Focus;
    type Pick = Pick;
    type Layout = Layout;
    type Event = Event;

    /// Master's column stays while the programs scroll.
    const PINNED: usize = 1;

    fn count(&self) -> usize {
        self.columns.len()
    }
    fn hover_mut(&mut self) -> &mut Hover {
        &mut self.hover
    }
    fn ptr_mut(&mut self) -> &mut Pointer<Part> {
        &mut self.ptr
    }
    fn focus_mut(&mut self) -> &mut Focus {
        &mut self.focus
    }
    fn cues_mut(&mut self) -> &mut bool {
        &mut self.cues
    }
    fn first_mut(&mut self) -> &mut usize {
        &mut self.first
    }
    fn shown(&self, _: &Layout) -> usize {
        MAX_SHOWN
    }

    fn current(&self, lay: &Layout) -> Layout {
        let columns = lay
            .columns
            .iter()
            .zip(&self.columns)
            .map(|(cl, c)| {
                cl.as_ref().map(|cl| ColumnLayout {
                    track: trackbar::at_position(&cl.track, c.volume.pos),
                    ..cl.clone()
                })
            })
            .collect();
        Layout {
            columns,
            ..lay.clone()
        }
    }

    /// Only master greys; a greyed combo lights nothing.
    fn part_at(&self, lay: &Layout, x: i32, y: i32) -> Option<Part> {
        let live = |i: usize| self.columns.get(i).is_some_and(|c| c.volume.enabled);
        match hit(self, lay, x, y) {
            Hit::Track(i, trackbar::Hit::Thumb) if live(i) => Some(Part::Thumb(i)),
            Hit::Mute(i) if live(i) => Some(Part::Mute(i)),
            Hit::Combo(p, _) if self.pick(p).is_some_and(|c| c.enabled) => Some(Part::Combo(p)),
            Hit::Bar(h) => Some(Part::Bar(h)),
            Hit::Media(t) if self.can(t) => Some(Part::Media(t)),
            Hit::Settings => Some(Part::Settings),
            _ => None,
        }
    }

    fn pressed(p: Part) -> bool {
        matches!(p, Part::Bar(_) | Part::Media(_) | Part::Settings)
    }

    fn picks(&self) -> impl Iterator<Item = Pick> {
        let apps = (1..self.columns.len()).map(Pick::App);
        [Pick::Device(Dir::Output), Pick::Device(Dir::Input)]
            .into_iter()
            .chain(apps)
    }

    /// Combo `p`; `None` for a program column without one.
    fn pick(&self, p: Pick) -> Option<&Combo> {
        match p {
            Pick::Device(dir) => Some(self.combo(dir)),
            Pick::App(i) => self.columns.get(i)?.output.as_ref(),
        }
    }

    fn pick_mut(&mut self, p: Pick) -> Option<&mut Combo> {
        match p {
            Pick::Device(Dir::Output) => Some(&mut self.output),
            Pick::Device(Dir::Input) => Some(&mut self.input),
            Pick::App(i) => self.columns.get_mut(i)?.output.as_mut(),
        }
    }

    fn combo_hit(&self, lay: &Layout, x: i32, y: i32) -> Option<(Pick, combo::Hit)> {
        match hit(self, lay, x, y) {
            Hit::Combo(p, h) => Some((p, h)),
            _ => None,
        }
    }

    fn combo_layout(lay: &Layout, p: Pick) -> Option<&combo::Layout> {
        lay.pick(p)
    }

    fn pick_column(p: Pick) -> Option<usize> {
        match p {
            Pick::App(i) => Some(i),
            Pick::Device(_) => None,
        }
    }

    /// A new default device, or a program's output.
    fn picked(&self, p: Pick, i: usize) -> Event {
        match p {
            Pick::Device(dir) => Event::Default(dir, i),
            Pick::App(col) => match &self.columns[col].key {
                Some(k) => Event::AppOutput(k.clone(), i.checked_sub(1)),
                None => Event::None,
            },
        }
    }

    /// Output, Input, each column's thumb, Mute and output combo, the transport buttons, then the
    /// settings link.
    fn focus_order(&self) -> Vec<Focus> {
        let mut v = Vec::new();
        for (f, c) in [(Focus::Output, &self.output), (Focus::Input, &self.input)] {
            if c.enabled {
                v.push(f);
            }
        }
        for (i, c) in self.columns.iter().enumerate() {
            if c.volume.enabled {
                v.extend([Focus::Thumb(i), Focus::Mute(i)]);
            }
            if c.output.as_ref().is_some_and(|o| o.enabled) {
                v.push(Focus::Device(i));
            }
        }
        v.extend(
            Transport::ALL
                .into_iter()
                .filter(|&t| self.can(t))
                .map(Focus::Media),
        );
        v.push(Focus::Settings);
        v
    }

    fn focus_column(f: Focus) -> Option<usize> {
        match f {
            Focus::Thumb(i) | Focus::Mute(i) | Focus::Device(i) => Some(i),
            Focus::Output | Focus::Input | Focus::Media(_) | Focus::Settings => None,
        }
    }

    fn bar(&self, lay: &Layout) -> Option<ScrollParts> {
        lay.bar.map(|bar| bar_parts(self, lay, bar))
    }
}

impl VolumePopup {
    fn can(&self, t: Transport) -> bool {
        self.playing.as_ref().is_some_and(|np| np.can(t))
    }

    pub(super) fn combo(&self, dir: Dir) -> &Combo {
        match dir {
            Dir::Output => &self.output,
            Dir::Input => &self.input,
        }
    }

    fn slide(&mut self, i: usize, f: impl FnOnce(&mut TrackBar) -> bool) -> Event {
        let ch = self.channel(i);
        let t = &mut self.columns[i].volume;
        if t.enabled && f(t) {
            Event::Volume(ch, t.pos)
        } else {
            Event::None
        }
    }

    fn flip(&mut self, i: usize) -> Event {
        if !self.columns[i].volume.enabled {
            return Event::None;
        }
        let ch = self.channel(i);
        let c = &mut self.columns[i];
        c.mute = !c.mute;
        Event::Mute(ch, c.mute)
    }

    /// A reader's Focus on the control at (`x`, `y`): it takes the keyboard as Tab gives it, with
    /// its focus ring shown, without sliding, muting, playing or dropping anything.
    pub fn focus_at(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        let lay = &self.current(lay);
        let f = match hit(self, lay, x, y) {
            Hit::Track(i, _) => Focus::Thumb(i),
            Hit::Mute(i) => Focus::Mute(i),
            Hit::Media(t) => Focus::Media(t),
            Hit::Settings => Focus::Settings,
            Hit::Combo(p, combo::Hit::Field | combo::Hit::Drop) => combo_focus(p),
            _ => return Event::None,
        };
        self.focus_to(f)
    }

    /// The left button went down at (`x`, `y`). The thumb is where the volume puts it now; a press on the channel steps it one position; focus moves to the pressed part; a combo's field or drop button opens its list. With a list open, a press on it acts there and any other press closes it; outside the panel closes the popup.
    pub fn press(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        let lay = &self.current(lay);
        self.hover.point = Some((x, y));
        if let Some(ev) = self.press_open_list(lay, x, y) {
            return ev;
        }
        let part = self.part_at(lay, x, y);
        let sticky = matches!(part, Some(Part::Thumb(_) | Part::Bar(ScrollHit::Thumb)));
        let changed = self.ptr.press(part, sticky);
        let live = |i: usize| self.columns[i].volume.enabled;
        match hit(self, lay, x, y) {
            Hit::Outside => Event::Close,
            Hit::Track(i, _) | Hit::Mute(i) if !live(i) => Event::None,
            Hit::Track(i, h) => {
                self.focus = Focus::Thumb(i);
                match h {
                    trackbar::Hit::Thumb => Event::Redraw,
                    trackbar::Hit::Left => self.slide(i, |t| t.step(-1)),
                    trackbar::Hit::Right => self.slide(i, |t| t.step(1)),
                }
            }
            Hit::Mute(i) => {
                self.focus = Focus::Mute(i);
                Event::Redraw
            }
            Hit::Media(t) if self.can(t) => {
                self.focus = Focus::Media(t);
                Event::Redraw
            }
            Hit::Settings => {
                self.focus = Focus::Settings;
                Event::Redraw
            }
            Hit::Bar(h) => self.press_bar(lay, h, x),
            Hit::Combo(p, combo::Hit::Field | combo::Hit::Drop)
                if self.pick(p).is_some_and(|c| c.enabled) =>
            {
                self.focus = combo_focus(p);
                if let Some(c) = self.pick_mut(p) {
                    c.toggle();
                }
                Event::Redraw
            }
            _ => self.lit(changed),
        }
    }

    /// The pointer moved: a held thumb follows it, then the part under it is found again. A held scrollbar thumb scrolls the programs to follow the pointer's x.
    pub fn motion(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        self.hover.point = Some((x, y));
        let moved = match self.ptr.held() {
            Some(Part::Thumb(i)) => match &lay.columns[i] {
                Some(cl) => {
                    let pos = trackbar::position_at(&cl.track, y);
                    self.slide(i, |t| t.set(pos))
                }
                None => Event::None,
            },
            Some(Part::Bar(ScrollHit::Thumb)) => self.drag_bar(lay, x),
            _ => Event::None,
        };
        let changed = self.ptr.motion(self.part_at(&self.current(lay), x, y));
        match moved {
            Event::None => self.lit(changed),
            ev => ev,
        }
    }

    /// The button came up: on the Mute box it held, the box flips; on the
    /// transport button or the link it held, that one acts.
    pub fn release(&mut self) -> Event {
        let before = self.ptr.lit();
        match self.ptr.release() {
            Some(Part::Mute(i)) => self.flip(i),
            Some(Part::Media(t)) => Event::Media(t),
            Some(Part::Settings) => Event::Settings,
            _ => self.lit_from(before),
        }
    }

    pub fn leave(&mut self) -> Event {
        self.hover.point = None;
        let changed = self.ptr.leave();
        self.lit(changed)
    }

    /// The wheel turned `rows` rows ([`crate::pointer::WHEEL_ROWS`] per notch, positive is down) at (`x`, `y`): over an open list it scrolls it, over the program scrollbar a column per notch, over a column that column (down is quieter), anywhere else master. A turn short of a notch counts as one. The focus stays.
    pub fn scroll(&mut self, lay: &Layout, x: i32, y: i32, rows: i32) -> Event {
        if let Some(ev) = self.scroll_list(lay, x, y, rows) {
            return ev;
        }
        let notches = notches(rows);
        let i = match hit(self, lay, x, y) {
            Hit::Track(i, _) | Hit::Mute(i) | Hit::Column(i) | Hit::Combo(Pick::App(i), _) => i,
            Hit::Bar(_) => return self.scroll_bar(lay, notches),
            _ => 0,
        };
        let ev = self.slide(i, |t| t.step(-notches));
        let lit = self.rehit(lay);
        match ev {
            Event::None => lit,
            ev => ev,
        }
    }

    /// Up / Down, Page Up / Page Down, Home and End move the focused thumb (Up is louder) or pick on the focused combo; Tab and Shift+Tab walk the focus order; Space flips Mute or clicks a transport button or the link (so does Enter); Esc closes. With a list open every key goes to the list. A key on a program scrolled out of view scrolls it back first.
    pub fn key(&mut self, lay: &Layout, key: form::Key) -> Event {
        use form::Key as K;
        let reveal = !matches!(key, K::Tab { .. } | K::Escape);
        self.key_with(lay, key, reveal, |vp, key| match (key, vp.focus) {
            (K::Tab { back }, _) => {
                vp.walk(lay, back);
                Event::Redraw
            }
            (K::Escape, _) => Event::Close,
            (K::Space, Focus::Mute(i)) => vp.flip(i),
            (K::Space | K::Enter, Focus::Media(t)) if vp.can(t) => Event::Media(t),
            (K::Space | K::Enter, Focus::Settings) => Event::Settings,
            (k, Focus::Output) => vp.on_combo(Pick::Device(Dir::Output), |c| c.key(k)),
            (k, Focus::Input) => vp.on_combo(Pick::Device(Dir::Input), |c| c.key(k)),
            (k, Focus::Device(i)) => vp.on_combo(Pick::App(i), |c| c.key(k)),
            (k, Focus::Thumb(i)) => match k {
                K::Up | K::Right => vp.slide(i, |t| t.step(1)),
                K::Down | K::Left => vp.slide(i, |t| t.step(-1)),
                K::PageUp => vp.slide(i, |t| t.step(t.page as i32)),
                K::PageDown => vp.slide(i, |t| t.step(-(t.page as i32))),
                K::Home => vp.slide(i, |t| t.set(t.count - 1)),
                K::End => vp.slide(i, |t| t.set(0)),
                _ => Event::None,
            },
            _ => Event::None,
        })
    }
}
