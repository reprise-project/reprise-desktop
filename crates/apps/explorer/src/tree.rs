use crate::foldertree::{self, Inputs, Kids};
use crate::place::Place;
use crate::{Explorer, LIST_WAIT};
use repriseapp::Ctx;
use repriseapp::listjob::Job;
use reprisekit::controls::treeview::{self, TreeView};
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

    /// Reads a Folders node's children on a thread. One that takes longer than [`LIST_WAIT`] (a
    /// dead mount) fills on a later wake ([`Explorer::take_fills`]), the node open and empty until then.
    pub fn fill_node(&mut self, path: Vec<usize>, ctx: &mut Ctx) {
        let Some(place) = self.place_of(&path) else {
            return;
        };
        if self.filling.iter().any(|(p, _)| *p == place) {
            return;
        }
        match self.read_job(ctx, self.kids_reader(&path, place.clone())) {
            Ok(kids) => self.fill_read(&place, kids, ctx),
            Err(job) => self.filling.push((place, job)),
        }
    }

    /// Runs `read` on a listing thread and waits [`LIST_WAIT`] for it; a later answer (a dead
    /// mount) comes back as the job, for a wake to take.
    pub(crate) fn read_job<T: Send + 'static>(
        &self,
        ctx: &Ctx,
        read: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, Job<T>> {
        #[cfg(test)]
        if let Some(gate) = self.held.clone() {
            let job = Job::spawn(Some(&ctx.wake), move || {
                drop(gate.read());
                read()
            });
            return job.take().ok_or(job);
        }
        let job = Job::spawn(Some(&ctx.wake), read);
        match job.wait(LIST_WAIT) {
            Some(answer) => Ok(answer),
            None => Err(job),
        }
    }

    /// The read of the children of the node at `path`, standing for `place`, for a thread.
    pub(crate) fn kids_reader(
        &self,
        path: &[usize],
        place: Place,
    ) -> impl FnOnce() -> Kids + Send + 'static {
        self.kids_reader_in(&self.tree, &self.tree_places, path, place)
    }

    /// [`Explorer::kids_reader`] for a node of another tree over this window's places (the
    /// Move/Copy picker's).
    pub(crate) fn kids_reader_in(
        &self,
        tree: &TreeView,
        places: &[Place],
        path: &[usize],
        place: Place,
    ) -> impl FnOnce() -> Kids + Send + 'static {
        let known: Vec<Place> = tree
            .node(path)
            .map(|n| {
                n.children
                    .iter()
                    .map(|c| places[c.id as usize].clone())
                    .collect()
            })
            .unwrap_or_default();
        let world = self.tree_world();
        move || {
            let known: Vec<&Place> = known.iter().collect();
            foldertree::read_kids(&place, &world.inputs(), &known)
        }
    }

    /// The children of each place of `chain` ([`foldertree::to_fill`]) for a thread, for
    /// [`foldertree::follow_with`].
    pub(crate) fn chain_reader(
        &self,
        chain: Vec<Place>,
    ) -> impl FnOnce() -> foldertree::Chain + Send + 'static {
        let world = self.tree_world();
        move || {
            let inp = world.inputs();
            chain
                .into_iter()
                .map(|p| {
                    let kids = foldertree::read_kids(&p, &inp, &[]);
                    (p, kids)
                })
                .collect()
        }
    }

    fn tree_world(&self) -> foldertree::World {
        foldertree::World {
            show_hidden: self.settings.show_hidden,
            drives: self.drives.clone(),
            volumes: self.volumes.clone(),
            drive_mounts: self.drive_mounts.clone(),
        }
    }

    /// Fills the node standing for `place` from `kids`, if the tree still holds it.
    fn fill_read(&mut self, place: &Place, kids: Kids, ctx: &mut Ctx) {
        let Some(path) = self.tree_path_of(place) else {
            return;
        };
        let inp = Inputs {
            show_hidden: self.settings.show_hidden,
            drives: &self.drives,
            volumes: &self.volumes,
            drive_mounts: &self.drive_mounts,
        };
        foldertree::fill_with(
            &mut self.tree,
            &mut self.tree_places,
            path,
            kids,
            &inp,
            &mut ctx.icons,
        );
    }

    /// Fills the nodes whose late reads ([`Explorer::fill_node`]) answered: true when one did.
    pub(crate) fn take_fills(&mut self, ctx: &mut Ctx) -> bool {
        let mut done = Vec::new();
        self.filling.retain(|(place, job)| match job.take() {
            Some(kids) => {
                done.push((place.clone(), kids));
                false
            }
            None => true,
        });
        let filled = !done.is_empty();
        for (place, kids) in done {
            self.fill_read(&place, kids, ctx);
        }
        filled
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

    /// Reads the children of the node standing for `place` again when it was opened, off the loop
    /// as [`Explorer::fill_node`] reads: [`Explorer::tree_follow`] only fills a node whose children
    /// are still empty. A read of the node still in flight stands for this one.
    pub fn refill_tree_node(&mut self, place: &Place, ctx: &mut Ctx) {
        if let Some(path) =
            foldertree::refill_path(&self.tree, &self.tree_places, place, &self.drive_mounts)
        {
            self.fill_node(path, ctx);
        }
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
