#!/bin/sh
# Usage: entrypoint.sh plain|implicit|idle|bench
# Installs the certificate named by FTPS_CERT (default `server`) and runs vsftpd in the
# foreground with the selected configuration.
set -eu

conf="/etc/vsftpd/vsftpd-${1:?usage: entrypoint.sh plain|implicit|idle|bench}.conf"
cert="${FTPS_CERT:-server}"

install -m 644 "/fixtures/ftps/${cert}.pem" /etc/vsftpd/certs/server.pem
install -m 600 "/fixtures/ftps/${cert}.key" /etc/vsftpd/certs/server.key

# Small tmpfs for disk-full tests, owned by frank.
if [ -d /home/frank/small ]; then
    chown frank:frank /home/frank/small
    chmod 755 /home/frank/small
fi

# Benchmarks (docker/compose.bench.yml): latency on everything the container sends.
# Needs NET_ADMIN; the bench harness may change it later with `tc qdisc replace`.
if [ -n "${NETEM_DELAY:-}" ]; then
    tc qdisc replace dev eth0 root netem delay "$NETEM_DELAY" limit 100000
fi

exec /usr/sbin/vsftpd "$conf"
