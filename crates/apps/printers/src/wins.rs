//! Windows of one kind open at once, at most one per printer, each under an id of its own.

use repriseapp::WinId;

/// A window that shows one printer.
pub trait ForPrinter {
    fn printer(&self) -> &str;
    fn printer_mut(&mut self) -> &mut String;
}

impl ForPrinter for crate::queue::Queue {
    fn printer(&self) -> &str {
        &self.printer
    }
    fn printer_mut(&mut self) -> &mut String {
        &mut self.printer
    }
}

impl ForPrinter for crate::props::PropsSheet {
    fn printer(&self) -> &str {
        &self.printer
    }
    fn printer_mut(&mut self) -> &mut String {
        &mut self.printer
    }
}

/// Hands out `window-<n>` ids, none twice in a run, so a closed window's id never names a new one.
#[derive(Default)]
pub struct Ids(usize);

impl Ids {
    pub fn next(&mut self) -> WinId {
        let id = repriseapp::window_id(self.0);
        self.0 += 1;
        id
    }
}

/// The open windows in the order they opened.
pub struct Wins<T>(Vec<(WinId, T)>);

impl<T> Default for Wins<T> {
    fn default() -> Self {
        Wins(Vec::new())
    }
}

/// Where [`Wins::open`] left a printer's window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    New(WinId),
    Already(WinId),
}

impl Opened {
    pub fn id(self) -> WinId {
        match self {
            Opened::New(id) | Opened::Already(id) => id,
        }
    }

    /// The window to raise: one already open is somewhere behind; a new one maps on top.
    pub fn raise(self) -> Option<WinId> {
        match self {
            Opened::Already(id) => Some(id),
            Opened::New(_) => None,
        }
    }
}

impl<T: ForPrinter> Wins<T> {
    pub fn of(&self, printer: &str) -> Option<WinId> {
        self.0
            .iter()
            .find(|(_, w)| w.printer() == printer)
            .map(|(id, _)| *id)
    }

    /// `printer`'s window, made by `make` with a fresh id when it has none.
    pub fn open(&mut self, printer: &str, ids: &mut Ids, make: impl FnOnce() -> T) -> Opened {
        if let Some(id) = self.of(printer) {
            return Opened::Already(id);
        }
        let id = ids.next();
        self.0.push((id, make()));
        Opened::New(id)
    }

    pub fn by_printer_mut(&mut self, printer: &str) -> Option<&mut T> {
        self.0
            .iter_mut()
            .find(|(_, w)| w.printer() == printer)
            .map(|(_, w)| w)
    }

    /// Points the window of `old` at `new`.
    pub fn rename(&mut self, old: &str, new: &str) {
        if let Some(w) = self.by_printer_mut(old) {
            *w.printer_mut() = new.to_string();
        }
    }
}

impl<T> Wins<T> {
    pub fn get(&self, win: WinId) -> Option<&T> {
        self.0.iter().find(|(id, _)| *id == win).map(|(_, w)| w)
    }

    pub fn get_mut(&mut self, win: WinId) -> Option<&mut T> {
        self.0.iter_mut().find(|(id, _)| *id == win).map(|(_, w)| w)
    }

    pub fn close(&mut self, win: WinId) {
        self.0.retain(|(id, _)| *id != win);
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&mut T) -> bool) {
        self.0.retain_mut(|(_, w)| keep(w));
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (WinId, &T)> {
        self.0.iter().map(|(id, w)| (*id, w))
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.0.iter_mut().map(|(_, w)| w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct W(String);

    impl ForPrinter for W {
        fn printer(&self) -> &str {
            &self.0
        }
        fn printer_mut(&mut self) -> &mut String {
            &mut self.0
        }
    }

    fn w(name: &str) -> impl FnOnce() -> W + '_ {
        move || W(name.into())
    }

    #[test]
    fn each_printer_gets_its_own_window() {
        let (mut ids, mut wins) = (Ids::default(), Wins::default());
        let a = wins.open("Laser", &mut ids, w("Laser"));
        let b = wins.open("Inkjet", &mut ids, w("Inkjet"));
        assert!(matches!(a, Opened::New(_)) && matches!(b, Opened::New(_)));
        assert_ne!(a.id(), b.id());
        assert_eq!(wins.of("Laser"), Some(a.id()));
        assert_eq!(wins.of("Inkjet"), Some(b.id()));
    }

    #[test]
    fn opening_an_open_printer_again_raises_its_window() {
        let (mut ids, mut wins) = (Ids::default(), Wins::default());
        let a = wins.open("Laser", &mut ids, w("Laser"));
        wins.open("Inkjet", &mut ids, w("Inkjet"));
        let again = wins.open("Laser", &mut ids, || panic!("made a second Laser window"));
        assert_eq!(again, Opened::Already(a.id()));
        assert_eq!(again.raise(), Some(a.id()));
        assert_eq!(a.raise(), None);
        assert_eq!(wins.iter().count(), 2);
    }

    #[test]
    fn a_closed_window_s_id_is_not_given_to_the_next() {
        let (mut ids, mut wins) = (Ids::default(), Wins::default());
        let a = wins.open("Laser", &mut ids, w("Laser")).id();
        wins.close(a);
        assert!(wins.get(a).is_none() && wins.is_empty());
        let b = wins.open("Laser", &mut ids, w("Laser")).id();
        assert_ne!(a, b);
    }

    #[test]
    fn a_rename_keeps_the_window_and_its_id() {
        let (mut ids, mut wins) = (Ids::default(), Wins::default());
        let a = wins.open("Laser", &mut ids, w("Laser")).id();
        wins.rename("Laser", "Office");
        assert_eq!(wins.of("Office"), Some(a));
        assert_eq!(wins.of("Laser"), None);
    }
}
