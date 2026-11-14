use crate::classic::widgets::{ScrollHit, scrollbar_pos_at};
use crate::controls::combo::{self, Combo};
use crate::controls::form;
use crate::pointer::{Interaction, Lit, WHEEL_ROWS};
use crate::trackbar::{self, TrackBar};

use super::layout::{Hit, Layout, bar_parts, hit};
use super::{Dir, Event, Focus, MAX_SHOWN, Part, Pick, Transport, VolumePopup, combo_focus};

impl VolumePopup {
    /// The part at (`x`, `y`) on `lay` laid out by [`Self::current`].
    /// Only master greys; a greyed combo lights nothing.
    fn part_at(&self, lay: &Layout, x: i32, y: i32) -> Option<Part> {
        let live = |i: usize| self.columns.get(i).is_some_and(|c| c.volume.enabled);
        match hit(self, lay, x, y) {
            Hit::Track(i, trackbar::Hit::Thumb) if live(i) => Some(Part::Thumb(i)),
            Hit::Mute(i) if live(i) => Some(Part::Mute(i)),
            Hit::Combo(p, _) if self.pick(p).is_some_and(|c| c.enabled) => Some(Part::Combo(p)),
            Hit::Bar(h) => Some(Part::Bar(h)),
            Hit::Media(t) if self.can(t) => Some(Part::Media(t)),
            _ => None,
        }
    }

    fn can(&self, t: Transport) -> bool {
        self.playing.as_ref().is_some_and(|np| np.can(t))
    }

    pub(super) fn combo(&self, dir: Dir) -> &Combo {
        match dir {
            Dir::Output => &self.output,
            Dir::Input => &self.input,
        }
    }

