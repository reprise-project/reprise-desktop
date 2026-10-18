use reprisekit::{icontheme, raster, text, textedit, theme};

mod access;
mod activation;
mod appindex;
mod autohideview;
mod autostart;
mod backend;
mod barkeys;
mod calc;
mod calendarview;
mod camdev;
mod capture;
mod captureborder;
mod caret;
mod clientglass;
mod clientlog;
mod clipboard;
mod cliphistory;
mod clipview;
mod coalesce;
mod compositor;
mod configwatch;
mod constraint;
mod cursor;
mod desktops;
mod devices;
mod displays;
mod displaysview;
mod dmaupload;
mod dnd;
mod dnddrop;
mod dndsource;
mod eis;
mod emojiview;
mod flipview;
mod focus;
mod framepace;
mod gammacontrol;
mod glass;
mod globalshortcuts;
mod handlers;
mod hoverfade;
mod icon;
mod iconedit;
mod idle;
mod imefield;
mod input;
mod inputcapture;
mod inputqueue;
mod journal;
mod kblayout;
mod keyboard;
mod keyfilter;
mod launcherentry;
mod livepreview;
mod location;
mod logbridge;
mod media;
mod menuview;
mod mousekeys;
mod nightlight;
mod notifycenter;
mod notifylog;
mod notifyview;
mod offscreen;
mod outputmgmt;
mod outputpower;
mod overviewview;
mod panels;
mod peek;
mod pointers;
mod popupgrab;
mod privacy;
mod putview;
mod quickwatch;
mod reblur;
mod recentdocs;
mod recentwatch;
mod regionview;
mod remotekeys;
mod render;
mod samepixels;
mod sandbox;
mod screencopy;
mod screenshot;
mod session;
mod sessionend;
mod sessionfade;
mod sessionlock;
mod shake;
mod shakeview;
mod shmtex;
mod shortcutinhibit;
mod snapshot;
mod surfaceproto;
mod switcherview;
mod tablet;
mod tabletpad;
mod tabs;
mod tabview;
mod taskpreview;
mod textinput;
mod tooltip;
mod toplevels;
mod touch;
mod virtualpointer;
mod workspaces;
mod xdgforeign;
mod xdnd;

mod arrange;
mod autohide;
mod bardrag;
mod barplace;
mod binprops;
mod compose;
mod config;
mod configwrite;
mod debugview;
mod desktop;
mod desktopfiles;
mod desktopview;
mod dialog;
mod dialogdrop;
mod dialogview;
mod dirwatch;
mod dragview;
mod editkeys;
mod filemenu;
mod filepick;
mod fsjobs;
mod gestures;
mod hang;
mod hotcorner;
mod identifyview;
mod inputproto;
mod keyroute;
mod keys;
mod launcherview;
mod launches;
mod layer;
mod lock;
mod lockfinger;
mod lockview;
mod magnifier;
mod malloc;
mod mapfocus;
mod migrate;
mod osdview;
mod output;
mod pickthumbs;
mod placement;
mod power;
mod quicklaunch;
mod retitle;
mod saverview;
mod search;
mod snap;
mod snapbarview;
mod snappickerview;
mod snapview;
mod spawn;
mod startmenu;
mod startpanelview;
mod state;
mod switchanim;
mod taskbar;
mod taskbarview;
mod taskslide;
mod toolkits;
mod tray;
mod trayflyout;
mod trayreveal;
mod wallpaper;
mod wallpaperview;
mod waterview;
mod window;
mod windowanim;
mod windowrules;
mod windowstill;
mod winmenu;
mod xwayland;

use clap::Parser;
use compositor::State;
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic};
use std::path::PathBuf;

/// Reprise: a Wayland desktop with a classic retro look
#[derive(Parser)]
#[command(name = "reprise")]
struct Cli {
    /// Config file (default: $XDG_CONFIG_HOME/reprise/config.toml)
    #[arg(long)]
    config: Option<PathBuf>,
    /// Command to run once the compositor is up
    #[arg(long)]
    startup: Option<String>,
    /// Run the lock screen's fingerprint conversation for USER and exit
    /// (the lock starts it; see `lockfinger`)
    #[arg(long, hide = true, value_name = "USER")]
    fingerprint_conversation: Option<String>,
}

