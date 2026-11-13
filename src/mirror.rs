//! Displays that copy another (Duplicate): an output placed at another's top-left shows that
//! one's box, the primary's when it is among them. Its layout box becomes the copied one's
//! (`reprisescene::set_copied`), so windows, the pointer and the shell see a single display; when
//! its frame differs in size it draws the copied output's frame fitted and letterboxed.

use std::collections::HashMap;

use smithay::{
    backend::renderer::{
        Color32F, Renderer,
        element::{Id, Kind, texture::TextureRenderElement},
        gles::GlesRenderer,
        utils::DamageBag,
    },
    output::Output,
    utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::compositor::State;
use crate::offscreen::{Target, physical_size};
use crate::render::{OutputElement, Parts};

/// For outputs whose top-lefts are `places` (in the scene's order), the index of the one each
/// copies: outputs sharing a top-left copy the `primary` one when it is there, else the first.
pub fn sources(places: &[(i32, i32)], primary: Option<usize>) -> Vec<Option<usize>> {
    (0..places.len())
        .map(|i| {
            let same: Vec<usize> = (0..places.len())
                .filter(|&j| places[j] == places[i])
                .collect();
            let src = primary.filter(|p| same.contains(p)).unwrap_or(same[0]);
            (src != i).then_some(src)
        })
        .collect()
}

/// Where a `source` frame lands on a `target` frame, both physical: as large as fits with its
/// shape kept, centred.
pub fn fit(source: Size<i32, Physical>, target: Size<i32, Physical>) -> Rectangle<i32, Physical> {
    if source.w <= 0 || source.h <= 0 {
        return Rectangle::from_size(target);
    }
    let f =
        (f64::from(target.w) / f64::from(source.w)).min(f64::from(target.h) / f64::from(source.h));
    let w = ((f64::from(source.w) * f).round() as i32).min(target.w);
    let h = ((f64::from(source.h) * f).round() as i32).min(target.h);
    Rectangle::new(
        ((target.w - w) / 2, (target.h - h) / 2).into(),
        (w, h).into(),
    )
}

/// `output`'s frame in physical pixels, turned as it is laid out.
fn frame_size(output: &Output) -> Option<Size<i32, Physical>> {
    let mode = output.current_mode()?;
    Some(output.current_transform().transform_size(mode.size))
}

/// The copies drawn fitted, by the copying output's name: the copied frame, kept for its damage.
#[derive(Default)]
pub struct CopyFrames(HashMap<String, CopyFrame>);

struct CopyFrame {
    target: Target,
    id: Id,
    size: Size<i32, Physical>,
    scale: f64,
    damage: DamageBag<i32, Buffer>,
}

const DAMAGE_KEPT: usize = 8;

impl State {
    /// After the outputs are placed: mark each one at another's top-left as copying it, and the
    /// rest as showing their own box. Remote screens never copy.
    pub(crate) fn sync_copies(&mut self) {
        let outputs: Vec<Output> = (self.scene.outputs().iter())
            .filter(|o| !crate::virtualout::is_remote(o))
            .cloned()
            .collect();
        for o in &outputs {
            reprisescene::set_copied(o, None);
        }
        let primary = (self.saved_primary().map(str::to_string))
            .or_else(|| self.config.primary_output_name().map(str::to_string));
        let places: Vec<(i32, i32)> = (outputs.iter())
            .map(|o| (o.current_location().x, o.current_location().y))
            .collect();
        let primary = primary.and_then(|p| outputs.iter().position(|o| o.name() == p));
        let mut copies = Vec::new();
        for (i, src) in sources(&places, primary).into_iter().enumerate() {
            let Some(src) = src else {
                continue;
            };
            let area = reprisescene::output_rect(&outputs[src]);
            reprisescene::set_copied(&outputs[i], area);
            copies.push((outputs[i].clone(), outputs[src].clone()));
        }
        for (copy, src) in &copies {
            if !self.copies.iter().any(|(c, s)| c == copy && s == src) {
                log::info!("output {}: copies {}", copy.name(), src.name());
            }
        }
        self.copy_frames
            .0
            .retain(|name, _| copies.iter().any(|(c, _)| c.name() == *name));
        self.copies = copies;
    }

    /// The outputs in the layout with a desktop of their own: those that copy none.
    pub fn own_outputs(&self) -> Vec<Output> {
        (self.scene.outputs().iter())
            .filter(|o| reprisescene::copied(o).is_none())
            .cloned()
            .collect()
    }

    /// The output `output` copies.
    pub fn copy_source(&self, output: &Output) -> Option<&Output> {
        (self.copies.iter())
            .find(|(c, _)| c == output)
            .map(|(_, s)| s)
    }

    /// The output `output` copies when its frame is not the copied box at its own scale, so it
    /// draws that output's frame fitted ([`elements`]).
    pub fn fitted_source(&self, output: &Output) -> Option<Output> {
        let (_, src) = self.copies.iter().find(|(c, _)| c == output)?;
        let own = frame_size(output)?;
        let area = reprisescene::output_rect(src)?;
        let at_scale = physical_size(area.size, output.current_scale().fractional_scale());
        (at_scale != own).then(|| src.clone())
    }
}

/// What a copy of a different size shows: `source`'s frame (its scene, and its pointer when
/// `with_cursor`, drawn into a texture at its own size and scale) fitted onto `target`'s from
/// `origin`, the bands left to the clear colour.
pub fn elements(
    renderer: &mut GlesRenderer,
    frames: &mut CopyFrames,
    parts: Parts<'_>,
    (source, target): (&Output, &Output),
    origin: Point<i32, Physical>,
    with_cursor: bool,
) -> Vec<OutputElement> {
    let (Some(size), Some(own)) = (frame_size(source), frame_size(target)) else {
        return Vec::new();
    };
    let scale = source.current_scale().fractional_scale();
    let name = target.name();
    let frame = match frames.0.remove(&name) {
        Some(f) if f.size == size && f.scale == scale => f,
        _ => match Target::new(renderer, size, scale) {
            Ok(t) => CopyFrame {
                target: t.clear_to(Color32F::BLACK),
                id: Id::new(),
                size,
                scale,
                damage: DamageBag::new(DAMAGE_KEPT),
            },
            Err(e) => {
                log::warn!("{name}: no copy of {}: {e}", source.name());
                return Vec::new();
            }
        },
    };
    let frame = frames.0.entry(name).or_insert(frame);
    let inner = crate::render::output_elements(renderer, parts, source, (0, 0).into(), with_cursor);
    match frame.target.render(renderer, &inner) {
        Ok(damage) => frame.damage.add(damage),
        Err(e) => log::warn!("{}: copy of {}: {e}", target.name(), source.name()),
    }
    drop(inner);
    let at = fit(size, own);
    let target_scale = target.current_scale().fractional_scale();
    let shown: Size<f64, Logical> = (
        f64::from(at.size.w) / target_scale,
        f64::from(at.size.h) / target_scale,
    )
        .into();
    let whole = Rectangle::from_size((size.w, size.h).into());
    let texture = frame.target.texture();
    vec![OutputElement::Zoom(
        TextureRenderElement::from_texture_with_damage(
            frame.id.clone(),
            renderer.context_id(),
            (at.loc - origin).to_f64(),
            texture.clone(),
            1,
            Transform::Normal,
            None,
            Some(Rectangle::from_size(
                (f64::from(size.w), f64::from(size.h)).into(),
            )),
            Some(shown.to_i32_round()),
            Some(vec![whole]),
            frame.damage.snapshot(),
            Kind::Unspecified,
        ),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::renderer::element::Element;
    use smithay::utils::Scale;

    use crate::harness::Harness;
    use crate::output::HeadRequest;

    fn at(h: &mut Harness, o: &Output, place: (i32, i32)) {
        let req = HeadRequest {
            enabled: true,
            mode: None,
            position: Some(place),
            transform: Transform::Normal,
            scale: 1.0,
            adaptive_sync: false,
        };
        assert!(h.state.apply_head(o, &req));
        h.state.update_layout();
        h.dispatch();
    }

    /// DP-1 at 1920x1080 and DP-2 of `size` both placed at 0,0, as an output tool would.
    fn copied(size: (i32, i32)) -> (Harness, Output, Output) {
        let mut h = Harness::new();
        let a = h.head("DP-1", 1920, 1080);
        let b = h.head("DP-2", size.0, size.1);
        at(&mut h, &a, (0, 0));
        at(&mut h, &b, (0, 0));
        (h, a, b)
    }

    #[test]
    fn a_copy_shows_its_sources_box_with_one_taskbar() {
        let (h, a, b) = copied((1920, 1080));
        assert_eq!(h.state.layout_box(Some(&b)), h.state.layout_box(Some(&a)));
        assert_eq!(h.state.own_outputs(), std::slice::from_ref(&a));
        assert_eq!(h.state.bars.len(), 1, "the copy has no taskbar of its own");
        assert_eq!(h.state.fitted_source(&b), None, "same size: drawn as it is");
        assert_eq!(
            h.state.usable_area(Some(&b)),
            h.state.usable_area(Some(&a)),
            "the copy leaves room for the taskbar it shows"
        );
        assert_eq!(h.state.output_at(10.0, 10.0), Some(a));
    }

    #[test]
    fn a_copy_of_another_size_takes_the_copied_box_and_draws_it_fitted() {
        let (h, a, b) = copied((1280, 1024));
        assert_eq!(
            h.state.layout_box(None),
            h.state.layout_box(Some(&a)),
            "the pointer and windows see one 1920x1080 display"
        );
        assert_eq!(h.state.layout_box(Some(&b)).width, 1920);
        assert_eq!(h.state.fitted_source(&b), Some(a));
    }

    #[test]
    fn a_capture_of_a_copy_of_another_size_shows_the_fitted_frame_on_black() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let (mut h, _, b) = copied((1280, 1024));
        let mut shot = |origin: (i32, i32)| {
            let s = &mut h.state;
            let fitted = s.fitted_source(&b);
            let parts = Parts {
                scene: &s.scene,
                cursor: &s.cursor,
                cursors: &mut s.cursors,
                pointer: Point::default(),
            };
            let shown = (&b, fitted.as_ref());
            let white = Color32F::new(1.0, 1.0, 1.0, 1.0);
            let (elements, clear) = crate::capture::scene_elements(
                &mut renderer,
                &mut s.copy_frames,
                parts,
                shown,
                origin.into(),
                false,
                white,
            );
            let at: Vec<_> = (elements.iter())
                .map(|e| e.geometry(Scale::from(1.0)))
                .collect();
            (at, clear)
        };
        let r = |x, y, w, h| Rectangle::<i32, Physical>::new((x, y).into(), (w, h).into());
        let (at, clear) = shot((0, 0));
        assert_eq!(
            at,
            [r(0, 152, 1280, 720)],
            "1920x1080 fitted into 1280x1024"
        );
        assert_eq!(clear, Color32F::BLACK, "black bands");
        let (at, _) = shot((0, 100));
        assert_eq!(at, [r(0, 52, 1280, 720)], "a region from 0,100");
    }

    #[test]
    fn a_copy_moved_away_shows_its_own_box_again() {
        let (mut h, a, b) = copied((1280, 1024));
        at(&mut h, &b, (1920, 0));
        assert_eq!(h.state.own_outputs(), [a, b.clone()]);
        assert_eq!(h.state.bars.len(), 2);
        assert_eq!(h.state.layout_box(Some(&b)).width, 1280);
    }

    #[test]
    fn outputs_at_one_top_left_copy_the_primary_else_the_first() {
        let places = [(0, 0), (0, 0), (1920, 0), (0, 0)];
        assert_eq!(sources(&places, None), [None, Some(0), None, Some(0)]);
        assert_eq!(
            sources(&places, Some(3)),
            [Some(3), Some(3), None, None],
            "the primary is copied"
        );
        assert_eq!(
            sources(&places, Some(2)),
            [None, Some(0), None, Some(0)],
            "a primary elsewhere copies nothing"
        );
        assert_eq!(sources(&[(0, 0), (1920, 0)], None), [None, None]);
    }

    #[test]
    fn a_copy_fits_the_frame_whole_and_centres_it_between_bands() {
        let r = |x, y, w, h| Rectangle::<i32, Physical>::new((x, y).into(), (w, h).into());
        assert_eq!(
            fit((1920, 1080).into(), (1024, 768).into()),
            r(0, 96, 1024, 576),
            "bands above and below"
        );
        assert_eq!(
            fit((1280, 1024).into(), (1920, 1080).into()),
            r(285, 0, 1350, 1080),
            "bands at the sides"
        );
        assert_eq!(
            fit((2560, 1440).into(), (1920, 1080).into()),
            r(0, 0, 1920, 1080)
        );
    }
}
