use crate::Explorer;
use crate::foldertree::{self, Inputs, Kids};
use crate::place::Place;
use repriseapp::Ctx;
use reprisekit::controls::treeview;
use std::path::PathBuf;

impl Explorer {
    pub fn build_tree(&mut self, ctx: &mut Ctx) {
        let inp = Inputs {
            show_hidden: self.settings.show_hidden,
            drives: &self.drives,
            volumes: &self.volumes,
            drive_mounts: &self.drive_mounts,
        };
        foldertree::build(&mut self.tree, &mut self.tree_places, &inp, &mut ctx.icons);
    }

    pub fn fill_node(&mut self, path: Vec<usize>, ctx: &mut Ctx) {
        let inp = Inputs {
            show_hidden: self.settings.show_hidden,
            drives: &self.drives,
            volumes: &self.volumes,
            drive_mounts: &self.drive_mounts,
        };
        foldertree::fill(
            &mut self.tree,
            &mut self.tree_places,
            path,
            &inp,
            &mut ctx.icons,
        );
    }

    pub fn place_of(&self, path: &[usize]) -> Option<Place> {
        foldertree::place_of(&self.tree, &self.tree_places, path)
    }

    pub fn node_folder(&self) -> Option<(PathBuf, String)> {
        let path = self.tree.selected.as_ref()?;
        let node = self.tree.node(path)?;
        match foldertree::place_of(&self.tree, &self.tree_places, path)? {
            Place::Folder(dir)
                if !self.drive_mounts.contains(&dir) && !reprisefs::ops::is_mount_point(&dir) =>
            {
                Some((dir, node.label.clone()))
            }
            _ => None,
        }
    }

    pub fn tree_path_of(&self, place: &Place) -> Option<Vec<usize>> {
        foldertree::path_of(&self.tree, &self.tree_places, place, &self.drive_mounts)
    }

    /// [`Explorer::tree_follow`] only fills a node whose children are still empty.
    pub fn refill_tree_node(&mut self, place: &Place, ctx: &mut Ctx) {
        let inp = Inputs {
            show_hidden: self.settings.show_hidden,
            drives: &self.drives,
            volumes: &self.volumes,
            drive_mounts: &self.drive_mounts,
        };
        foldertree::refill(
            &mut self.tree,
            &mut self.tree_places,
            place,
            &inp,
            &mut ctx.icons,
        );
    }

    pub fn tree_follow(&mut self, place: &Place, ctx: &mut Ctx) {
        self.tree_follow_with(place, Vec::new(), ctx);
    }

    /// [`Explorer::tree_follow`] with the children of some places on the way already read
    /// ([`foldertree::read_kids`]).
    pub fn tree_follow_with(&mut self, place: &Place, read: Vec<(Place, Kids)>, ctx: &mut Ctx) {
        let inp = Inputs {
            show_hidden: self.settings.show_hidden,
            drives: &self.drives,
            volumes: &self.volumes,
            drive_mounts: &self.drive_mounts,
        };
        foldertree::follow_with(
            &mut self.tree,
            &mut self.tree_places,
            place,
            &inp,
            &mut ctx.icons,
            read,
        );
        if let Some(frame) = self.layout(ctx).tree {
            let tl = treeview::layout(
                &self.tree,
                &ctx.text,
                ctx.theme.fonts.size_px,
                frame,
                &ctx.theme.controls(),
            );
            self.tree.follow(&tl);
        }
    }
}
