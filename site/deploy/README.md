# Deploying hoprs.online

The landing page is a static build (`site/dist`) served by the author's VPS
(`92.113.39.90`, Ubuntu 24.04) through KoreGo's Caddy, behind Cloudflare.

## What is on the server

| Thing | Where |
|---|---|
| Deploy user | `hoprs-site` (system user, no password) |
| Releases | `/var/www/hoprs/releases/<UTC timestamp>-<sha7>/` (last 5 kept) |
| Live release | `/var/www/hoprs/current` → symlink to one release |
| Receive script | `/usr/local/bin/hoprs-site-receive` (`0755 root:root`), a copy of [`hoprs-site-receive`](hoprs-site-receive) |
| Deploy key | `~hoprs-site/.ssh/authorized_keys`, one ed25519 key with `restrict,command="/usr/local/bin/hoprs-site-receive"`; its private half is the `SITE_DEPLOY_KEY` repository secret only |
| KoreGo domain | `hoprs.online`: static route `/` → `/var/www/hoprs/current`, `immutablePrefix /assets/` (one-year `immutable` cache), `www` → apex redirect, force HTTPS, compression, Cloudflare-only, security headers incl. a `'self'`-only CSP |
| TLS | Cloudflare Origin CA certificate (ECC, `hoprs.online` + `*.hoprs.online`, expires 2041-11-10), stored in KoreGo as "Cloudflare origin hoprs"; Cloudflare SSL mode is Full (strict) |
| DNS | Cloudflare zone `hoprs.online`: `A @ → 92.113.39.90` and `CNAME www → hoprs.online`, both proxied |

KoreGo owns the Caddy config: change the domain only through KoreGo (panel or
its API/MCP), never by editing Caddy or `korego.db`.

## How a deploy happens

`.github/workflows/site.yml`, job `deploy`, on every push to `main` that touches
`site/` (and on manual dispatch from `main`):

1. `build` runs lint, typecheck, build, tests and uploads `site/dist`.
2. `deploy` pipes a tar of it over SSH, pinning the server's host key:
   `tar -C dist -c . | ssh hoprs-site@92.113.39.90 "$GITHUB_SHA"`.
3. The forced command unpacks into a new release dir, refuses one without
   `index.html`, swaps `current` atomically and prunes to the newest 5 releases by name (never the live one).
4. The job fetches `https://hoprs.online/` and checks for the `<h1>`.

The key can do nothing else: any other command, or none, prints the usage line
and exits 2.

## Rolling back

Point `current` at an older release (as root, or as `hoprs-site`):

```bash
ls -1t /var/www/hoprs/releases/
ln -sfn /var/www/hoprs/releases/<older> /var/www/hoprs/current.tmp
mv -T /var/www/hoprs/current.tmp /var/www/hoprs/current
```

Or re-run the `Site` workflow for an older commit on `main`. Caddy serves the
new target immediately; purge the Cloudflare cache if HTML looks stale.

## Rotating the deploy key

```bash
ssh-keygen -t ed25519 -N '' -C hoprs-site-deploy -f key
# replace the line in ~hoprs-site/.ssh/authorized_keys on the VPS:
#   restrict,command="/usr/local/bin/hoprs-site-receive" <contents of key.pub>
gh secret set SITE_DEPLOY_KEY < key
shred -u key
```

If the server's host key ever changes, update `SITE_HOST_KEY` in the workflow
after checking the new key on the server itself
(`cat /etc/ssh/ssh_host_ed25519_key.pub`), not with `ssh-keyscan`.
