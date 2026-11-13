# A clean Ubuntu 24.04 for scripts/check-deb.sh: nothing installed beyond the base image,
# so the package's own dependencies must bring in everything hoprs needs. The check tools
# (xvfb, desktop-file-utils, appstream) are installed by inside.sh only after hoprs, and
# never become package dependencies. apt needs the network (archive.ubuntu.com).
# Pinned by digest (the multi-arch index of ubuntu:24.04). To refresh:
#   docker buildx imagetools inspect ubuntu:24.04   (the "Digest:" line)
# and update the same pin in docker/deb-build and docker/deb-check.
FROM ubuntu:24.04@sha256:2198e4c65076e9cb0b3d695219df667c0e3b0072da64ca3c2648aca17dcecaf5
ENV DEBIAN_FRONTEND=noninteractive
# The base image is minimised: dpkg skips /usr/share/doc and more. A desktop installs
# them, and the check wants to see the licences land in /usr/share/doc/hoprs.
RUN rm -f /etc/dpkg/dpkg.cfg.d/excludes
COPY inside.sh /usr/local/bin/hoprs-deb-check
ENTRYPOINT ["/usr/local/bin/hoprs-deb-check"]
