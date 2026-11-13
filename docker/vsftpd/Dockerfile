# vsftpd for hoprs integration tests (plain FTP + explicit FTPS, implicit FTPS, and plain
# FTP with short idle timeouts) and benchmarks (plain FTP behind netem).
# Test-only: weak password, committed certificates.
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends vsftpd netcat-openbsd procps iproute2 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /var/run/vsftpd/empty /etc/vsftpd/certs /fixtures/ftps

RUN useradd -m -u 1101 -s /bin/bash frank && echo 'frank:frank-pass' | chpasswd

COPY fixtures/ftps/*.pem fixtures/ftps/*.key /fixtures/ftps/
COPY vsftpd/vsftpd-plain.conf vsftpd/vsftpd-implicit.conf vsftpd/vsftpd-idle.conf \
    vsftpd/vsftpd-bench.conf /etc/vsftpd/
COPY vsftpd/entrypoint.sh /entrypoint.sh
RUN chmod 755 /entrypoint.sh && chmod 600 /fixtures/ftps/*.key

ENTRYPOINT ["/entrypoint.sh"]
