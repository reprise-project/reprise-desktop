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

/// The pointer's shape over this program's surfaces, set through cursor-shape-v1: a grip's
/// under it, else the one the program's last reply to motion asked for
/// ([`Reply::pointer`](crate::Reply::pointer)).
pub(crate) struct PointerShape {
    manager: Option<CursorShapeManager>,
    device: Option<Box<dyn ShapeDevice>>,
    /// The serial of the pointer's last enter, which a shape names.
    enter: u32,
    /// The shape set since that enter; the compositor shows the default one on each enter.
    shown: Shape,
    grip: Option<Shape>,
    asked: Shape,
    /// The pointer is on a window's own surface, not on a menu popup the host shows for it.
    pub(crate) on_window: bool,
    /// The window the event being handled is pointer motion for: its reply picks the shape.
    pub(crate) motion_for: Option<WinId>,
}

impl PointerShape {
    pub(crate) fn new(manager: Option<CursorShapeManager>) -> PointerShape {
        PointerShape {
            manager,
            device: None,
            enter: 0,
            shown: Shape::Default,
            grip: None,
            asked: Shape::Default,
            on_window: false,
            motion_for: None,
        }
    }

    pub(crate) fn attach(&mut self, pointer: &WlPointer, qh: &QueueHandle<Host>) {
        self.device = (self.manager.as_ref())
            .map(|m| Box::new(m.get_shape_device(pointer, qh)) as Box<dyn ShapeDevice>);
    }

    pub(crate) fn detach(&mut self) {
        if let Some(d) = self.device.take() {
            d.destroy();
        }
    }

    pub(crate) fn entered(&mut self, serial: u32) {
        self.enter = serial;
        self.shown = Shape::Default;
        self.grip = None;
        self.asked = Shape::Default;
    }

    fn set_grip(&mut self, grip: Option<Shape>) {
        self.grip = grip;
        self.show(grip.unwrap_or(self.asked));
    }

    /// The program asked for `shape` in its reply to motion.
    pub(crate) fn ask(&mut self, shape: reprisekit::pointer::Shape) {
        self.asked = wire(shape);
        self.show(self.grip.unwrap_or(self.asked));
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

/// The seat's cursor-shape device, or a test's record of what it was sent.
trait ShapeDevice {
    fn set_shape(&self, serial: u32, shape: Shape);
    fn destroy(&self);
}

impl ShapeDevice for WpCursorShapeDeviceV1 {
    fn set_shape(&self, serial: u32, shape: Shape) {
        WpCursorShapeDeviceV1::set_shape(self, serial, shape);
    }
    fn destroy(&self) {
        WpCursorShapeDeviceV1::destroy(self);
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
        self.shape.set_grip(grip.map(|g| shape_of(Some(g))));
        match grip {
            Some(Grip::Move) if press => self.begin_move(win),
            Some(Grip::Resize(e)) if press => self.begin_resize(win, resize_edge(e)),
            _ => return false,
        }
        true
    }
}

/// `shape` as cursor-shape-v1 names it.
pub(crate) fn wire(shape: reprisekit::pointer::Shape) -> Shape {
    use reprisekit::pointer::Shape as S;
    match shape {
        S::Default => Shape::Default,
        S::Hand => Shape::Pointer,
        S::Text => Shape::Text,
        S::ColResize => Shape::ColResize,
        S::RowResize => Shape::RowResize,
        S::EResize => Shape::EResize,
        S::SResize => Shape::SResize,
        S::SeResize => Shape::SeResize,
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
    use reprisekit::pointer::Shape as Kit;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Default)]
    struct Sent(Rc<RefCell<Vec<(u32, Shape)>>>);

    impl ShapeDevice for Sent {
        fn set_shape(&self, serial: u32, shape: Shape) {
            self.0.borrow_mut().push((serial, shape));
        }
        fn destroy(&self) {}
    }

    fn shape_with(sent: &Sent) -> PointerShape {
        let mut p = PointerShape::new(None);
        p.device = Some(Box::new(sent.clone()));
        p
    }

    #[test]
    fn a_shape_asked_for_is_sent_once_on_the_last_enter_s_serial() {
        let sent = Sent::default();
        let mut p = shape_with(&sent);
        p.entered(7);
        p.ask(Kit::Text);
        p.ask(Kit::Text);
        p.ask(Kit::Default);
        p.entered(9);
        p.ask(Kit::Default);
        p.ask(Kit::ColResize);
        assert_eq!(
            *sent.0.borrow(),
            [(7, Shape::Text), (7, Shape::Default), (9, Shape::ColResize)],
            "a repeat and the default an enter already shows send nothing"
        );
    }

    #[test]
    fn a_grip_s_arrow_holds_over_what_the_program_asks_for() {
        let sent = Sent::default();
        let mut p = shape_with(&sent);
        p.entered(1);
        p.ask(Kit::Text);
        p.set_grip(Some(Shape::EResize));
        p.ask(Kit::Default);
        p.set_grip(None);
        assert_eq!(
            *sent.0.borrow(),
            [(1, Shape::Text), (1, Shape::EResize), (1, Shape::Default)]
        );
    }

    #[test]
    fn each_kit_shape_goes_out_under_its_cursor_shape_name() {
        for s in [
            Kit::Default,
            Kit::Hand,
            Kit::Text,
            Kit::ColResize,
            Kit::RowResize,
            Kit::EResize,
            Kit::SResize,
            Kit::SeResize,
        ] {
            let name = format!("{:?}", wire(s)).to_lowercase();
            assert_eq!(name, s.name().replace('-', ""), "{s:?}");
        }
    }

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
