//! Work off the UI thread: decodes, scaled copies, folder listings and deletes on one worker,
//! and the timer slideshows and moving pictures step on.

use repriseapp::Poster;
use repriseapp::decode::{Decoders, frames, read_capped_from};
use reprisefs::bincap::Fate;
use reprisekit::raster::Canvas;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Instant;

/// A decoded picture: its frames (one for a still) and the file's size in bytes.
pub struct Picture {
    pub frames: Vec<(Arc<Canvas>, u32)>,
    pub bytes: u64,
}

impl Picture {
    pub fn size(&self) -> (u32, u32) {
        self.frames
            .first()
            .map_or((0, 0), |(c, _)| (c.width, c.height))
    }

    pub fn animated(&self) -> bool {
        self.frames.len() > 1
    }
}

/// What a scaled copy is of: the open picture, its turn and its size on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderKey {
    pub generation: u64,
    pub turns: u8,
    pub size: (i32, i32),
}

pub enum Job {
    Open {
        generation: u64,
        path: PathBuf,
    },
    List(PathBuf),
    Render {
        key: RenderKey,
        picture: Arc<Canvas>,
    },
    Fate {
        path: PathBuf,
        erase: bool,
    },
    Delete {
        path: PathBuf,
        erase: bool,
    },
}

pub enum Msg {
    Opened {
        generation: u64,
        picture: Result<Picture, String>,
    },
    Listed {
        dir: PathBuf,
        pictures: Vec<PathBuf>,
    },
    Rendered {
        key: RenderKey,
        canvas: Canvas,
    },
    /// Whether Delete erases and asks, as the Recycle Bin settings and the drive have it.
    Fate {
        path: PathBuf,
        mode: reprisefs::bincap::DeleteMode,
    },
    Deleted {
        path: PathBuf,
        result: Result<(), String>,
    },
    /// A deadline given to [`Timer::set`] passed.
    Due,
}

/// The message a picture that does not open shows instead.
pub const CANNOT_OPEN: &str = "This picture cannot be shown.";

fn open(path: &std::path::Path) -> Result<Picture, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let bytes = read_capped_from(file).map_err(|e| e.to_string())?;
    let len = bytes.len() as u64;
    if let Some(f) = frames(&bytes) {
        let frames = f
            .into_iter()
            .map(|f| (Arc::new(f.canvas), f.delay_ms))
            .collect();
        return Ok(Picture { frames, bytes: len });
    }
    let d = Decoders::default_set()
        .decode(&bytes, (u32::MAX, u32::MAX))
        .ok_or_else(|| CANNOT_OPEN.to_string())?;
    Ok(Picture {
        frames: vec![(Arc::new(d.canvas), 0)],
        bytes: len,
    })
}

fn fate(path: &std::path::Path, erase: bool) -> reprisefs::bincap::DeleteMode {
    let bins = reprisefs::trash::Bins::system();
    let f = bins.as_ref().map_or(Fate::NoBin, |b| b.fate(path));
    let confirm = bins.as_ref().is_none_or(|b| b.policy.confirm);
    reprisefs::bincap::delete_mode(confirm, &[f], erase)
}

fn delete(path: &std::path::Path, erase: bool) -> Result<(), String> {
    if erase {
        return std::fs::remove_file(path).map_err(|e| e.to_string());
    }
    let bins = reprisefs::trash::Bins::system().ok_or("Cannot find the Recycle Bin.")?;
    bins.trash(path).map(drop).map_err(|e| e.to_string())
}

/// Starts the worker. Jobs run in order, except that only the last of several waiting
/// renders runs.
pub fn spawn(post: Poster<Msg>) -> Sender<Job> {
    let (tx, rx) = channel::<Job>();
    std::thread::Builder::new()
        .name("imageviewer-load".into())
        .spawn(move || work(rx, post))
        .expect("spawn the picture loader");
    tx
}

fn work(rx: Receiver<Job>, post: Poster<Msg>) {
    let mut held: Option<Job> = None;
    loop {
        let job = match held.take() {
            Some(j) => j,
            None => match rx.recv() {
                Ok(j) => j,
                Err(_) => return,
            },
        };
        let job = match job {
            Job::Render { .. } => {
                let mut last = job;
                while let Ok(next) = rx.try_recv() {
                    match next {
                        Job::Render { .. } => last = next,
                        other => {
                            held = Some(other);
                            break;
                        }
                    }
                }
                last
            }
            other => other,
        };
        post.post(run(job));
    }
}

/// Do `job` and say how it went.
pub fn run(job: Job) -> Msg {
    match job {
        Job::Open { generation, path } => Msg::Opened {
            generation,
            picture: open(&path),
        },
        Job::List(dir) => Msg::Listed {
            pictures: crate::folder::list(&dir),
            dir,
        },
        Job::Render { key, picture } => {
            let (w, h) = (key.size.0.max(1) as u32, key.size.1.max(1) as u32);
            Msg::Rendered {
                key,
                canvas: reprisekit::controls::imageview::render(&picture, key.turns, (w, h)),
            }
        }
        Job::Fate { path, erase } => Msg::Fate {
            mode: fate(&path, erase),
            path,
        },
        Job::Delete { path, erase } => Msg::Deleted {
            result: delete(&path, erase),
            path,
        },
    }
}

/// One deadline at a time: [`Msg::Due`] is posted once it passes.
pub struct Timer(Sender<Option<Instant>>);

impl Timer {
    pub fn spawn(post: Poster<Msg>) -> Timer {
        let (tx, rx) = channel::<Option<Instant>>();
        std::thread::Builder::new()
            .name("imageviewer-timer".into())
            .spawn(move || {
                let mut due: Option<Instant> = None;
                loop {
                    let got = match due {
                        None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                        Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                    };
                    match got {
                        Ok(next) => due = next,
                        Err(RecvTimeoutError::Timeout) => {
                            due = None;
                            post.post(Msg::Due);
                        }
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("spawn the viewer's timer");
        Timer(tx)
    }

    /// Post [`Msg::Due`] at `at`, replacing the deadline before; `None` clears it.
    pub fn set(&self, at: Option<Instant>) {
        let _ = self.0.send(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_no_picture_says_it_cannot_be_shown() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("broken.png");
        std::fs::write(&p, b"not a picture").unwrap();
        assert_eq!(open(&p).err().as_deref(), Some(CANNOT_OPEN));
        assert!(open(&dir.path().join("gone.png")).is_err());
    }
}
