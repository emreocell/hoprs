# Capture box for the landing page screenshots (site/scripts/capture/run.sh).
# Ubuntu 24.04 with the released hoprs .deb, a virtual X server, xdotool to drive the app
# and the image tools for AVIF/WebP and the OG card. Same base pin as docker/deb-check.
FROM ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5
ENV DEBIAN_FRONTEND=noninteractive
ARG DEB_URL=https://github.com/engviniciusbs/hoprs/releases/download/v1.2.0/hoprs_1.2.0-1_amd64.deb
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl xvfb xdotool x11-utils \
        imagemagick libavif-bin webp openssh-client fonts-jetbrains-mono \
    && curl -fsSL -o /tmp/hoprs.deb "$DEB_URL" \
    && apt-get install -y --no-install-recommends /tmp/hoprs.deb \
    && rm -rf /var/lib/apt/lists/* /tmp/hoprs.deb
