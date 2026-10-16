//! The compositor's one `State`: the Wayland display and its protocol
//! globals, the seat, the windows and the backend. Every protocol handler
//! is a trait impl on `State` (`handlers`), and every event source on the
//! loop gets `&mut State`, so nothing holds a pointer into it.

use std::{ffi::OsString, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use smithay::{
    backend::renderer::gles::GlesRenderer,
    desktop::PopupManager,
    input::{Seat, SeatState, keyboard::XkbConfig, pointer::CursorImageStatus},
    reexports::{
        calloop::{Interest, LoopHandle, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration_manager::Mode as KdeDefaultMode,
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
        },
    },
    utils::{Clock, Logical, Monotonic, Point},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        image_capture_source::{ImageCaptureSourceState, OutputCaptureSourceState},
        image_copy_capture::ImageCopyCaptureState,
        output::OutputManagerState,
        presentation::PresentationState,
        selection::{data_device::DataDeviceState, ext_data_control, wlr_data_control},
        shell::kde::decoration::KdeDecorationState,
        shell::xdg::{XdgShellState, decoration::XdgDecorationState, dialog::XdgDialogState},
        shm::ShmState,
        socket::ListeningSocketSource,
        xdg_toplevel_tag::XdgToplevelTagManager,
    },
};

use reprisekit::{
    frame::FrameHit,
    icontheme::{IconCache, IconTheme},
    pointer::DoubleClick,
    text::TextRenderer,
};
use reprisescene::TreeId;

use crate::{
    backend::Backend,
    capture::Captures,
    config::Config,
    cursor::Cursors,
    desktopview::DesktopView,
    glass::GlassCtx,
    input::Interaction,
    menuview::{Menu, MenuPending},
    quicklaunch::QuickItem,
    spawn::SessionEnv,
    taskbarview::Bar,
    wallpaperview::Wallpaper,
    window::{Window, WindowId},
};

pub type DeskScene = reprisescene::Scene<GlesRenderer, SceneOwner>;

/// What a tree in the scene belongs to: the tag a hit test walks up to
/// (`Scene::find_data`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneOwner {
    Window(WindowId),
    /// A taskbar, named by its own tree (`Bar::tree`); its orb tree in
    /// `layers.start_orb` carries the same tag.
    Taskbar(TreeId),
    Menu(usize),
    Desktop,
    Layer,
    Dialog(usize),
    Launcher,
    StartPanel,
    Osd,
    Switcher,
    Overview,
    /// The tray's hidden icons flyout; hits resolve to its cells.
    TrayFlyout,
    /// The task button's preview panel; hits resolve to its cells.
    TaskPreview,
    /// The snap layout picker (`snappickerview`); hits resolve to its
    /// zones.
    SnapPicker,
    Notification,
    NotifyCenter,
    ClipView,
    EmojiView,
    Calendar,
    /// The displays cell's panel (`displaysview`); hits resolve to its parts.
    Displays,
    /// A debug overlay panel (`debugview`); it takes no pointer input.
    Debug,
}

/// The scene's layer trees, bottom to top: the background (wallpaper),
/// layer-shell's bottom layer, the windows, the windows kept on top,
/// layer-shell's top layer, the taskbar, a fullscreen window, the desktop
/// switch animation, the menu tree, Start's orb over its menu, the
/// layer-shell overlay tree, and finally the lock screen, above everything
/// else. The overlay tree is left out of the glass backdrop: what lies on
/// top of every glass (drag images, a client's dim) must not blur into it.
pub struct Layers {
    pub background: TreeId,
    pub bottom: TreeId,
    pub windows: TreeId,
    /// Windows kept above the others: those set Always on Top and the
    /// windows they own. `State::restack` moves trees between this and
    /// `windows`.
    pub on_top: TreeId,
    pub top: TreeId,
    pub taskbar: TreeId,
    /// Holds the focused window's tree while it is fullscreen, so it
    /// covers the taskbar; `State::restack` moves trees in and out of it.
    pub fullscreen: TreeId,
    pub switching: TreeId,
    /// Compositor overlays that must sit above the taskbar: the Start menu
    /// stack and the Alt+Tab switcher.
    pub menu: TreeId,
    pub start_orb: TreeId,
    pub overlay: TreeId,
    /// The lock screen, above everything including layer-shell overlays.
    pub lock: TreeId,
    pub saver: TreeId,
}

