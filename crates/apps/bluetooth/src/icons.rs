//! The 32 px device icons the lists and sheets show, drawn once per kind.

use std::collections::HashMap;

use reprisekit::art::devicon;
use reprisekit::raster::Canvas;

use crate::bz::Kind;

pub struct Icons(HashMap<devicon::Device, Canvas>);

impl Icons {
    pub fn new() -> Icons {
        Icons(
            devicon::ALL
                .iter()
                .map(|d| (*d, devicon::draw(*d)))
                .collect(),
        )
    }

    pub fn get(&self, kind: Kind) -> &Canvas {
        &self.0[&kind.icon()]
    }
}
