#!/bin/sh
# Prepares the test server and runs sshd in the foreground.
set -eu

# Host key: generated once, kept in the sshd-hostkeys volume.
if [ ! -f /etc/ssh/keys/ssh_host_ed25519_key ]; then
    ssh-keygen -q -N "" -t ed25519 -f /etc/ssh/keys/ssh_host_ed25519_key
fi
chmod 600 /etc/ssh/keys/ssh_host_ed25519_key

# alice: both fixture keys authorized.
install -d -o alice -g alice -m 700 /home/alice/.ssh
cat /fixtures/id_ed25519.pub /fixtures/id_ed25519_pp.pub > /home/alice/.ssh/authorized_keys
chown alice:alice /home/alice/.ssh/authorized_keys
chmod 600 /home/alice/.ssh/authorized_keys

# Small tmpfs for disk-full tests, owned by alice.
if [ -d /home/alice/small ]; then
    chown alice:alice /home/alice/small
    chmod 755 /home/alice/small
fi

exec /usr/sbin/sshd -D -e