impl Layers {
    pub(crate) fn new(scene: &mut DeskScene) -> Layers {
        let root = scene.root();
        let mut t = || scene.tree_create(root);
        let layers = Layers {
            background: t(),
            bottom: t(),
            windows: t(),
            on_top: t(),
            top: t(),
            taskbar: t(),
            fullscreen: t(),
            switching: t(),
            menu: t(),
            start_orb: t(),
            overlay: t(),
            lock: t(),
            saver: t(),
        };
        crate::glass::leave_out(scene, layers.overlay);
        layers
    }
}

pub struct State {
    pub config: Config,
    /// Where `config` was read from, and read again from on a change
    /// (`configwatch`).
    pub config_path: PathBuf,
    pub dh: DisplayHandle,
    pub handle: LoopHandle<'static, State>,
    /// Stops the event loop: SIGTERM, SIGINT.
    /// The session's end (the `exit` bind, Log Off) stops it too.
    pub signal: LoopSignal,
    pub clock: Clock<Monotonic>,
    pub env: SessionEnv,
    /// The GTK theme and Qt palette written for `[toolkits] ... =
    /// "reprise"` this session.
    pub toolkits: crate::toolkits::Generated,

    pub compositor_state: CompositorState,
    pub shm_state: ShmState,
    pub xdg_shell_state: XdgShellState,
    /// KDE server-decoration, defaulting to server-side (`handlers`).
    pub kde_decoration_state: KdeDecorationState,
    pub data_device_state: DataDeviceState,
    /// Clipboard managers and wl-clipboard read and set the selection
    /// through these without a focused surface.
    pub ext_data_control_state: ext_data_control::DataControlState,
    pub wlr_data_control_state: wlr_data_control::DataControlState,
    /// The primary selection and the compositor's own clipboard
    /// (`clipboard`).
    pub clip: crate::clipboard::Clip,
    /// The drag the seat carries, and what outlives it (`dnd`).
    pub dnd: crate::dnd::Dnd,
    /// The desktop's icon drags and the drops' file work (`dragview`).
    pub drags: crate::dragview::Drags,
    pub seat_state: SeatState<State>,
    pub seat: Seat<State>,
    pub output_capture_source_state: OutputCaptureSourceState,
    pub image_copy_capture_state: ImageCopyCaptureState,
    /// Viewporter, fractional scale, explicit sync and the other
    /// per-surface protocols (`surfaceproto`).
    pub surface_protos: crate::surfaceproto::SurfaceProtos,
    pub foreign: crate::xdgforeign::Foreign,
    pub toplevels: crate::toplevels::Toplevels,
    pub activation: smithay::wayland::xdg_activation::XdgActivationState,
    pub outputs_mgmt: crate::outputmgmt::OutputMgmt,
    /// Globals no handler reads back, kept for as long as the display.
    #[allow(dead_code)]
    pub globals: Globals,

