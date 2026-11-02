//! `$XDG_STATE_HOME/reprise/widgets.toml`: each widget's options, the monitor it stands on, and
//! its top-left cell on each monitor it has stood on, in 100% cells (`desktop::to_saved`).

use std::collections::BTreeMap;

use reprisekit::controls::deskwidget::{Face, Kind, MeterStyle};
use serde::{Deserialize, Serialize};

pub fn path() -> std::path::PathBuf {
    #[cfg(test)]
    return crate::harness::scratch().join("widgets.toml");
    #[cfg(not(test))]
    repriseconf::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("widgets.toml")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Store {
    pub face: Face,
    pub seconds: bool,
    pub meter: MeterStyle,
    /// The monitor id (`outputlayouts::monitor_ids`) each widget stands on.
    pub homes: BTreeMap<Kind, String>,
    /// Each widget's top-left cell by monitor id.
    pub cells: BTreeMap<Kind, BTreeMap<String, (i32, i32)>>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileStore {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clock: Option<FileWidget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    calendar: Option<FileWidget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    meter: Option<FileWidget>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileWidget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    face: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seconds: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    monitor: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    cells: BTreeMap<String, [i32; 2]>,
}

impl Store {
    /// The file's text read; text that is not TOML, and a value that does not parse, read as the
    /// defaults.
    pub fn parse(text: &str) -> Store {
        let file: FileStore = toml::from_str(text).unwrap_or_else(|e| {
            log::warn!("widgets.toml: {e}; widgets start over");
            FileStore::default()
        });
        let mut store = Store::default();
        let widgets = [
            (Kind::Clock, file.clock),
            (Kind::Calendar, file.calendar),
            (Kind::Meter, file.meter),
        ];
        for (kind, w) in widgets {
            let Some(w) = w else {
                continue;
            };
            match kind {
                Kind::Clock => {
                    store.face = (w.face.as_deref())
                        .and_then(Face::from_name)
                        .unwrap_or_default();
                    store.seconds = w.seconds.unwrap_or_default();
                }
                Kind::Meter => {
                    store.meter = (w.style.as_deref())
                        .and_then(MeterStyle::from_name)
                        .unwrap_or_default();
                }
                Kind::Calendar => {}
            }
            if let Some(m) = w.monitor {
                store.homes.insert(kind, m);
            }
            if !w.cells.is_empty() {
                let cells = w.cells.into_iter().map(|(id, [c, r])| (id, (c, r)));
                store.cells.insert(kind, cells.collect());
            }
        }
        store
    }

    pub fn render(&self) -> String {
        let widget = |kind: Kind| {
            let mut w = FileWidget {
                monitor: self.homes.get(&kind).cloned(),
                cells: (self.cells.get(&kind).into_iter().flatten())
                    .map(|(id, &(c, r))| (id.clone(), [c, r]))
                    .collect(),
                ..FileWidget::default()
            };
            match kind {
                Kind::Clock => {
                    w.face = Some(self.face.name().into());
                    w.seconds = Some(self.seconds);
                }
                Kind::Meter => w.style = Some(self.meter.name().into()),
                Kind::Calendar => {}
            }
            Some(w)
        };
        let file = FileStore {
            clock: widget(Kind::Clock),
            calendar: widget(Kind::Calendar),
            meter: widget(Kind::Meter),
        };
        toml::to_string(&file).unwrap_or_default()
    }

    /// The file as it stands, or the defaults when there is none or it cannot be read.
    pub fn load() -> Store {
        match std::fs::read_to_string(path()) {
            Ok(text) => Store::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(e) => {
                log::warn!("widgets.toml: {e}; widgets start over");
                Store::default()
            }
        }
    }

    /// Write the store on the state writer thread.
    pub fn save(&self) {
        let text = self.render();
        crate::state::run_in_order(move || {
            let path = path();
            if let Err(e) =
                repriseconf::edit_or_reset(&path, |_| Ok::<_, std::convert::Infallible>(text))
            {
                log::warn!("{}: {e}", path.display());
            }
        });
    }
}
