FROM archlinux@sha256:52dfbd5afb23a220d5ee92dafcb4c788d96c026ed04179890c23adef4c897fa0
LABEL org.opencontainers.image.source=https://github.com/reprise-project/reprise-desktop
# The archive date names the repos' state, so a new rustc, clippy or Mesa reaches CI only
# through a commit that bumps it.
ARG ARCH_ARCHIVE=2026/10/25
COPY pacman.sh /tmp/ci/pacman.sh
COPY ci-image/ /tmp/ci/ci-image/
RUN bash /tmp/ci/ci-image/setup.sh && rm -rf /tmp/ci /var/cache/pacman/pkg/*