    /// Everything on screen, in layout coordinates, and the outputs that
    /// show it.
    pub scene: DeskScene,
    pub layers: Layers,
    pub windows: Vec<Window>,
    pub focused: Option<WindowId>,
    pub focus_order: Vec<WindowId>,
    /// The window whose caption button is lit (`State::frame_hover`).
    pub hot_frame: Option<WindowId>,
    /// Windows whose caption buttons are still fading (`hoverfade`).
    pub fading_frames: Vec<WindowId>,
    /// The Glass shadow's pieces, shared by every window's shadow nodes.
    pub shadows: crate::window::ShadowCache,
    /// The fade timer, while something fades.
    pub hover_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    pub text: TextRenderer,
    pub caption_text: TextRenderer,
    /// The move, size or caption button press the pointer is in.
    pub interaction: Interaction,
    /// Left presses on a frame's caption or icon, paired into double clicks.
    pub caption_clicks: DoubleClick<(WindowId, FrameHit)>,
    /// The pointer at the last button press.
    pub press_at: (f64, f64),
    pub buttons_down: u32,
    pub popups: PopupManager,
    /// What the pointer shows: a client's surface, a named cursor or nothing.
    pub cursor: CursorImageStatus,
    pub cursors: Cursors,
    /// The libinput pointers, pointer constraints, relative motion,
    /// gestures and cursor shapes (`pointers`, `constraint`).
    pub pointers: crate::pointers::Pointers,
    /// Capture sessions and the frames waiting for their output's next
    /// frame or their window's next commit.
    pub captures: Captures,
    pub desktops: crate::desktops::Desktops,
    pub workspaces: crate::workspaces::Workspaces,
    /// What the record dot and the microphone and camera indicators show
    /// (`privacy`).
    pub privacy: crate::privacy::Privacy,
    /// The red border around what is captured (`captureborder`).
    pub capture_border: crate::captureborder::CaptureBorder,
    /// The geoclue agent: its prompts and the location pin (`location`).
    pub location: crate::location::Location,
    /// The DRM session, GPU and connectors; None only while starting.
    pub backend: Option<Backend>,
    /// `[wallpaper] image` and its per-output buffers; None for the
    /// background colour alone.
    pub wallpaper: Option<Wallpaper>,
    /// `[wallpaper] slideshow` while it runs.
    pub slideshow: Option<crate::wallpaperview::Slideshow>,
    pub bars: Vec<Bar>,
    /// What the bars share: the lock, the bands, the Quick Launch drag.
    pub bar_shared: crate::taskbarview::BarShared,
    /// The clock's timer, firing at each minute boundary while a bar is
    /// shown.
    pub clock_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    /// Bars whose Start glow is still fading (`hoverfade`).
    pub fading_bars: Vec<TreeId>,
    pub icons: IconCache,
    pub app_index: Vec<reprisefs::apps::AppEntry>,
    pub launches: crate::launches::Launches,
    /// Keeps `app_index` current as programs come and go (`appindex`).
    pub apps_watch: crate::appindex::AppIndexWatch,
    pub start_text: TextRenderer,
    pub start_panel_text: TextRenderer,
    pub quicklaunch: Vec<QuickItem>,
    pub menus: Vec<Menu>,
    /// What the hover timer will do when it fires, and the timer, while
    /// something is pending.
    pub menu_pending: Option<MenuPending>,
    pub menu_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    /// The menu fade-in's timer, while a menu fades in.
    pub menu_fade_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    /// The open stack was driven from the keyboard: access keys show.
    pub menu_keys: bool,
    /// Where the desktop's menu was opened, for what New and Paste make.
    pub desktop_menu_at: Option<(i32, i32)>,
    /// The desktop icons on the first output; None without a home
    /// directory or before `desktopview::setup`.
    pub desktop: Option<DesktopView>,
    /// What every piece of glass draws with this frame (`glass`).
    pub glass: Rc<GlassCtx>,
    /// Desktop water's fields and pictures (`waterview`).
    pub water: Rc<crate::waterview::WaterCtx>,
    /// Refreshes the glass a held frame left stale once `[glass]
    /// blur_rate`'s cap is up (`reblur`), while armed.
    pub reblur_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    /// The snap outline and grid, and each app's remembered frame
    /// (`placement`).
    pub wm: crate::placement::Wm,
    /// The tab groups and the pointer on their captions (`tabview`).
    pub tabs: crate::tabview::TabView,
    /// The compositor's dialogs (`dialogview`), their edit fields' caret,
    /// the file work they start off the loop, and the windows' pings
    /// (`hang`).
    pub dialogs: crate::dialogview::Dialogs,
    pub caret: crate::caret::Caret,
    pub fsjobs: crate::fsjobs::FsJobs,
    /// The recent files and document folders the menus list (`recentdocs`).
    pub recent: crate::recentdocs::RecentDocs,
    pub pickthumbs: crate::pickthumbs::PickThumbs,
    pub hang: crate::hang::Hang,
    /// The key route's own state: the Super tap, chords, keys kept from
    /// the client (`keyboard`).
    pub key_input: crate::keyboard::KeyInput,
    pub devices: crate::devices::Devices,
    pub ime: crate::textinput::Ime,
    pub switcher: crate::switcherview::SwitcherView,
    pub overview: crate::overviewview::Overviews,
    /// `reprise_input_v1`: the EIS clients, the input captures and the
    /// global shortcut objects (`inputproto`).
    pub inputproto: crate::inputproto::InputProto,
    pub window_anims: crate::windowanim::WindowAnims,
    pub magnifier: crate::magnifier::Magnifier,
    pub switch_anim: crate::switchanim::SwitchAnim,
    /// The StatusNotifierItem tray, or None when it could not be started
    /// (`tray::setup`).
    pub tray: Option<crate::tray::Tray>,
    pub xwayland: crate::xwayland::Xwayland,
    pub idle: crate::idle::IdleState,
    pub saver: crate::saverview::SaverState,
    pub power: crate::power::PowerState,
    pub night: crate::nightlight::NightLight,
    /// wlr-layer-shell: panels, docks, launchers, backgrounds (`layer`).
    pub layers_shell: crate::layer::LayerShell,
    /// The lock screen and ext-session-lock (`lockview`, `sessionlock`).
    pub lock: crate::lockview::LockScreen,
    pub launcher: Option<crate::launcherview::LauncherView>,
    pub start_panel: Option<crate::startpanelview::StartPanelView>,
    /// The Start panel's frame, kept between opens so an open skips drawing it again.
    pub start_backing: reprisekit::glass::startpanel::Backing,
    pub panels: crate::panels::Panels,
    pub osd: crate::osdview::OsdView,
    pub displays: crate::displays::Displays,
    pub debug: crate::debugview::DebugView,
    pub identify: crate::identifyview::IdentifyView,
    pub live_previews: crate::livepreview::LivePreviews,
    pub task_preview: crate::taskpreview::TaskPreviewView,
    pub snap_picker: crate::snappickerview::SnapPickerView,
    pub snap_bar: crate::snapbarview::SnapBarView,
    /// Each bar's auto-hide (`autohideview`).
    pub bar_hide: crate::autohideview::BarHide,
    pub peek: crate::peek::Peek,
    pub titles: crate::retitle::Flush,
    /// Dead keys and the Compose key for the lock's password field; None
    /// when the locale has no Compose file.
    pub compose: Option<crate::compose::Compose>,
    pub notify: crate::notifyview::NotifyView,
    pub tooltip: crate::tooltip::TooltipView,
    pub access: crate::access::Access,
    /// The apps the last session ended with, restored at login
    /// (`session`).
    pub session: crate::session::Session,
    /// Log Off, Turn Off or Restart waiting for the programs to close
    /// (`sessionend`).
    pub ending: Option<crate::sessionend::Ending>,
    pub calendar: crate::calendarview::CalendarState,
    pub displays_view: crate::displaysview::DisplaysViewState,
}

