//! The night light on an output whose CRTC has no gamma table (a virtual
//! GPU): a last pass multiplies each channel by its gain, which is what a
//! linear ramp does (`color::ramp`). The pass sits in front of everything,
//! so screenshots and recordings of such an output carry the tint. Gains
//! live in the output's user data, where `render` finds them.

use crate::mutex::lock;
use std::sync::Mutex;

use smithay::{
    backend::renderer::{
        Color32F,
        element::{Element, Id, RenderElement},
        gles::{GlesError, GlesFrame, GlesRenderer, ffi},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    output::Output,
    utils::{Buffer, Physical, Point, Rectangle, Scale},
};

struct Tint {
    id: Id,
    commit: CommitCounter,
    gains: Option<[f32; 3]>,
}

fn with<T>(output: &Output, f: impl FnOnce(&mut Tint) -> T) -> T {
    let data = output.user_data();
    data.insert_if_missing_threadsafe(|| {
        Mutex::new(Tint {
            id: Id::new(),
            commit: CommitCounter::default(),
            gains: None,
        })
    });
    let tint = data.get::<Mutex<Tint>>().expect("inserted above");
    f(&mut lock(tint))
}

/// Tint `output` by `gains` (red, green, blue, 0 to 1), or stop with None. Returns whether anything changed.
pub fn set(output: &Output, gains: Option<[f32; 3]>) -> bool {
    with(output, |t| {
        if t.gains == gains {
            return false;
        }
        t.gains = gains;
        t.commit.increment();
        true
    })
}

/// The pass for `output`, with `origin` (physical pixels from its top-left) at 0,0; None when untinted.
pub fn element(output: &Output, origin: Point<i32, Physical>) -> Option<TintElement> {
    let tint = output.user_data().get::<Mutex<Tint>>()?;
    let (id, commit, gains) = {
        let t = lock(tint);
        (t.id.clone(), t.commit, t.gains?)
    };
    let size = output
        .current_transform()
        .transform_size(output.current_mode()?.size);
    Some(TintElement {
        id,
        commit,
        geo: Rectangle::new(Point::default() - origin, size),
        gains,
    })
}

pub struct TintElement {
    id: Id,
    commit: CommitCounter,
    geo: Rectangle<i32, Physical>,
    gains: [f32; 3],
}

impl Element for TintElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size((f64::from(self.geo.size.w), f64::from(self.geo.size.h)).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geo
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }
}

impl RenderElement<GlesRenderer> for TintElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), GlesError> {
        // SAFETY: plain GL state calls on the frame's current context; the blend function is restored before return.
        frame.with_context(|gl| unsafe {
            gl.BlendFuncSeparate(ffi::ZERO, ffi::SRC_COLOR, ffi::ZERO, ffi::ONE)
        })?;
        let [r, g, b] = self.gains;
        let drawn = frame.draw_solid(dst, damage, Color32F::new(r, g, b, 0.5));
        // SAFETY: as above.
        frame.with_context(|gl| unsafe { gl.BlendFunc(ffi::ONE, ffi::ONE_MINUS_SRC_ALPHA) })?;
        drawn
    }
}

/// The gains a client's ramp asks for: each channel's last entry. Exact for linear ramps, the usual kind, nearest multiply otherwise.
pub fn gains_of(ramp: &[u16]) -> Option<[f32; 3]> {
    let size = ramp.len() / 3;
    if size == 0 {
        return None;
    }
    let end = |c: usize| f32::from(ramp[c * size + size - 1]) / f32::from(u16::MAX);
    Some([end(0), end(1), end(2)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_ramp_tints_by_each_channel_s_end() {
        let ramp = [0, 65535, 0, 32767, 0, 0];
        let g = gains_of(&ramp).unwrap();
        assert_eq!(g[0], 1.0);
        assert!((g[1] - 0.5).abs() < 0.001, "{g:?}");
        assert_eq!(g[2], 0.0);
        assert_eq!(gains_of(&[]), None);
    }
}
