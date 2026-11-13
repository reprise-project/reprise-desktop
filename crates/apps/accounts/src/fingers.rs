//! Your fingerprints through fprintd: what the Fingerprints page reads, and the adds and deletes
//! it starts, each off the loop.

use std::sync::mpsc::{Receiver, TryRecvError};

use reprisebus::fprint::{self, Adding, Found};
use reprisekit::controls::addfinger::{Refusal, Scan, Scanned};
use reprisekit::controls::fingerprints::{Facts, Finger, Reader};

/// Where reads, deletes and adds go; tests give their own.
#[derive(Clone, Copy)]
pub struct Sources {
    pub read: fn() -> Found,
    pub delete: fn(Option<&str>) -> Result<(), fprint::Refusal>,
}

impl Default for Sources {
    /// This computer's fprintd; under test none, so nothing reaches the system bus.
    fn default() -> Sources {
        if cfg!(test) {
            Sources {
                read: || Found::NoService,
                delete: |_| Err(fprint::Refusal::Failed),
            }
        } else {
            Sources {
                read: fprint::read,
                delete: fprint::delete,
            }
        }
    }
}

/// How a delete or an add ended.
pub type Ended = Result<(), fprint::Refusal>;

/// What the add thread sends: its steps, then how it ended.
pub enum Msg {
    Step(Adding),
    Ended(Ended),
}

/// An add under way, started from the dialog of generation `from`. Dropping `stop` stops it.
pub struct Add {
    pub from: u64,
    pub rx: Receiver<Msg>,
    pub stop: fprint::Stop,
}

/// What is known of your fingerprints, and the read, delete and add under way.
#[derive(Default)]
pub struct Fingers {
    pub found: Option<Found>,
    pub reading: Option<Receiver<Found>>,
    pub deleting: Option<Receiver<Ended>>,
    pub add: Option<Add>,
    /// The generation the next Add a Fingerprint dialog opens with.
    pub opened: u64,
}

pub fn refusal(r: fprint::Refusal) -> Refusal {
    match r {
        fprint::Refusal::NotAuthorized => Refusal::NotAuthorized,
        fprint::Refusal::InUse => Refusal::InUse,
        fprint::Refusal::Failed => Refusal::Failed,
    }
}

/// What the dialog makes of one step of an add.
pub enum Step {
    Started,
    Scanned(Scanned),
}

impl From<&Adding> for Step {
    fn from(a: &Adding) -> Step {
        match a {
            Adding::Started => Step::Started,
            Adding::Status { result, .. } => Step::Scanned(Scanned::from_key(result)),
        }
    }
}

impl Fingers {
    /// The page's facts for an account that is `yours`.
    pub fn facts(&self, yours: bool) -> Facts {
        let reader = match &self.found {
            None => Reader::Looking,
            Some(Found::NoService) => Reader::NoService,
            Some(Found::NoReader) => Reader::NoReader,
            Some(Found::Silent) => Reader::Silent,
            Some(Found::Reader(r)) => Reader::Found {
                name: r.name.clone(),
                fingers: Finger::ALL
                    .into_iter()
                    .filter(|f| r.fingers.iter().any(|k| k == f.key()))
                    .collect(),
            },
        };
        Facts {
            reader,
            yours,
            busy: self.deleting.is_some() || self.add.is_some(),
        }
    }

    /// How the reader takes a finger and how many scans it wants; a press of unknown count
    /// before the first read.
    pub fn scan(&self) -> (Scan, u32) {
        match &self.found {
            Some(Found::Reader(r)) if r.swipe => (Scan::Swipe, r.stages),
            Some(Found::Reader(r)) => (Scan::Press, r.stages),
            _ => (Scan::Press, 0),
        }
    }

    /// The read, once it is in.
    pub fn read_landed(&mut self) -> bool {
        let got = match self.reading.as_ref().map(Receiver::try_recv) {
            Some(Ok(found)) => found,
            Some(Err(TryRecvError::Disconnected)) => {
                self.reading = None;
                return false;
            }
            _ => return false,
        };
        self.reading = None;
        self.found = Some(got);
        true
    }

    /// The delete, once it is in.
    pub fn delete_landed(&mut self) -> Option<Ended> {
        let got = match self.deleting.as_ref()?.try_recv() {
            Ok(got) => got,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(fprint::Refusal::Failed),
        };
        self.deleting = None;
        Some(got)
    }

    /// The add's steps in since the last call, with its generation, and how it ended once it
    /// has.
    pub fn add_landed(&mut self) -> Option<(u64, Vec<Step>, Option<Ended>)> {
        let a = self.add.as_ref()?;
        let from = a.from;
        let mut steps = Vec::new();
        let ended = loop {
            match a.rx.try_recv() {
                Ok(Msg::Step(s)) => steps.push(Step::from(&s)),
                Ok(Msg::Ended(r)) => break Some(r),
                Err(TryRecvError::Empty) => break None,
                Err(TryRecvError::Disconnected) => break Some(Err(fprint::Refusal::Failed)),
            }
        };
        if ended.is_some() {
            self.add = None;
        }
        (!steps.is_empty() || ended.is_some()).then_some((from, steps, ended))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn reader(fingers: &[&str]) -> Found {
        Found::Reader(fprint::Reader {
            name: "Virtual reader".into(),
            swipe: false,
            stages: 5,
            fingers: fingers.iter().map(|f| f.to_string()).collect(),
        })
    }

    #[test]
    fn your_fingers_list_in_hand_order_and_unknown_names_drop() {
        let f = Fingers {
            found: Some(reader(&["right-index-finger", "any", "left-thumb"])),
            ..Fingers::default()
        };
        assert_eq!(
            f.facts(true).reader,
            Reader::Found {
                name: "Virtual reader".into(),
                fingers: vec![Finger::LeftThumb, Finger::RightIndex],
            }
        );
        assert_eq!(Fingers::default().facts(true).reader, Reader::Looking);
        assert_eq!(f.scan(), (Scan::Press, 5));
    }

    #[test]
    fn an_add_under_way_tells_its_steps_then_ends_and_frees_the_page() {
        let (tx, rx) = channel();
        let (stop, _) = fprint::Stop::new();
        let mut f = Fingers {
            found: Some(reader(&[])),
            add: Some(Add { from: 3, rx, stop }),
            ..Fingers::default()
        };
        assert!(f.facts(true).busy);
        assert!(f.add_landed().is_none(), "nothing in yet");
        tx.send(Msg::Step(Adding::Started)).unwrap();
        tx.send(Msg::Step(Adding::Status {
            result: "enroll-stage-passed".into(),
            done: false,
        }))
        .unwrap();
        let (from, steps, ended) = f.add_landed().expect("two steps");
        assert_eq!(from, 3);
        assert!(matches!(
            steps[..],
            [Step::Started, Step::Scanned(Scanned::Passed)]
        ));
        assert!(ended.is_none() && f.add.is_some());
        tx.send(Msg::Ended(Err(fprint::Refusal::InUse))).unwrap();
        let (_, _, ended) = f.add_landed().expect("the end");
        assert_eq!(ended, Some(Err(fprint::Refusal::InUse)));
        assert!(!f.facts(true).busy);
    }
}