impl State {
    /// The display, its socket and every global, on `handle`'s loop. The
    /// backend comes after (`backend::init`).
    pub fn new(
        config: Config,
        config_path: PathBuf,
        handle: LoopHandle<'static, State>,
        signal: LoopSignal,
    ) -> Result<State, Box<dyn std::error::Error>> {
        let display: Display<State> = Display::new()?;
        let dh = display.handle();

        let listening = listen()?;
        let socket = listening.socket_name().to_os_string();
        handle.insert_source(listening, |stream, _, state| {
            let data = ClientState {
                sandbox: crate::sandbox::ClientSandbox::of_stream(&stream, None),
                ..ClientState::default()
            };
            if let Err(e) = state.dh.insert_client(stream, Arc::new(data)) {
                log::warn!("client refused: {e}");
            }
        })?;
        handle.insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, state| {
                // SAFETY: the display is owned by this source and never
                // dropped while the loop dispatches it.
                unsafe { display.get_mut().dispatch_clients(state)? };
                state.ime_sync_grab();
                Ok(PostAction::Continue)
            },
        )?;

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        let kb = &config.keyboard;
        if seat
            .add_keyboard(kb.xkb_config(), kb.repeat_delay, kb.repeat_rate)
            .is_err()
        {
            log::warn!(
                "keyboard: no keymap for layout {:?} variant {:?}; using the default",
                kb.layout,
                kb.variant
            );
            seat.add_keyboard(XkbConfig::default(), kb.repeat_delay, kb.repeat_rate)?;
        }
        seat.add_pointer();

