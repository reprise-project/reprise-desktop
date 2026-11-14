//! The Screen Capture page's probe: `reprise-record --probe` in a subprocess off the loop, the
//! first time the page opens in a run; its answer is kept until Settings closes.

use crate::capturepage::{self, ProbeState};
use crate::registry::{self, Page};
use crate::{SettingsApp, take};
use repriseconf::recording::Probed;
use std::ffi::OsStr;
use std::process::{Command, Stdio};
use std::time::Duration;

const PROGRAM: &str = "reprise-record";

/// `program --probe`'s answer; a probe that runs past `timeout` is killed. One that is missing,
/// killed or exits non-zero is [`ProbeState::Failed`].
pub(crate) fn run_probe(program: &OsStr, timeout: Duration) -> ProbeState {
    let mut cmd = Command::new(program);
    cmd.arg("--probe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match reprisefs::subprocess::run(&mut cmd, timeout) {
        Ok(ran) if ran.status.success() => ProbeState::Done(Probed::parse(&ran.stdout)),
        Ok(ran) => {
            log::warn!("{PROGRAM} --probe ended with {}", ran.status);
            ProbeState::Failed
        }
        Err(reprisefs::subprocess::Error::Timeout) => {
            log::warn!("{PROGRAM} --probe did not answer; killed it");
            ProbeState::Failed
        }
        Err(e) => {
            log::warn!("{}: {e}", program.to_string_lossy());
            ProbeState::Failed
        }
    }
}

impl SettingsApp {
    /// Start the probe the first time the page opens, and grey the page by what is known.
    pub(crate) fn ensure_capture(&mut self) {
        if registry::open(&self.sheet) == Page::ScreenCapture
            && self.capture_probe == ProbeState::Unknown
        {
            self.capture_probe = ProbeState::Checking;
            self.start_capture();
        }
        self.show_capture();
    }

    /// The probe ended: keep its answer and grey the page by it. True when it ended.
    pub(crate) fn capture_done(&mut self) -> bool {
        let Some(rx) = &self.capture_job else {
            return false;
        };
        let state = match take(rx) {
            Ok(None) => return false,
            Ok(Some(state)) => state,
            Err(()) => ProbeState::Failed,
        };
        self.capture_job = None;
        self.capture_probe = state;
        self.show_capture();
        true
    }

    /// Grey the page by the probe and say where it is; after any rebuild of the page.
    pub(crate) fn show_capture(&mut self) {
        let page = &mut self.sheet.pages[Page::ScreenCapture.index()];
        capturepage::show_probe(page, &self.want, &self.capture_probe);
    }

    #[cfg(not(test))]
    fn start_capture(&mut self) {
        // Long enough for every encoder of every codec, each tried for at most 5 s.
        const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
        self.capture_job = Some(self.spawn(|| run_probe(OsStr::new(PROGRAM), PROBE_TIMEOUT)));
    }

    /// Tests feed `capture_job` themselves, so no recorder runs.
    #[cfg(test)]
    fn start_capture(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseconf::recording::AudioCodec;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    fn script(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("reprise-record");
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn a_probe_that_hangs_is_killed_and_fails() {
        let dir = tempfile::tempdir().unwrap();
        let hang = script(dir.path(), "#!/bin/sh\nexec sleep 60\n");
        let started = Instant::now();
        assert_eq!(
            run_probe(hang.as_os_str(), Duration::from_millis(300)),
            ProbeState::Failed
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_probe_that_crashes_or_is_missing_fails() {
        let dir = tempfile::tempdir().unwrap();
        let crash = script(
            dir.path(),
            "#!/bin/sh\necho 'video h264 software'\nkill -SEGV $$\n",
        );
        assert_eq!(
            run_probe(crash.as_os_str(), Duration::from_secs(5)),
            ProbeState::Failed
        );
        assert_eq!(
            run_probe(
                OsStr::new("/nonexistent/reprise-record"),
                Duration::from_secs(5)
            ),
            ProbeState::Failed
        );
    }

    #[test]
    fn a_probe_that_answers_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let ok = script(
            dir.path(),
            "#!/bin/sh\nprintf 'video h264 software\\nnew line\\naudio aac\\n'\n",
        );
        let ProbeState::Done(p) = run_probe(ok.as_os_str(), Duration::from_secs(5)) else {
            panic!("the probe answered");
        };
        assert_eq!(p.audio, [AudioCodec::Aac]);
        assert_eq!(p.video.len(), 1, "the line it does not know is skipped");
    }
}