    /// Combo `p`; `None` for a program column without one.
    pub(super) fn pick(&self, p: Pick) -> Option<&Combo> {
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

    pub(super) fn open_list(&self) -> Option<Pick> {
        let apps = (1..self.columns.len()).map(Pick::App);
        [Pick::Device(Dir::Output), Pick::Device(Dir::Input)]
            .into_iter()
            .chain(apps)
            .find(|&p| self.pick(p).is_some_and(|c| c.open.is_some()))
    }

    fn close_list(&mut self) {
        if let Some(p) = self.open_list()
            && let Some(c) = self.pick_mut(p)
        {
            c.open = None;
        }
    }

    /// The event for row `i` picked on combo `p`: a new default device, or a program's output.
    fn picked(&self, p: Pick, i: usize) -> Event {
        match p {
            Pick::Device(dir) => Event::Default(dir, i),
            Pick::App(col) => match &self.columns[col].key {
                Some(k) => Event::AppOutput(k.clone(), i.checked_sub(1)),
                None => Event::None,
            },
        }
    }

    /// What combo `p`'s act asks: a redraw, or for a pick its event.
    fn act(&self, p: Pick, a: combo::Act) -> Event {
        match a {
            combo::Act::None => Event::None,
            combo::Act::Redraw => Event::Redraw,
            combo::Act::Picked(i) => self.picked(p, i),
        }
    }

    fn press_list(&mut self, p: Pick, h: combo::Hit) -> Event {
        let a = self
            .pick_mut(p)
            .map_or(combo::Act::None, |c| c.press_list(h));
        self.act(p, a)
    }

    fn list_key(&mut self, p: Pick, key: form::Key) -> Event {
        let a = self
            .pick_mut(p)
            .map_or(combo::Act::None, |c| c.list_key(key));
        self.act(p, a)
    }

    fn combo_key(&mut self, p: Pick, key: form::Key) -> Event {
        let a = self
            .pick_mut(p)
            .map_or(combo::Act::None, |c| c.closed_key(key));
        self.act(p, a)
    }

    pub(super) fn rehit(&mut self, lay: &Layout) -> Event {
        let Some((x, y)) = self.point else {
            return Event::None;
        };
        let over = self.part_at(&self.current(lay), x, y);
        let changed = self.ptr.motion(over);
        self.lit(changed)
    }

    /// `Redraw` when what draws changed and the look shows it; a held scrollbar part or transport button shows pressed in every look.
    fn lit(&self, changed: bool) -> Event {
        let bar = matches!(self.ptr.held(), Some(Part::Bar(_) | Part::Media(_)));
        if changed && (self.styled.get() || bar) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    fn lit_from(&self, before: Lit<Part>) -> Event {
        let changed = self.ptr.lit() != before;
        if changed
            && matches!(
                before,
                Some((Part::Bar(_) | Part::Media(_), Interaction::Pressed))
            )
        {
            Event::Redraw
        } else {
            self.lit(changed)
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

    /// The focus order: Output, Input, each column's thumb, Mute and output combo, then
    /// the transport buttons. A greyed combo, column or button is no stop.
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
        v
    }

    fn walk(&mut self, back: bool) {
        let order = self.focus_order();
        let n = order.len();
        if n == 0 {
            return;
        }
        let next = match order.iter().position(|f| *f == self.focus) {
            Some(i) if back => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None if back => n - 1,
            None => 0,
        };
        self.focus = order[next];
        if let Focus::Thumb(i) | Focus::Mute(i) | Focus::Device(i) = self.focus {
            self.reveal(i);
        }
    }

    fn reveal(&mut self, i: usize) {
        if i == 0 {
            return;
        }
        let a = i - 1;
        if a < self.first {
            self.first = a;
        } else if a >= self.first + MAX_SHOWN {
            self.first = a + 1 - MAX_SHOWN;
        }
    }

    fn last_first(&self) -> usize {
        self.columns.len().saturating_sub(1 + MAX_SHOWN)
    }

    fn scroll_bar(&mut self, lay: &Layout, notches: i32) -> Event {
        let first = (self.first as i64 + i64::from(notches)).clamp(0, self.last_first() as i64);
        if first as usize == self.first {
            return Event::None;
        }
        self.first = first as usize;
        // A program's list scrolled out of view would go on taking the keys unseen.
        if let Some(Pick::App(i)) = self.open_list()
            && !(self.first..self.first + MAX_SHOWN).contains(&(i - 1))
        {
            self.close_list();
        }
        self.rehit(lay);
        Event::Redraw
    }

    /// A press on the program scrollbar: an arrow scrolls one column, the track a page; the thumb is held `grab` px from its left edge.
    fn press_bar(&mut self, lay: &Layout, h: ScrollHit, x: i32) -> Event {
        let last = self.last_first();
        self.first = match h {
            ScrollHit::Back => self.first.saturating_sub(1),
            ScrollHit::Forward => (self.first + 1).min(last),
            ScrollHit::PageBack => self.first.saturating_sub(MAX_SHOWN),
            ScrollHit::PageForward => (self.first + MAX_SHOWN).min(last),
            ScrollHit::Thumb => {
                if let Some(bar) = lay.bar
                    && let Some(thumb) = bar_parts(self, lay, bar).thumb
                {
                    self.grab = x - thumb.x;
                }
                self.first
            }
        };
        Event::Redraw
    }

    /// The left button went down at (`x`, `y`). The thumb is where the volume puts it now; a press on the channel steps it one position; focus moves to the pressed part; a combo's field or drop button opens its list. With a list open, a press on it acts there and any other press closes it; outside the panel closes the popup.
    /// A reader's Focus on the control at (`x`, `y`): it takes the keyboard as Tab gives it, with
    /// its focus ring shown, without sliding, muting, playing or dropping anything.
    pub fn focus_at(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        let lay = &self.current(lay);
        let live = |i: usize| self.columns[i].volume.enabled;
        let f = match hit(self, lay, x, y) {
            Hit::Track(i, _) if live(i) => Focus::Thumb(i),
            Hit::Mute(i) if live(i) => Focus::Mute(i),
            Hit::Media(t) if self.can(t) => Focus::Media(t),
            Hit::Combo(p, combo::Hit::Field | combo::Hit::Drop)
                if self.pick(p).is_some_and(|c| c.enabled) =>
            {
                combo_focus(p)
            }
            _ => return Event::None,
        };
        let cued = !self.cues;
        self.cues = true;
        if self.focus == f {
            return if cued { Event::Redraw } else { Event::None };
        }
        self.close_list();
        self.focus = f;
        Event::Redraw
    }

    pub fn press(&mut self, lay: &Layout, x: i32, y: i32) -> Event {
        let lay = &self.current(lay);
        self.point = Some((x, y));
        if let Some(d) = self.open_list() {
            if let Hit::Combo(hd, h) = hit(self, lay, x, y)
                && hd == d
            {
                return self.press_list(d, h);
            }
            self.close_list();
            // Nothing under it acts at the release either.
            self.ptr.clear();
            return Event::Redraw;
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
        self.point = Some((x, y));
        let moved = match self.ptr.held() {
            Some(Part::Thumb(i)) => match &lay.columns[i] {
                Some(cl) => {
                    let pos = trackbar::position_at(&cl.track, y);
                    self.slide(i, |t| t.set(pos))
                }
                None => Event::None,
            },
            Some(Part::Bar(ScrollHit::Thumb)) => match lay.bar {
                Some(bar) => {
                    let p = bar_parts(self, lay, bar);
                    let apps = self.columns.len() - 1;
                    let to = scrollbar_pos_at(&p, x - self.grab, MAX_SHOWN, apps, true);
                    if to == self.first {
                        Event::None
                    } else {
                        self.first = to;
                        Event::Redraw
                    }
                }
                None => Event::None,
            },
            _ => Event::None,
        };
        let changed = self.ptr.motion(self.part_at(&self.current(lay), x, y));
        match moved {
            Event::None => self.lit(changed),
            ev => ev,
        }
    }

    /// The button came up: on the Mute box it held, the box flips; on the
    /// transport button it held, that button acts.
    pub fn release(&mut self) -> Event {
        let before = self.ptr.lit();
        match self.ptr.release() {
            Some(Part::Mute(i)) => self.flip(i),
            Some(Part::Media(t)) => Event::Media(t),
            _ => self.lit_from(before),
        }
    }

    pub fn leave(&mut self) -> Event {
        self.point = None;
        let changed = self.ptr.leave();
        self.lit(changed)
    }

    /// The wheel turned `rows` rows ([`WHEEL_ROWS`] per notch, positive is down) at (`x`, `y`): over an open list it scrolls it, over the program scrollbar a column per notch, over a column that column (down is quieter), anywhere else master. A turn short of a notch counts as one. The focus stays.
    pub fn scroll(&mut self, lay: &Layout, x: i32, y: i32, rows: i32) -> Event {
        if let Some(d) = self.open_list()
            && lay.pick(d).is_some_and(|cl| cl.list.contains(x, y))
        {
            return if self.pick_mut(d).is_some_and(|c| c.scroll(rows)) {
                Event::Redraw
            } else {
                Event::None
            };
        }
        let notches = rows.signum() * (rows.abs() / WHEEL_ROWS).max(1);
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

    /// Up / Down, Page Up / Page Down, Home and End move the focused thumb (Up is louder) or pick on the focused combo; Tab and Shift+Tab walk the focus order; Space flips Mute or clicks a transport button (so does Enter); Esc closes. With a list open every key goes to the list. A key on a program scrolled out of view scrolls it back first.
    pub fn key(&mut self, lay: &Layout, key: form::Key) -> Event {
        let first = !self.cues;
        self.cues = true;
        if let Some(d) = self.open_list() {
            return self.list_key(d, key);
        }
        let shown = self.first;
        if let Focus::Thumb(i) | Focus::Mute(i) | Focus::Device(i) = self.focus
            && !matches!(key, form::Key::Tab { .. } | form::Key::Escape)
        {
            self.reveal(i);
        }
        let ev = match (key, self.focus) {
            (form::Key::Tab { back }, _) => {
                self.walk(back);
                Event::Redraw
            }
            (form::Key::Escape, _) => return Event::Close,
            (form::Key::Space, Focus::Mute(i)) => self.flip(i),
            (form::Key::Space | form::Key::Enter, Focus::Media(t)) if self.can(t) => {
                Event::Media(t)
            }
            (k, Focus::Output) => self.combo_key(Pick::Device(Dir::Output), k),
            (k, Focus::Input) => self.combo_key(Pick::Device(Dir::Input), k),
            (k, Focus::Device(i)) => self.combo_key(Pick::App(i), k),
            (k, Focus::Thumb(i)) => match k {
                form::Key::Up | form::Key::Right => self.slide(i, |t| t.step(1)),
                form::Key::Down | form::Key::Left => self.slide(i, |t| t.step(-1)),
                form::Key::PageUp => self.slide(i, |t| t.step(t.page as i32)),
                form::Key::PageDown => self.slide(i, |t| t.step(-(t.page as i32))),
                form::Key::Home => self.slide(i, |t| t.set(t.count - 1)),
                form::Key::End => self.slide(i, |t| t.set(0)),
                _ => Event::None,
            },
            _ => Event::None,
        };
        let lit = self.rehit(lay);
        // The first key shows the focus ring even when it moves nothing.
        match ev {
            Event::None if first || self.first != shown => Event::Redraw,
            Event::None => lit,
            ev => ev,
        }
    }
}