        let env = SessionEnv {
            renderer: "gles2",
            qt_platform_theme: config.toolkits.qt_platform_theme.clone(),
            ..SessionEnv::new(
                socket.to_string_lossy().into_owned(),
                &config.theme.colors,
                config.interface,
                config.blue_variant,
                config.glass.mode,
                &config.icons.theme,
                config.cursor.effective(),
            )
        };
        let cursors = Cursors::new(config.cursor.effective());
        let mut scene = DeskScene::new();
        let layers = Layers::new(&mut scene);
        let text = TextRenderer::load(&config.theme.fonts);
        let caption_text = caption_text(&config.theme);
        let start_text = start_text(&config.theme);
        let start_panel_text = reprisekit::interfaces::start_panel_text(&config.theme);
        let icons = IconCache::new(IconTheme::load(
            &config.icons.theme,
            &IconTheme::default_roots(),
        ));
        let app_index = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
        log::info!(
            "icon theme {} ({} entries indexed)",
            config.icons.theme,
            app_index.len()
        );
        let quicklaunch = crate::taskbarview::load_quicklaunch(&config);

        let ime = crate::textinput::Ime::new(&dh, &seat);
        let clip = crate::clipboard::Clip::new(&dh);

        Ok(State {
            compositor_state: CompositorState::new::<State>(&dh),
            shm_state: ShmState::new::<State>(&dh, vec![]),
            xdg_shell_state: XdgShellState::new::<State>(&dh),
            kde_decoration_state: KdeDecorationState::new::<State>(&dh, KdeDefaultMode::Server),
            data_device_state: DataDeviceState::new::<State>(&dh),
            ext_data_control_state: ext_data_control::DataControlState::new::<State, _>(
                &dh,
                Some(&clip.primary),
                crate::sandbox::filter("ext_data_control_manager_v1"),
            ),
            wlr_data_control_state: wlr_data_control::DataControlState::new::<State, _>(
                &dh,
                Some(&clip.primary),
                crate::sandbox::filter("zwlr_data_control_manager_v1"),
            ),
            output_capture_source_state: OutputCaptureSourceState::new_with_filter::<State, _>(
                &dh,
                crate::sandbox::filter("ext_output_image_capture_source_manager_v1"),
            ),
            image_copy_capture_state: ImageCopyCaptureState::new_with_filter::<State, _>(
                &dh,
                crate::sandbox::filter("ext_image_copy_capture_manager_v1"),
            ),
            outputs_mgmt: crate::outputmgmt::OutputMgmt::new(&dh),
            clip,
            dnd: Default::default(),
            drags: Default::default(),
            globals: Globals {
                xdg_decoration: XdgDecorationState::new::<State>(&dh),
                output_manager: OutputManagerState::new_with_xdg_output::<State>(&dh),
                image_capture_source: ImageCaptureSourceState::new(),
                screencopy: crate::screencopy::ScreencopyManagerState::new::<State>(&dh),
                presentation: PresentationState::new::<State>(&dh, libc::CLOCK_MONOTONIC as u32),
                xdg_dialog: XdgDialogState::new::<State>(&dh),
                xdg_toplevel_tag: XdgToplevelTagManager::new::<State>(&dh),
                xdg_toplevel_icon: crate::icon::manager(&dh),
                virtual_pointer: crate::virtualpointer::VirtualPointerManagerState::new(&dh),
                security_context: crate::sandbox::security_context_state(&dh),
            },
            surface_protos: crate::surfaceproto::SurfaceProtos::new(&dh),
            foreign: crate::xdgforeign::Foreign::new(&dh),
            toplevels: crate::toplevels::Toplevels::new(&dh),
            activation: smithay::wayland::xdg_activation::XdgActivationState::new::<State>(&dh),
            seat_state,
            seat,
            scene,
            layers,
            windows: Vec::new(),
            focused: None,
            focus_order: Vec::new(),
            hot_frame: None,
            fading_frames: Vec::new(),
            shadows: Default::default(),
            hover_timer: None,
            text,
            caption_text,
            interaction: Interaction::Idle,
            caption_clicks: DoubleClick::default(),
            press_at: (0.0, 0.0),
            buttons_down: 0,
            popups: PopupManager::default(),
            cursor: CursorImageStatus::default_named(),
            cursors,
            pointers: crate::pointers::Pointers::new(&dh),
            captures: Captures::default(),
            desktops: crate::desktops::Desktops::new(config.workspaces),
            workspaces: crate::workspaces::Workspaces::new(
                &dh,
                crate::desktops::Desktops::new(config.workspaces),
            ),
            privacy: crate::privacy::Privacy::start(&handle),
            capture_border: Default::default(),
            location: crate::location::Location::start(&handle, config.location),
            backend: None,
            // A slideshow picks its first picture once the loop runs.
            wallpaper: config
                .wallpaper
                .image
                .as_deref()
                .filter(|_| config.wallpaper.slideshow.is_none())
                .map(Wallpaper::new),
            slideshow: None,
            bars: Vec::new(),
            bar_shared: crate::taskbarview::BarShared::load(),
            clock_timer: None,
            fading_bars: Vec::new(),
            icons,
            app_index,
            launches: {
                let p = crate::state::load();
                crate::launches::Launches {
                    pins: p.start_pins,
                    recent: p.launches,
                }
            },
            apps_watch: Default::default(),
            start_text,
            start_panel_text,
            quicklaunch,
            menus: Vec::new(),
            menu_pending: None,
            menu_timer: None,
            menu_fade_timer: None,
            menu_keys: false,
            desktop_menu_at: None,
            desktop: None,
            glass: GlassCtx::new(),
            water: crate::waterview::WaterCtx::new(),
            reblur_timer: None,
            wm: crate::placement::Wm::load(),
            tabs: Default::default(),
            dialogs: Default::default(),
            caret: Default::default(),
            fsjobs: Default::default(),
            recent: Default::default(),
            pickthumbs: Default::default(),
            hang: Default::default(),
            key_input: crate::keyboard::KeyInput::new(&dh),
            devices: crate::devices::Devices::new(&dh),
            ime,
            switcher: Default::default(),
            overview: Default::default(),
            inputproto: crate::inputproto::InputProto::new(&dh),
            window_anims: Default::default(),
            magnifier: Default::default(),
            switch_anim: Default::default(),
            tray: None,
            xwayland: crate::xwayland::Xwayland::new(&dh),
            idle: crate::idle::IdleState::new(&dh, handle.clone()),
            saver: Default::default(),
            power: crate::power::PowerState::default(),
            night: crate::nightlight::NightLight::new(&dh),
            layers_shell: crate::layer::LayerShell::new(&dh),
            lock: crate::lockview::LockScreen::new(&dh),
            launcher: None,
            start_panel: None,
            start_backing: Default::default(),
            panels: Default::default(),
            osd: Default::default(),
            displays: Default::default(),
            debug: Default::default(),
            identify: Default::default(),
            live_previews: Default::default(),
            task_preview: Default::default(),
            snap_picker: Default::default(),
            snap_bar: Default::default(),
            bar_hide: Default::default(),
            peek: Default::default(),
            titles: Default::default(),
            compose: crate::compose::Compose::from_locale(),
            notify: Default::default(),
            tooltip: Default::default(),
            access: crate::access::Access::new(&handle),
            session: crate::session::Session::load(),
            ending: None,
            calendar: Default::default(),
            displays_view: Default::default(),
            config,
            config_path,
            dh,
            handle,
            signal,
            clock: Clock::new(),
            env,
            toolkits: Default::default(),
        })
    }

    pub fn socket(&self) -> OsString {
        OsString::from(&self.env.wayland_display)
    }

    pub fn now(&self) -> Duration {
        self.clock.now().into()
    }

    /// Where the pointer is, or where a click the compositor makes for
    /// the keyboard or a screen reader lands while it runs
    /// (`access::click`).
    pub fn pointer_location(&self) -> Point<f64, Logical> {
        if let Some(p) = self.access.click_at {
            return p;
        }
        self.seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default()
    }

    /// Ask every output for a new frame for a change on the scene.
    pub fn redraw_all(&mut self) {
        crate::backend::schedule_all(self, crate::backend::Ask::Scene);
    }

    /// Ask every output for a new frame for what clients drew: nothing on
    /// the scene moved, so glass may keep its blur for `[glass]
    /// blur_rate` (`reblur`).
    pub fn redraw_content(&mut self) {
        crate::backend::schedule_all(self, crate::backend::Ask::Content);
    }

    /// Ask every output for a new frame for where the pointer went, when
    /// nothing else changed: the frame may only move the cursor plane
    /// (`backend::pointer_frame`).
    pub fn redraw_pointer(&mut self) {
        crate::backend::schedule_all(self, crate::backend::Ask::Pointer);
    }
}

