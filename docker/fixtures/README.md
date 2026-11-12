# Test fixtures

These SSH keys exist **only for the Docker integration tests** (`docker/compose.yml`).
They are committed on purpose, are public knowledge and protect nothing. Never use
them for a real server or account.

- `id_ed25519` / `id_ed25519.pub`: unencrypted client key, authorized for `alice`.
- `id_ed25519_pp` / `id_ed25519_pp.pub`: client key encrypted with passphrase `fixture-pp`,
  authorized for `alice`.

Test users in the container (passwords are test-only too):

| user  | auth                                   |
|-------|----------------------------------------|
| alice | password `alice-pass`, both keys above |
| bob   | password `bob-pass` only               |
| carol | keyboard-interactive (PAM), `carol-pass` |
