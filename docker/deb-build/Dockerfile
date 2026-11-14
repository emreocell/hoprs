# Builds the hoprs .deb on Ubuntu 24.04, for scripts/package-deb.sh (its default mode): the
# binary then links against glibc 2.39 and installs on Ubuntu 24.04 and later. (Built on
# a newer system it would need that system's glibc.) The source and the target directory
# come from a bind mount; the image holds only the toolchain and cargo-deb.
# Pinned by digest (the multi-arch index of ubuntu:24.04). To refresh:
#   docker buildx imagetools inspect ubuntu:24.04   (the "Digest:" line)
# and update the same pin in docker/deb-build and docker/deb-check.
FROM ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5
ARG RUST_VERSION
ARG CARGO_DEB_VERSION
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/opt/rustup \
    PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential ca-certificates cmake curl \
        dpkg-dev git pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | CARGO_HOME=/opt/cargo sh -s -- -y --no-modify-path --profile minimal \
            --default-toolchain "$RUST_VERSION" \
    && CARGO_HOME=/opt/cargo cargo install cargo-deb --version "$CARGO_DEB_VERSION" --locked \
    && rm -rf /opt/cargo/registry /opt/cargo/git \
    && chmod -R a+rX /opt/rustup /opt/cargo
# The same version rust-toolchain.toml pins (package-deb.sh reads it from there). Set
# here as well so rustup never tries to add the file's extra components (rustfmt,
# clippy) to the read-only toolchain while the build runs as the caller's user.
ENV RUSTUP_TOOLCHAIN=$RUST_VERSION