/// The first free socket from `wayland-0` on, as clients and tools that
/// assume the first display expect.
fn listen() -> Result<ListeningSocketSource, Box<dyn std::error::Error>> {
    let mut last = None;
    for n in 0..33 {
        match ListeningSocketSource::with_name(&format!("wayland-{n}")) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(last.map_or_else(|| "no Wayland socket".into(), |e| e.into()))
}

/// Protocol states whose handlers never ask for them back.
#[allow(dead_code)]
pub struct Globals {
    pub xdg_decoration: XdgDecorationState,
    pub output_manager: OutputManagerState,
    pub image_capture_source: ImageCaptureSourceState,
    pub screencopy: crate::screencopy::ScreencopyManagerState,
    pub presentation: PresentationState,
    pub xdg_dialog: XdgDialogState,
    pub xdg_toplevel_tag: XdgToplevelTagManager,
    pub xdg_toplevel_icon: smithay::wayland::xdg_toplevel_icon::XdgToplevelIconManager,
    pub virtual_pointer: crate::virtualpointer::VirtualPointerManagerState,
    pub security_context: smithay::wayland::security_context::SecurityContextState,
}

/// The caption renderer for `theme`'s look: the style's own font, or the
/// body font for Classic, which sets both in it.
pub fn caption_text(theme: &crate::theme::Theme) -> TextRenderer {
    match theme.look.style() {
        Some(s) => TextRenderer::load(&s.caption_text.font()),
        None => {
            // Classic sets no dialog, so it loads no second dialog font.
            let fonts = crate::theme::FontConfig {
                shell_dlg: None,
                ..theme.fonts.clone()
            };
            TextRenderer::load(&fonts)
        }
    }
}

/// The font Start's label draws with: the style's own, or the body font
/// for Classic.
pub fn start_text(theme: &crate::theme::Theme) -> TextRenderer {
    match theme.look.style() {
        Some(s) => TextRenderer::load(&s.start_text.font()),
        None => {
            let fonts = crate::theme::FontConfig {
                shell_dlg: None,
                ..theme.fonts.clone()
            };
            TextRenderer::load(&fonts)
        }
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    /// Whether it runs in a sandbox, for the global filters (`sandbox`).
    pub sandbox: crate::sandbox::ClientSandbox,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
