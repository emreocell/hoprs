# OpenSSH server for hoprs integration tests. Test-only: weak passwords, committed keys.
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends openssh-server netcat-openbsd procps \
    && rm -rf /var/lib/apt/lists/* \
    && rm -f /etc/ssh/ssh_host_* \
    && mkdir -p /run/sshd /etc/ssh/keys /fixtures \
    # pam_loginuid cannot write the audit login uid inside a container.
    && sed -i 's/^session\s\+required\s\+pam_loginuid.so/session optional pam_loginuid.so/' /etc/pam.d/sshd

RUN useradd -m -u 1001 -s /bin/bash alice && echo 'alice:alice-pass' | chpasswd \
    && useradd -m -u 1002 -s /bin/bash bob && echo 'bob:bob-pass' | chpasswd \
    && useradd -m -u 1003 -s /bin/bash carol && echo 'carol:carol-pass' | chpasswd \
    && useradd -m -u 1004 -s /bin/bash dave && echo 'dave:dave-pass' | chpasswd \
    && useradd -m -u 1005 -s /bin/bash erin && echo 'erin:erin-pass' | chpasswd

COPY fixtures/id_ed25519.pub fixtures/id_ed25519_pp.pub /fixtures/
COPY sshd/sshd_config /etc/ssh/sshd_config
COPY sshd/entrypoint.sh /entrypoint.sh
RUN chmod 755 /entrypoint.sh

EXPOSE 22
ENTRYPOINT ["/entrypoint.sh"]