fn main() {
    malloc::pin_mmap_threshold();
    spawn::block_signals();
    // zbus logs every served call at info and Smithay its GL devices; RUST_LOG
    // turns them back on.
    reprisebus::journal::builder("info,zbus=warn,smithay=warn", journal::init()).init();
    logbridge::install();
    let cli = Cli::parse();
    if let Some(user) = cli.fingerprint_conversation.as_deref() {
        std::process::exit(lockfinger::child_main(user));
    }
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(config::Config::default_path);
    migrate::run(&config_path, &state::path());
    let (config, config_notice) = config::Config::load(cli.config.as_deref());
    config.warn_retired();
    if let Some(notice) = &config_notice {
        log::warn!("{notice}");
    }
    if let Err(e) = run(config, config_path, cli.startup.as_deref(), config_notice) {
        log::error!("{e}");
        std::process::exit(1);
    }
}

/// Start the display and the backend, then run the event loop until it stops.
/// A `config_notice` goes up as a balloon once the outputs are up.
fn run(
    config: config::Config,
    config_path: PathBuf,
    startup: Option<&str>,
    config_notice: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new()?;
    let mut state = State::new(
        config,
        config_path,
        event_loop.handle(),
        event_loop.get_signal(),
    )?;
    sandbox::set_trusted(&state.config.clipboard.trusted_apps);
    spawn::stop_stale_scopes(&state.env, state.config.session.end_processes);
    desktopview::setup(&mut state);
    dialogview::setup(&mut state);
    state.refresh_recent();
    state.keyboard_setup();
    toolkits::refresh(&mut state);
    tray::setup(&mut state);
    backend::init(&mut state)?;
    configwatch::setup(&mut state);
    appindex::setup(&mut state);
    quickwatch::setup(&mut state);
    recentwatch::setup(&mut state);
    xwayland::start(&mut state);
    event_loop.handle().insert_source(
        Generic::new(spawn::sigchld_fd()?, Interest::READ, Mode::Level),
        |_, fd, _| {
            spawn::drain_signalfd(fd);
            spawn::reap_children();
            Ok(PostAction::Continue)
        },
    )?;
    event_loop.handle().insert_source(
        Generic::new(spawn::term_fd()?, Interest::READ, Mode::Level),
        |_, fd, state| {
            if let Some(sig) = spawn::read_signal(fd) {
                log::info!("signal {sig}: shutting down");
                sessionfade::out(state, Box::new(|state| state.signal.stop()));
            }
            Ok(PostAction::Continue)
        },
    )?;
    power::setup(&mut state);
    nightlight::setup(&mut state);
    idle::setup(&mut state);
    debugview::config_changed(&mut state);
    state.apply_wallpaper_config();

    log::info!(
        "running on WAYLAND_DISPLAY={}",
        state.socket().to_string_lossy()
    );
    let session_begun = spawn::begin_session(&state.env);
    spawn::theme_portal_gtk(
        &state.env,
        tray::settings::Appearance::live(&state).gtk_theme(),
    );
    if let Some(cmd) = startup {
        spawn::spawn(cmd, &state.env, None);
    }
    if let Some(body) = config_notice {
        event_loop.handle().insert_idle(move |state| {
            notifyview::notice(state, "Configuration", &body);
        });
    }
    autostart::start(&mut state, session_begun);

    event_loop.run(None, &mut state, |state| {
        state.popups.cleanup();
        state.refresh_toplevels();
        state.refresh_workspaces();
        sessionend::tick(state);
        state.ime_field_sync();
        state.decode_wanted_icons();
        if let Err(e) = state.dh.flush_clients() {
            log::warn!("flush: {e}");
        }
    })?;
    tray::shutdown(&mut state);
    // Open windows end with the session: their places and apps are what the next
    // one restores.
    session::end(&mut state);
    shutdown(state, event_loop);
    Ok(())
}

/// After the loop stopped: clients go with the display, the session's variables
/// leave the user manager, the state file lands. `[session] end_processes =
/// false` leaves the login session and the app scopes alone.
fn shutdown(state: State, event_loop: EventLoop<'static, State>) {
    let env = state.env.clone();
    let end_processes = state.config.session.end_processes;
    drop(state);
    drop(event_loop);
    spawn::end_session_env(&env, end_processes);
    state::flush();
    log::info!("shut down");
}
