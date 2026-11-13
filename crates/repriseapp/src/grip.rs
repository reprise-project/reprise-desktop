//! What a frameless window's own drawing grabs ([`Extended::grip`](crate::Extended::grip)): the
//! pointer's shape over it and the move or resize a left press there starts.

use crate::{BTN_LEFT, Host, PointerInput, WinId};
use reprisekit::frame::{Edges, Grip};
use smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;
use smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager;
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_pointer::WlPointer;

/// The pointer's shape over this program's surfaces, set through cursor-shape-v1.
pub(crate) struct PointerShape {
    manager: Option<CursorShapeManager>,
    device: Option<WpCursorShapeDeviceV1>,
    /// The serial of the pointer's last enter, which a shape names.
    enter: u32,
    /// The shape set since that enter; the compositor shows the default one on each enter.
    shown: Shape,
}

impl PointerShape {
    pub(crate) fn new(manager: Option<CursorShapeManager>) -> PointerShape {
        PointerShape {
            manager,
            device: None,
            enter: 0,
            shown: Shape::Default,
        }
    }

    pub(crate) fn attach(&mut self, pointer: &WlPointer, qh: &QueueHandle<Host>) {
        self.device = (self.manager.as_ref()).map(|m| m.get_shape_device(pointer, qh));
    }

    pub(crate) fn detach(&mut self) {
        if let Some(d) = self.device.take() {
            d.destroy();
        }
    }

    pub(crate) fn entered(&mut self, serial: u32) {
        self.enter = serial;
        self.shown = Shape::Default;
    }

    fn show(&mut self, shape: Shape) {
        if shape == self.shown {
            return;
        }
        if let Some(d) = &self.device {
            d.set_shape(self.enter, shape);
            self.shown = shape;
        }
    }
}

impl Host {
    /// Pointer input `p` on window `win`'s own surface, before the program hears it; true when a
    /// left press on a grip started its move or resize.
    pub(crate) fn grip_pointer(&mut self, win: WinId, p: &PointerInput) -> bool {
        let (at, press) = match *p {
            PointerInput::Motion { x, y } => ((x, y), false),
            PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                ..
            } => ((x, y), true),
            _ => return false,
        };
        let Some(size) = self.slots.iter().find(|s| s.id == win).map(|s| s.size) else {
            return false;
        };
        let grip = self.app.grip(win, at, size);
        self.shape.show(shape_of(grip));
        match grip {
            Some(Grip::Move) if press => self.begin_move(win),
            Some(Grip::Resize(e)) if press => self.begin_resize(win, resize_edge(e)),
            _ => return false,
        }
        true
    }
}

pub(crate) fn resize_edge(e: Edges) -> ResizeEdge {
    ResizeEdge::try_from(e.0).unwrap_or(ResizeEdge::None)
}

pub(crate) fn shape_of(grip: Option<Grip>) -> Shape {
    let Some(Grip::Resize(e)) = grip else {
        return Shape::Default;
    };
    match resize_edge(e) {
        ResizeEdge::Top => Shape::NResize,
        ResizeEdge::Bottom => Shape::SResize,
        ResizeEdge::Left => Shape::WResize,
        ResizeEdge::Right => Shape::EResize,
        ResizeEdge::TopLeft => Shape::NwResize,
        ResizeEdge::TopRight => Shape::NeResize,
        ResizeEdge::BottomLeft => Shape::SwResize,
        ResizeEdge::BottomRight => Shape::SeResize,
        _ => Shape::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_edge_and_corner_resizes_from_its_xdg_edge_under_its_arrow() {
        let (t, b, l, r) = (Edges::TOP, Edges::BOTTOM, Edges::LEFT, Edges::RIGHT);
        for (e, edge, shape) in [
            (t, ResizeEdge::Top, Shape::NResize),
            (b, ResizeEdge::Bottom, Shape::SResize),
            (l, ResizeEdge::Left, Shape::WResize),
            (r, ResizeEdge::Right, Shape::EResize),
            (t | l, ResizeEdge::TopLeft, Shape::NwResize),
            (t | r, ResizeEdge::TopRight, Shape::NeResize),
            (b | l, ResizeEdge::BottomLeft, Shape::SwResize),
            (b | r, ResizeEdge::BottomRight, Shape::SeResize),
        ] {
            assert_eq!(resize_edge(e), edge, "{e:?}");
            assert_eq!(shape_of(Some(Grip::Resize(e))), shape, "{e:?}");
        }
        assert_eq!(shape_of(Some(Grip::Move)), Shape::Default);
        assert_eq!(shape_of(None), Shape::Default);
    }
}
