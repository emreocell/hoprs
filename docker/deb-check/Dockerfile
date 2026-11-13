# A clean Ubuntu 24.04 for scripts/check-deb.sh: nothing installed beyond the base image,
# so the package's own dependencies must bring in everything hoprs needs. The check tools
# (xvfb, desktop-file-utils, appstream) are installed by inside.sh only after hoprs, and
# never become package dependencies. apt needs the network (archive.ubuntu.com).
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
# The base image is minimised: dpkg skips /usr/share/doc and more. A desktop installs
# them, and the check wants to see the licences land in /usr/share/doc/hoprs.
RUN rm -f /etc/dpkg/dpkg.cfg.d/excludes
COPY inside.sh /usr/local/bin/hoprs-deb-check
ENTRYPOINT ["/usr/local/bin/hoprs-deb-check"]
