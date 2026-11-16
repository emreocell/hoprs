# Capture box for the landing page and README screenshots (site/scripts/capture/run.sh,
# scripts/readme-shots.sh). Ubuntu 24.04 with hoprs installed, a virtual X server, xdotool
# to drive the app and the image tools for AVIF/WebP and the OG card. Same base pin as
# docker/deb-check. The scripts build it from a temporary context holding this file and a
# debs/ folder: a local .deb there (LOCAL_DEB=...) is installed, else DEB_URL is downloaded.
FROM ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5
ENV DEBIAN_FRONTEND=noninteractive
ARG DEB_URL=https://github.com/engviniciusbs/hoprs/releases/download/v1.3.1/hoprs_1.3.1-1_amd64.deb
COPY debs/ /tmp/debs/
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl xvfb xdotool x11-utils \
        imagemagick libavif-bin webp openssh-client fonts-jetbrains-mono \
    && if ls /tmp/debs/*.deb >/dev/null 2>&1; then \
           cp /tmp/debs/*.deb /tmp/hoprs.deb; \
       else \
           curl -fsSL -o /tmp/hoprs.deb "$DEB_URL"; \
       fi \
    && apt-get install -y --no-install-recommends /tmp/hoprs.deb \
    && rm -rf /var/lib/apt/lists/* /tmp/hoprs.deb /tmp/debs
