use crate::foldertree::{self, Inputs, Kids};
use crate::place::Place;
use crate::{Explorer, LIST_WAIT};
use repriseapp::Ctx;
use repriseapp::listjob::Job;
use reprisekit::controls::treeview::{self, TreeView};
use std::path::PathBuf;

impl Explorer {
    pub fn build_tree(&mut self, ctx: &mut Ctx) {
        foldertree::build(&mut self.tree, &mut self.tree_places, &mut ctx.icons);
        self.fill_node(vec![0], ctx);
    }

    /// Reads a Folders node's children on a thread. One that takes longer than [`LIST_WAIT`] (a
    /// dead mount) fills on a later wake ([`Explorer::take_fills`]), the node open and empty until then.
    pub fn fill_node(&mut self, path: Vec<usize>, ctx: &mut Ctx) {
        let Some(place) = self.place_of(&path) else {
            return;
        };
        if self.filling.iter().any(|(p, ..)| *p == place) {
            return;
        }
        match self.read_job(ctx, self.kids_reader(&path, place.clone())) {
            Ok(kids) => self.fill_read(&place, kids, ctx),
            Err(job) => self.filling.push((place, job, false)),
        }
    }

    /// How long the loop waits on a listing: [`LIST_WAIT`], or not at all while a test holds reads.
    pub(crate) fn list_wait(&self) -> std::time::Duration {
        #[cfg(test)]
        if self.held.is_some() {
            return std::time::Duration::ZERO;
        }
        LIST_WAIT
    }

    /// Runs `read` on a listing thread and waits [`LIST_WAIT`] for it; a later answer (a dead
    /// mount) comes back as the job, for a wake to take.
    pub(crate) fn read_job<T: Send + 'static>(
        &self,
        ctx: &Ctx,
        read: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, Job<T>> {
        let job = self.spawn_read(ctx, read);
        match job.wait(self.list_wait()) {
            Some(answer) => Ok(answer),
            None => Err(job),
        }
    }

    /// Runs `read` on a listing thread that wakes the window once it answers. While a test holds
    /// reads, it waits for the test's write lock to drop first.
    pub(crate) fn spawn_read<T: Send + 'static>(
        &self,
        ctx: &Ctx,
        read: impl FnOnce() -> T + Send + 'static,
    ) -> Job<T> {
        #[cfg(test)]
        if let Some(gate) = self.held.clone() {
            return Job::spawn(Some(&ctx.wake), move || {
                drop(gate.read());
                read()
            });
        }
        Job::spawn(Some(&ctx.wake), read)
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
            foldertree::read_kids(
                &place,
                &world.inputs(),
                &known,
                world.scale,
                &world.specials,
            )
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
                    let kids = foldertree::read_kids(&p, &inp, &[], world.scale, &world.specials);
                    (p, kids)
                })
                .collect()
        }
    }

    fn tree_world(&self) -> foldertree::World {
        foldertree::World {
            scale: self.drawn_scale.max(1),
            show_hidden: self.settings.show_hidden,
            drives: self.drives.clone(),
            volumes: self.volumes.clone(),
            drive_mounts: self.drive_mounts.clone(),
            specials: self.specials.clone(),
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
        let art = foldertree::fill_with(
            &mut self.tree,
            &mut self.tree_places,
            path,
            kids,
            &inp,
            &mut ctx.icons,
        );
        self.note_art(place, art);
    }

    /// Fills the nodes whose late reads ([`Explorer::fill_node`]) answered: true when one did.
    pub(crate) fn take_fills(&mut self, ctx: &mut Ctx) -> bool {
        let mut done = Vec::new();
        self.filling.retain(|(place, job, again)| match job.take() {
            Some(kids) => {
                done.push((place.clone(), kids, *again));
                false
            }
            None => true,
        });
        let filled = !done.is_empty();
        for (place, kids, again) in done {
            self.fill_read(&place, kids, ctx);
            if again {
                self.refill_tree_node(&place, ctx);
            }
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
    /// are still empty. A read of the node still in flight may predate the change, so the node is
    /// read again once it lands.
    pub fn refill_tree_node(&mut self, place: &Place, ctx: &mut Ctx) {
        let Some(path) =
            foldertree::refill_path(&self.tree, &self.tree_places, place, &self.drive_mounts)
        else {
            return;
        };
        let at = self.place_of(&path);
        if let Some((.., again)) = self
            .filling
            .iter_mut()
            .find(|(p, ..)| at.as_ref() == Some(p))
        {
            *again = true;
            return;
        }
        self.fill_node(path, ctx);
    }

    /// Reads the children of the nodes whose children wear their own icons again
    /// ([`Explorer::refill_tree_node`]), so those come at the scale drawn now.
    pub(crate) fn refill_tree_art(&mut self, ctx: &mut Ctx) {
        for place in std::mem::take(&mut self.tree_art) {
            self.refill_tree_node(&place, ctx);
        }
    }

    /// Notes whether a fill of `place`'s node left a child wearing its own icon.
    fn note_art(&mut self, place: &Place, art: bool) {
        self.tree_art.retain(|p| p != place);
        if art {
            self.tree_art.push(place.clone());
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
        let filled = foldertree::follow_with(
            &mut self.tree,
            &mut self.tree_places,
            place,
            &inp,
            &mut ctx.icons,
            read,
        );
        for (p, art) in filled {
            self.note_art(&p, art);
        }
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
