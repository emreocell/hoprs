# FTPS test fixtures

These certificates and keys exist **only for the Docker integration tests**
(`docker/compose.yml`, services `ftp` and `ftps-implicit`). They are committed on purpose,
their private keys are public knowledge and they protect nothing. Never trust `ca.pem`
outside the tests, and never use any of these files for a real server.

- `ca.pem`: test CA (its private key was discarded after signing `server.pem`).
- `server.pem` / `server.key`: server certificate signed by `ca.pem`, SAN `IP:127.0.0.1`
  and `DNS:localhost`. Used by both FTP services by default.
- `other.pem` / `other.key`: unrelated self-signed certificate (same SANs), used to
  simulate a server whose certificate changed (`FTPS_CERT=other`).
- `server.fingerprint`: output of
  `openssl x509 -noout -fingerprint -sha256 -in server.pem`, value only.

Test user in both containers: `frank`, password `frank-pass` (test-only).

Regenerating (changes every fingerprint; update `server.fingerprint` too):

```sh
openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem -days 36500 \
  -subj "/CN=hoprs test CA" -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign"
openssl req -newkey rsa:2048 -nodes -keyout server.key -out server.csr -subj "/CN=localhost"
printf 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1,DNS:localhost\n' > server.ext
openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out server.pem \
  -days 36500 -extfile server.ext
openssl req -x509 -newkey rsa:2048 -nodes -keyout other.key -out other.pem -days 36500 \
  -subj "/CN=localhost" -addext "subjectAltName=IP:127.0.0.1,DNS:localhost"
openssl x509 -noout -fingerprint -sha256 -in server.pem | cut -d= -f2 > server.fingerprint
rm -f ca.key ca.srl server.csr server.ext
```
