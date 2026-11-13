# Builds the hoprs .deb on Ubuntu 24.04, for scripts/package-deb.sh --container: the
# binary then links against glibc 2.39 and installs on Ubuntu 24.04 and later. (Built on
# a newer system it would need that system's glibc.) The source and the target directory
# come from a bind mount; the image holds only the toolchain and cargo-deb.
FROM ubuntu:24.04
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
# Overrides rust-toolchain.toml's `stable` channel, so the build uses the pinned version.
ENV RUSTUP_TOOLCHAIN=$RUST_VERSION
