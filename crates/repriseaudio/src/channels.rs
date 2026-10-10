//! Levels are linear 0..1, the scale `wpctl` shows (the cube root of
//! PipeWire's own channel volumes). Balance runs -1 (left only) to 1 (right
//! only); the quieter side plays at the louder's times `1 - |balance|`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Centre,
}

pub fn side(position: &str) -> Side {
    match position {
        "FL" | "SL" | "RL" | "FLC" | "RLC" | "TFL" | "TRL" | "TSL" | "FLW" | "FLH" | "LLFE" => {
            Side::Left
        }
        "FR" | "SR" | "RR" | "FRC" | "RRC" | "TFR" | "TRR" | "TSR" | "FRW" | "FRH" | "RLFE" => {
            Side::Right
        }
        _ => Side::Centre,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Channels {
    /// Parallel to `levels`; empty when the node reports no map (all centre).
    pub map: Vec<String>,
    pub levels: Vec<f64>,
    /// Each level over the loudest when the node was last heard, so one turned
    /// down to nothing keeps its balance; empty when not known.
    pub shares: Vec<f64>,
}

impl Channels {
    fn side_of(&self, i: usize) -> Side {
        self.map.get(i).map_or(Side::Centre, |p| side(p))
    }

    fn loudest(&self, s: Side) -> Option<f64> {
        (0..self.levels.len())
            .filter(|&i| self.side_of(i) == s)
            .map(|i| self.levels[i])
            .reduce(f64::max)
    }

    pub fn volume(&self) -> f64 {
        self.levels.iter().copied().fold(0.0, f64::max)
    }

    pub fn stereo(&self) -> bool {
        self.loudest(Side::Left).is_some() && self.loudest(Side::Right).is_some()
    }

    pub fn heard_shares(&self) -> Option<Vec<f64>> {
        let loudest = self.volume();
        if loudest > 0.0 {
            Some(self.levels.iter().map(|l| l / loudest).collect())
        } else {
            (!self.levels.is_empty() && self.shares.len() == self.levels.len())
                .then(|| self.shares.clone())
        }
    }

    pub fn balance(&self) -> f64 {
        if self.volume() <= 0.0
            && let Some(shares) = self.heard_shares()
        {
            let heard = Channels {
                map: self.map.clone(),
                levels: shares,
                shares: Vec::new(),
            };
            return heard.balance();
        }
        let (Some(l), Some(r)) = (self.loudest(Side::Left), self.loudest(Side::Right)) else {
            return 0.0;
        };
        if l >= r {
            if l > 0.0 { r / l - 1.0 } else { 0.0 }
        } else {
            1.0 - l / r
        }
    }

    pub fn with_volume(&self, v: f64) -> Vec<f64> {
        match self.heard_shares() {
            Some(shares) => shares.iter().map(|s| s * v).collect(),
            None => vec![v; self.levels.len()],
        }
    }

    pub fn at(&self, v: f64, b: f64) -> Vec<f64> {
        let b = b.clamp(-1.0, 1.0);
        let target = |s: Side| match s {
            Side::Left => v * (1.0 - b.max(0.0)),
            Side::Right => v * (1.0 + b.min(0.0)),
            Side::Centre => v,
        };
        (0..self.levels.len())
            .map(|i| {
                let s = self.side_of(i);
                match self.loudest(s) {
                    Some(m) if m > 0.0 => self.levels[i] / m * target(s),
                    _ => target(s),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo(l: f64, r: f64) -> Channels {
        Channels {
            map: vec!["FL".into(), "FR".into()],
            levels: vec![l, r],
            shares: Vec::new(),
        }
    }

    fn close(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn positions_fall_on_their_side() {
        assert_eq!(side("FL"), Side::Left);
        assert_eq!(side("RR"), Side::Right);
        assert_eq!(side("FC"), Side::Centre);
        assert_eq!(side("LFE"), Side::Centre);
        assert_eq!(side("MONO"), Side::Centre);
        assert_eq!(side("AUX0"), Side::Centre);
    }

    #[test]
    fn balance_is_the_quieter_sides_share_of_the_louder() {
        assert_eq!(stereo(0.8, 0.8).balance(), 0.0);
        assert!(
            (stereo(0.8, 0.4).balance() + 0.5).abs() < 1e-9,
            "left louder is negative"
        );
        assert!((stereo(0.2, 0.8).balance() - 0.75).abs() < 1e-9);
        assert_eq!(stereo(0.0, 0.0).balance(), 0.0, "silence is centred");
        assert_eq!(stereo(0.0, 0.5).balance(), 1.0);
        let mono = Channels {
            map: vec!["MONO".into()],
            levels: vec![0.5],
            shares: Vec::new(),
        };
        assert!(!mono.stereo());
        assert_eq!(mono.balance(), 0.0);
    }

    #[test]
    fn a_volume_write_keeps_the_balance() {
        let ch = stereo(0.8, 0.4);
        let out = ch.with_volume(0.5);
        assert!(close(&out, &[0.5, 0.25]), "{out:?}");
        let after = stereo(out[0], out[1]);
        assert!((after.balance() - ch.balance()).abs() < 1e-9);
        assert!((after.volume() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_volume_write_on_a_silent_node_sets_every_channel() {
        assert_eq!(stereo(0.0, 0.0).with_volume(0.6), [0.6, 0.6]);
    }

    #[test]
    fn balance_sets_the_quieter_side_and_keeps_the_volume() {
        let ch = stereo(0.6, 0.6);
        assert!(close(&ch.at(0.6, -0.5), &[0.6, 0.3]));
        assert!(close(&ch.at(0.6, 0.25), &[0.45, 0.6]));
        assert!(close(&ch.at(0.6, 1.0), &[0.0, 0.6]));
        let back = stereo(0.0, 0.6).at(0.6, 0.0);
        assert!(
            close(&back, &[0.6, 0.6]),
            "a silent side comes back: {back:?}"
        );
    }

    #[test]
    fn surround_keeps_each_sides_shape_and_the_centre_follows_the_volume() {
        let ch = Channels {
            map: ["FL", "FR", "FC", "LFE", "RL", "RR"]
                .map(String::from)
                .to_vec(),
            levels: vec![0.8, 0.8, 0.7, 0.8, 0.4, 0.4],
            shares: Vec::new(),
        };
        let out = ch.at(0.8, 0.5);
        assert!(
            close(&out, &[0.4, 0.8, 0.7, 0.8, 0.2, 0.4]),
            "the rear pair stays half the front: {out:?}"
        );
        let louder = ch.with_volume(1.0);
        assert!(
            close(&louder, &[1.0, 1.0, 0.875, 1.0, 0.5, 0.5]),
            "{louder:?}"
        );
    }
}
