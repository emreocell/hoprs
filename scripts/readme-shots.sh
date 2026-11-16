#!/usr/bin/env bash
# Captures the README screenshots of hoprs (docs/images/*.png), driven for real
# against the Docker test servers, like site/scripts/capture/run.sh (which this reuses:
# same Dockerfile, sites.toml and planted trees; the landing page shots are unaffected).
#   scripts/readme-shots.sh            # capture, convert, tear everything down
#   KEEP=1 scripts/readme-shots.sh     # leave the servers and the capture box running
#   LOCAL_DEB=target/deb-ubuntu-24.04/debian/hoprs_1.3.0-1_amd64.deb scripts/readme-shots.sh
#                                      # shoot a .deb built by scripts/package-deb.sh
# Without LOCAL_DEB the box downloads DEB_URL (default: the release in the Dockerfile).
# Click coordinates are physical pixels (logical x 2), found by capturing and looking.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
cap="$repo/site/scripts/capture"
out="$repo/docs/images"
work=$(mktemp -d)
box=hoprs-readme-shots
sshd=hoprs-test-sshd-1
compose=(docker compose -f "$repo/docker/compose.yml" -p hoprs-test)

cleanup() {
    if [ -z "${KEEP:-}" ]; then
        docker rm -f "$box" >/dev/null 2>&1 || true
        "${compose[@]}" down -v >/dev/null 2>&1 || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT

echo "==> test servers"
"${compose[@]}" up -d --build --wait sshd ftp

echo "==> remote tree (a small web project in alice's home)"
docker exec -i "$sshd" bash -s <<'REMOTE'
set -euo pipefail
d=/home/alice/web-app
rm -rf "$d"
mkdir -p "$d"/{src/components,src/pages,assets/img,assets/fonts,logs,config,releases}
cd "$d"
printf 'import { render } from "./render";\nrender(document.getElementById("root"));\n' > src/main.ts
printf 'export function render(el) {\n  el.textContent = "ok";\n}\n' > src/render.ts
for f in Header Footer Nav Card; do printf 'export default function %s() { return null; }\n' "$f" > "src/components/$f.tsx"; done
for f in index about pricing; do printf 'export default {};\n' > "src/pages/$f.tsx"; done
head -c 184320 /dev/urandom > assets/img/hero.webp
head -c 40960 /dev/urandom > assets/img/logo.png
head -c 98304 /dev/urandom > assets/fonts/inter.woff2
head -c 31457280 /dev/urandom > releases/web-app-2.4.0.tar.gz
cat > deploy.sh <<'S'
#!/usr/bin/env bash
set -euo pipefail
npm ci && npm run build
rsync -a dist/ /var/www/web-app/
sudo systemctl reload nginx
S
chmod 755 deploy.sh
cat > nginx.conf <<'S'
server {
    listen 80;
    server_name build.example;
    root /var/www/web-app;
    location / { try_files $uri /index.html; }
}
S
printf '{\n  "name": "web-app",\n  "private": true,\n  "scripts": { "build": "vite build" }\n}\n' > package.json
head -c 61440 /dev/urandom | base64 > package-lock.json
printf '# web-app\n\nStatic site for build.example.\n' > README.md
printf 'node_modules\ndist\n.env\n' > .gitignore
printf '<!doctype html>\n<div id="root"></div>\n<script type="module" src="/src/main.ts"></script>\n' > index.html
printf '{ "compilerOptions": { "strict": true } }\n' > tsconfig.json
printf 'PORT=8080\n' > config/app.env
for i in $(seq 1 400); do echo "127.0.0.1 - - [13/Nov/2026:10:$((i % 60)):00 +0000] \"GET / HTTP/1.1\" 200 512"; done > logs/access.log
for i in $(seq 1 30); do echo "2026/11/13 10:$((i % 60)):00 [warn] upstream timed out"; done > logs/error.log
# Believable, varied dates: older sources, recent logs and releases.
touch -d '2026-10-02 09:14' src src/* src/*/* tsconfig.json .gitignore
touch -d '2026-10-21 16:40' assets assets/* assets/*/* index.html README.md
touch -d '2026-11-03 11:05' config config/* nginx.conf package.json package-lock.json
touch -d '2026-11-12 18:20' deploy.sh releases releases/*
touch -d '2026-11-14 08:57' logs logs/*
chown -R alice:alice "$d"
# A neutral prompt: the container's hostname is a random id.
grep -q 'alice@build' /home/alice/.bashrc \
    || printf '%s\n' "PS1='\\[\\033[01;32m\\]\\u@build\\[\\033[00m\\]:\\[\\033[01;34m\\]\\w\\[\\033[00m\\]\\$ '" >> /home/alice/.bashrc
REMOTE

echo "==> capture box"
# Built from a temporary context (the Dockerfile and a debs/ folder), so nothing lands in
# the repo: the local .deb when LOCAL_DEB is set, else an empty debs/ and DEB_URL.
mkdir -p "$work/ctx/debs"
cp "$cap/Dockerfile" "$work/ctx/"
if [ -n "${LOCAL_DEB:-}" ]; then cp "$LOCAL_DEB" "$work/ctx/debs/"; fi
build_args=()
if [ -n "${DEB_URL:-}" ]; then build_args=(--build-arg "DEB_URL=$DEB_URL"); fi
docker build -q -t hoprs-readme-shots "${build_args[@]}" "$work/ctx" >/dev/null
docker rm -f "$box" >/dev/null 2>&1 || true
docker run -d --name "$box" --network host \
    --add-host build.example:127.0.0.1 --add-host files.example:127.0.0.1 \
    -v "$cap:/capture:ro" -v "$repo/docker/fixtures/id_ed25519:/fixture/id_ed25519:ro" \
    hoprs-readme-shots sleep infinity >/dev/null
docker exec "$box" dpkg-query -W hoprs   # which .deb this run shoots

docker exec -i "$box" bash -s <<'LOCAL'
set -euo pipefail
useradd -m -s /bin/bash dev
H=/home/dev
mkdir -p "$H/.ssh" "$H/.config/hoprs"
install -m 600 /fixture/id_ed25519 "$H/.ssh/id_ed25519"
ssh-keyscan -p 2222 build.example > "$H/.ssh/known_hosts" 2>/dev/null
cp /capture/sites.toml "$H/.config/hoprs/sites.toml"
d=$H/projects/web-app
mkdir -p "$d"/{src/components,src/pages,assets/img,assets/fonts,dist/assets,config,logs}
cd "$d"
printf 'export default {};\n' > src/main.ts
for f in Header Footer Nav Card; do printf 'export default function %s() { return null; }\n' "$f" > "src/components/$f.tsx"; done
printf 'export default {};\n' > src/pages/index.tsx
head -c 120000 /dev/urandom > dist/assets/index-8f2c1a.js
head -c 24000 /dev/urandom > dist/assets/index-3b9e0d.css
printf '<!doctype html>\n' > dist/index.html
head -c 184320 /dev/urandom > assets/img/hero.webp
printf '#!/usr/bin/env bash\nset -euo pipefail\nnpm ci && npm run build\n' > deploy.sh
chmod 755 deploy.sh
printf 'server {\n    listen 80;\n}\n' > nginx.conf
printf '{\n  "name": "web-app"\n}\n' > package.json
head -c 61440 /dev/urandom | base64 > package-lock.json
printf '# web-app\n\nStatic site for build.example.\n' > README.md
printf '# Changelog\n\n## 2.4.1\n\n- Fix the pricing page on narrow screens.\n' > CHANGELOG.md
printf 'node_modules\ndist\n.env\n' > .gitignore
printf '<!doctype html>\n' > index.html
printf '{ "compilerOptions": { "strict": true } }\n' > tsconfig.json
# README shots: most of the project exists on both sides, so the compare panel shows a short
# "Only on this computer" group (CHANGELOG, dist, the 64 MB tarball), one server-only folder
# (releases/) and a "Different" group (size or time differs); the rest is identical.
printf 'export default {};\n' > src/pages/about.tsx
printf 'export default {};\n' > src/pages/pricing.tsx
printf 'export function render(el) {\n  el.textContent = "ok";\n}\n' > src/render.ts
head -c 40960 /dev/urandom > assets/img/logo.png
head -c 98304 /dev/urandom > assets/fonts/inter.woff2
printf 'PORT=3000\nDEBUG=1\n' > config/app.env
for i in $(seq 1 120); do echo "127.0.0.1 - - [12/Nov/2026:10:$((i % 60)):00 +0000] \"GET / HTTP/1.1\" 200 512"; done > logs/access.log
for i in $(seq 1 12); do echo "2026/11/12 10:$((i % 60)):00 [warn] upstream timed out"; done > logs/error.log
head -c 67108864 /dev/urandom > web-app-2.4.1.tar.gz
touch -d '2026-10-02 09:14' src src/* src/*/* tsconfig.json .gitignore
touch -d '2026-10-21 16:40' assets assets/* assets/*/* index.html README.md
touch -d '2026-11-03 11:05' nginx.conf package.json package-lock.json deploy.sh config config/*
touch -d '2026-11-12 09:10' logs logs/*
touch -d '2026-11-14 09:31' dist dist/* dist/*/* CHANGELOG.md web-app-2.4.1.tar.gz
chown -R dev:dev "$H"
LOCAL

x() { docker exec "$box" env DISPLAY=:99 "$@"; }
# click X Y [COUNT]: iced takes the pointer position from motion events, so move, settle, click.
click() {
    x xdotool mousemove "$1" "$2"
    sleep 0.3
    x xdotool click --repeat "${3:-1}" --delay 80 1
}
# pixel X Y: the screen's colour there, as srgb(r,g,b).
pixel() { x import -window root -crop "1x1+$1+$2" -format '%[pixel:p{0,0}]' info:; }
# until_white X Y: waits (up to 20 s) for the tab underline at X Y; switching tabs while
# the throttled upload runs can take a few seconds to show.
until_white() {
    for _ in $(seq 1 40); do [ "$(pixel "$1" "$2")" = 'srgb(248,248,248)' ] && return 0; sleep 0.5; done
    echo "tab at $1,$2 never became active" >&2
    return 1
}
shoot() { x import -window root "/tmp/$1.png"; docker cp -q "$box:/tmp/$1.png" "$work/$1.png"; }

# launch [LANG]: starts hoprs (English unless a locale is given) and sizes the window.
launch() {
    local lang=${1:-C.UTF-8}
    docker exec -d -u dev -e HOME=/home/dev -e DISPLAY=:99 -e XDG_RUNTIME_DIR=/tmp \
        -e ICED_BACKEND=tiny-skia -e WINIT_X11_SCALE_FACTOR=2 \
        -e LANG="$lang" -e LC_ALL="$lang" "$box" bash -c 'hoprs > /tmp/hoprs.log 2>&1'
    for _ in $(seq 1 50); do x xdotool search --name '^hoprs$' >/dev/null 2>&1 && break; sleep 0.2; done
    win=$(x xdotool search --name '^hoprs$' | head -1)
    x xdotool windowsize --sync "$win" 2880 1800
    x xdotool windowfocus --sync "$win"   # no window manager: give it the keyboard by hand
    sleep 1
}
# open_files: connects "build server" and switches to its files tab, then waits for the
# listings and the folder comparison (the Differences panel) to settle.
open_files() {
    click 104 422                # sidebar: "build server"
    sleep 3
    click 968 40                 # tab: "build server sftp"
    sleep 6
    x xdotool mousemove 2400 1750
}
quit_hoprs() { docker exec "$box" pkill -x hoprs || true; sleep 1; }

echo "==> hoprs"
docker exec -d "$box" Xvfb :99 -screen 0 2880x1800x24 -nolisten tcp
sleep 1
launch
open_files

echo "==> palette"
x xdotool key ctrl+k
sleep 1
x xdotool type --delay 80 new
sleep 1
shoot palette
x xdotool key Escape
sleep 1

echo "==> compare panel: two differing files selected"
x xdotool mousemove 1200 1300
x xdotool click 5            # one wheel notch: the list jumps four rows
sleep 1
click 556 1330               # checkbox: deploy.sh
click 556 1390               # checkbox: index.html
x xdotool mousemove 2400 1750
sleep 1
shoot compare
quit_hoprs

echo "==> Brazilian Portuguese"
launch pt_BR.UTF-8
open_files
click 640 738                # local: CHANGELOG.md
x xdotool key ctrl+Return    # Upload: a finished transfer in the queue
sleep 4
click 2200 618 2             # remote: into src/ and back to .., which re-lists both folders
sleep 1.5
click 2200 318 2
sleep 4
x xdotool mousemove 2400 1750
sleep 1
shoot pt-br
quit_hoprs
docker exec "$sshd" rm -f /home/alice/web-app/CHANGELOG.md   # the English run uploads it again

echo "==> English: finished and in-flight uploads"
launch
open_files
click 640 738                # local: CHANGELOG.md
x xdotool key ctrl+Return    # Upload
sleep 5                      # the listings and the comparison refresh by themselves
# ls -la in the terminal tab now, before the throttle below slows the keystrokes down.
click 660 40                 # tab: "build server ssh"
sleep 1
click 1600 800               # terminal body
for line in 'clear' 'cd web-app && ls -la'; do
    x xdotool type --delay 25 "$line"
    x xdotool key Return
    sleep 0.6
done
sleep 0.5
click 968 40
sleep 2
click 2200 618 2             # remote: into src/ and back to .., which re-lists both folders
sleep 1.5
click 2200 318 2
sleep 4
# Slow the server's inbound traffic so the big upload is still running when captured.
docker run --rm --net "container:$sshd" --cap-add NET_ADMIN --entrypoint sh hoprs-test-sshd -c \
    'tc qdisc del dev eth0 ingress 2>/dev/null; tc qdisc add dev eth0 handle ffff: ingress &&
     tc filter add dev eth0 parent ffff: protocol ip u32 match u32 0 0 police rate 24mbit burst 256k drop flowid :1'
x xdotool mousemove 1200 800 # scroll the local list to its end
x xdotool click 5
sleep 1
click 640 1048               # local: web-app-2.4.1.tar.gz
x xdotool key ctrl+Return
x xdotool mousemove 2400 1750
sleep 9
shoot files

echo "==> terminal tab"
click 660 40                 # tab: "build server ssh"
until_white 540 78           # its underline
x xdotool mousemove 2400 1750
sleep 1
shoot terminal

echo "==> convert"
mkdir -p "$out"
for s in files compare terminal palette pt-br; do docker cp -q "$work/$s.png" "$box:/tmp/$s.png"; done
docker exec "$box" bash -c 'cd /tmp; for s in files compare terminal palette pt-br; do
    convert "$s.png" -strip -filter Lanczos -resize 1440x900 -colors 256 -define png:compression-level=9 "out-$s.png"; done'
for s in files compare terminal palette pt-br; do docker cp -q "$box:/tmp/out-$s.png" "$out/$s.png"; done
ls -l "$out"
