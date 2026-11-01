#!/usr/bin/env bash
# Synthetic native Wayland mailbox behavior; no real account, keyring or X server.
set -euo pipefail
binary=$(realpath "${1:-target/debug/mailstronaut}")
fixture=$(realpath "${MAILSTRONAUT_HTML_FIXTURE:?prebuilt core worker test executable}")
root=$(mktemp -d "$PWD/target/p01.XXXXXX")
export XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config"
export XDG_DATA_HOME="$root/data" XDG_CACHE_HOME="$root/cache" XDG_STATE_HOME="$root/state"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"
chmod 700 "$root" "$XDG_RUNTIME_DIR"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET SWAYSOCK DBUS_SESSION_BUS_ADDRESS AT_SPI_BUS_ADDRESS
export XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=sway
export VK_DRIVER_FILES="${MAILSTRONAUT_TEST_VULKAN_ICD:?pinned software Vulkan driver}"
export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
app='' keyboard='' pointer='' compositor='' bus='' blocker='' spy='' portal=''
expected_links=0
cleanup() {
  # On an assertion failure, force the owned tracer to exit; --kill-on-exit then
  # kills its tracees. SIGTERM can leave strace waiting for its interactive child.
  if [[ -n "$app" ]]; then kill -KILL "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true; app=''; fi
  for pid in "$app" "$keyboard" "$pointer" "$compositor" "$bus" "$blocker" "$spy" "$portal"; do
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  done
}
trap cleanup EXIT
# A private bus without activation directories cannot launch host desktop services.
cat > "$root/bus.conf" <<CONFIG
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path=$root/runtime/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/><allow own="*"/>
    <allow send_destination="*"/><allow receive_sender="*"/>
  </policy>
</busconfig>
CONFIG
dbus-daemon --nofork --nopidfile --config-file="$root/bus.conf" > "$root/bus.log" 2>&1 & bus=$!
export DBUS_SESSION_BUS_ADDRESS="unix:path=$root/runtime/bus"
for _ in {1..100}; do [[ -S $root/runtime/bus ]] && break; sleep .05; done

config="$root/config.nix"
cat > "$config" <<'NIX'
{
  schemaVersion = 1;
  managed = true;
  appearance = "light";
  accounts = map (identity: {
    id = identity.id;
    provider = "manual";
    address = identity.address;
    displayName = identity.id;
    userName = identity.id;
    imap = {host = "imap.example.invalid"; port = 993; security = "tls";};
    smtp = {host = "smtp.example.invalid"; port = 587; security = "starttls";};
    auth = {kind = "password"; secretRef = "synthetic-conversation-password";};
  }) [
    {id = "conversation-synthetic"; address = "reader@example.test";}
    {id = "other-synthetic"; address = "other@example.test";}
  ];
}
NIX
cp assets/inline-fixture.png "$root/blocked.png"
python3 - "$root" <<'PYSPY' > "$root/spy.log" 2>&1 &
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
import sys
root=Path(sys.argv[1])
class Spy(BaseHTTPRequestHandler):
 def do_GET(self):
  with (root/'requests.log').open('a') as f:f.write(self.path+'\n')
  self.send_response(200 if self.path=='/control' else 302)
  self.send_header('Location','/redirect-target');self.end_headers();self.wfile.write(b'SYNTHETIC OBSERVER')
 def log_message(self,*args):pass
server=ThreadingHTTPServer(('127.0.0.1',0),Spy)
(root/'http-port').write_text(str(server.server_port));server.serve_forever()
PYSPY
spy=$!
for _ in {1..100}; do [[ -s $root/http-port ]] && break; kill -0 "$spy"; sleep .05; done
MAILSTRONAUT_HTML_HTTP_PORT=$(cat "$root/http-port"); export MAILSTRONAUT_HTML_HTTP_PORT MAILSTRONAUT_HTML_GUI=1
if [[ -n ${MAILSTRONAUT_LINK_PORTAL_FIXTURE:-} ]]; then
  touch "$root/synthetic-link-portal"
  MAILSTRONAUT_LINK_PORTAL_ROOT="$root" "$MAILSTRONAUT_LINK_PORTAL_FIXTURE" \
    link_portal::tests::serve_native_portal --ignored --exact --nocapture > "$root/portal.log" 2>&1 & portal=$!
  for _ in {1..100}; do [[ -s $root/portal-ready ]] && break; kill -0 "$portal"; sleep .05; done
  [[ -s $root/portal-ready ]]
fi
strace -ff -s 1024 -e trace=%network,%file -o "$root/control.trace" python3 - "$root" "$MAILSTRONAUT_HTML_HTTP_PORT" <<'PYCONTROL'
from pathlib import Path
import urllib.request,sys
assert (Path(sys.argv[1])/'blocked.png').read_bytes()
assert urllib.request.urlopen('http://127.0.0.1:'+sys.argv[2]+'/control').status==200
PYCONTROL
export MAILSTRONAUT_THREADS_CONFIG="$config" MAILSTRONAUT_THREADS_GUI_BINARY="$binary"
"$fixture" seed_conversation_gui --ignored --exact --nocapture > "$root/seed.log" 2>&1
cat > "$root/sway.conf" <<'SWAY'
xwayland disable
output HEADLESS-1 mode 1280x840
seat seat0 fallback true
default_border none
SWAY
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway --unsupported-gpu --config "$root/sway.conf" > "$root/compositor.log" 2>&1 & compositor=$!
for _ in {1..200}; do
  socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'wayland-*' -print -quit)
  sway_socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)
  [[ -n "$socket" && -S $sway_socket ]] && break
  kill -0 "$compositor"; sleep .05
done
[[ -S $socket ]]
export WAYLAND_DISPLAY="$socket"
export SWAYSOCK="$sway_socket"
[[ -S $SWAYSOCK ]]
WLRCTL_TEST_KEEP_POINTER=1 wlrctl pointer move 0 0 > "$root/pointer.log" 2>&1 & pointer=$!
sleep .5
start() {
  # Retain the keyboard capability, and make a live keyed device current before
  # a reconnect. A destroyed one-shot virtual keyboard can leave this synthetic
  # seat without a keymap, causing Modifiers to precede Keymap for a new client.
  if [[ -n "$keyboard" ]]; then kill "$keyboard"; wait "$keyboard" 2>/dev/null || true; fi
  wtype -k Escape -s 300000 > "$root/keyboard.log" 2>&1 & keyboard=$!
  sleep .3
  strace --kill-on-exit -ff -s 1024 -e trace=%network,%file -o "$root/$1.trace" \
    "$binary" --config "$config" > "$root/$1-app.log" 2>&1 & app=$!
  for _ in {1..150}; do
    if swaymsg -t get_tree | python3 -c 'import json,sys; t=json.load(sys.stdin); walk=lambda n: n.get("app_id")=="io.github.eduardz1.Mailstronaut" or any(walk(c) for c in n.get("nodes",[])); sys.exit(0 if walk(t) else 1)'; then break; fi
    kill -0 "$app"; sleep .1
  done
  sleep 1
  kill -0 "$app"
}
stop() {
  swaymsg '[app_id="io.github.eduardz1.Mailstronaut"] kill' > /dev/null
  for _ in {1..100}; do kill -0 "$app" 2>/dev/null || break; sleep .05; done
  if kill -0 "$app" 2>/dev/null; then printf '%s\n' 'Application did not close normally' >&2; exit 1; fi
  wait "$app"; app=''
}
capture() { timeout 15 grim "$root/$1.png"; }
click() {
  swaymsg seat seat0 cursor set "$1" "$2" > /dev/null
  swaymsg seat seat0 cursor press button1 > /dev/null
  swaymsg seat seat0 cursor release button1 > /dev/null
  sleep .3
}

check_seen() {
  python3 - "$root" "$@" <<'PYSEEN'
from pathlib import Path
import json,sqlite3,sys
root=Path(sys.argv[1]);ids=json.loads((root/'conversation-ids.json').read_text())
c=sqlite3.connect(root/'data/mailstronaut/mail.sqlite3');expected=set(sys.argv[2:])
for key,mid in ids.items():
 seen=c.execute('SELECT min(seen) FROM effective_memberships WHERE message_id=?',(mid,)).fetchone()[0]
 assert seen==int(key in expected),(key,seen,expected)
print('Only explicitly opened originals have Seen:',sorted(expected))
PYSEEN
}
copy_body() {
  wtype -s 150 -M ctrl -M shift t -m shift -m ctrl -s 400 -M ctrl a -m ctrl -s 200 -M ctrl c -m ctrl
  # Clipboard offers are asynchronous Wayland events. Observe the actual value
  # with a deadline instead of assuming it exists 200 ms after sending a key.
  for _ in {1..30}; do
    if timeout 1 wl-paste --no-newline > "$root/$1-body.txt" 2> "$root/$1-clipboard.log" && grep -q "$2" "$root/$1-body.txt"; then return; fi
    sleep .1
  done
  capture "$1-copy-failure"
  printf 'Expected copied body was not offered: %s\n' "$1" >&2
  return 1
}
open_html() { click 350 "$1"; click 684 246; sleep .5; }
check_links() {
  [[ -n "$portal" ]] || return 0
  python3 - "$root" "$1" <<'PYLINKS'
from pathlib import Path
import json,sys
root=Path(sys.argv[1]);path=root/'portal-requests.jsonl'
requests=[json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
assert len(requests)==int(sys.argv[2]),requests
expected='http://127.0.0.1:'+(root/'http-port').read_text().strip()+'/activate'
assert all(r['uri']==expected for r in requests),requests
print('Explicit web-link portal calls:',len(requests))
PYLINKS
}
wait_links() {
  for _ in {1..50}; do if check_links "$1" > "$root/link-receipt.log" 2>&1; then cat "$root/link-receipt.log"; return; fi; sleep .1; done
  capture link-count-failure
  cat "$root/link-receipt.log" >&2; return 1
}
start initial
sleep .5
capture listed
if [[ ${MAILSTRONAUT_HTML_CAPTURE_ONLY:-0} == 1 ]]; then
  click 350 150; sleep 1; capture alpha
  printf 'Inspection only: %s\n' "$root"; exit 0
fi
check_seen
wtype -s 150 -M ctrl l -m ctrl -k Down -s 200 -M ctrl r -m ctrl -s 500
check_seen
wtype -s 150 -k Return -s 800
capture alpha
check_seen alpha
check_links 0
copy_body alpha 'Synthetic HTML Alpha plain alternative'
open_html 150
capture alpha-html
# Clicks identify links without forwarding DOM events; the app itself never fetches.
click 650 507
capture link-request
if [[ -n "$portal" ]]; then
  wait_links 1
  # Kit owns menu focus, keyboard activation, arrow navigation and dismissal.
  click 835 246
  capture links-menu
  wtype -s 150 -k Escape -s 200 -k Return -s 200 -k Down -s 150 -k Return -s 400
  capture links-keyboard
  wait_links 2
  printf '1\n' > "$root/portal-response"
  wtype -s 150 -M ctrl -M shift l -m shift -m ctrl -s 200 -k Return -s 200 -k Down -k Return -s 400
  wait_links 3
  expected_links=3
  capture link-cancelled
  printf '0\n' > "$root/portal-response"
  if [[ ${MAILSTRONAUT_LINKS_ONLY:-0} == 1 ]]; then
    stop; touch "$root/portal-stop"; wait "$portal"; portal=''
    printf 'Native link inspection passed: %s\n' "$root"; exit 0
  fi
fi
# Native page/end/home scrolling uses the standard ScrollHandle.
click 1100 620
wtype -s 150 -k End -s 600
capture end
wtype -s 150 -k Home -s 600
capture home
# Native logical resize changes the CSS media query; native scale keeps that query.
swaymsg output HEADLESS-1 mode 960x840 > /dev/null
sleep .8
capture narrow
swaymsg output HEADLESS-1 mode 2560x1680 scale 2 > /dev/null
sleep 1
capture hidpi
if [[ -n "$portal" ]]; then
  # Sway input coordinates stay logical at 2x; exercise the actual HTML hit path.
  click 650 535
  wait_links 4
  expected_links=4
  capture hidpi-link
fi
swaymsg output HEADLESS-1 mode 1280x840 scale 1 > /dev/null
sleep .8
wtype -s 150 -M ctrl -M shift d -m shift -m ctrl -s 700
capture dark
check_seen alpha
# Beta deliberately refers to the same CID without owning the image.
open_html 270
capture beta
if [[ -n "$portal" ]]; then
  click 835 246
  wtype -s 150 -k Escape
  check_links "$expected_links"
fi
check_seen alpha beta
copy_body beta 'Synthetic HTML Beta plain alternative'
open_html 400
capture bad-image
check_seen alpha beta bad-image
copy_body bad-image 'Safe plain fallback for a broken local image.'
click 350 520
copy_body plain 'Synthetic plain message without HTML.'
check_seen alpha beta bad-image plain
capture plain
# Rapid switching cannot repaint a retired source or resurrect its raster.
for _ in {1..8}; do open_html 150; open_html 270; done
sleep .5
capture switched-beta
# Hiding and page restoration do not create an open authorization.
wtype -s 150 -M ctrl , -m ctrl -s 500
capture hidden
sleep 1
wtype -s 150 -M ctrl 1 -m ctrl -s 600
capture restored
check_seen alpha beta bad-image plain
stop
start offline
check_seen alpha beta bad-image plain
open_html 150
capture offline
stop
if [[ -n "$portal" ]]; then
  check_links "$expected_links"
  touch "$root/portal-stop"
  wait "$portal"; portal=''
fi
python3 - "$root" <<'PYPIXELS'
from pathlib import Path
import subprocess,sys
root=Path(sys.argv[1])
def read(name):
 path=root/(name+'.png')
 w,h=map(int,subprocess.check_output(['magick','identify','-format','%w %h',str(path)]).split())
 raw=subprocess.check_output(['magick',str(path),'-depth','8','RGB:-'])
 assert len(raw)==w*h*3
 return w,h,raw
def sample(image,x,y):
 w,h,raw=image;assert 0<=x<w and 0<=y<h
 start=(y*w+x)*3;return tuple(raw[start:start+3])
def bands(image):
 w,h,raw=image
 found={color:[w,h,-1,-1,0] for color in ((255,0,0),(0,255,0),(0,0,255))}
 for i,color in enumerate(zip(raw[0::3],raw[1::3],raw[2::3])):
  bounds=found.get(color)
  if bounds is not None:
   y,x=divmod(i,w)
   bounds[:]=[min(bounds[0],x),min(bounds[1],y),max(bounds[2],x),max(bounds[3],y),bounds[4]+1]
 return list(found.values())
for name,scale in [('alpha',1),('alpha-html',1),('link-request',1),('home',1),('hidpi',2),('dark',1),('offline',1)]:
 im=read(name)
 assert im[:2]==(1280*scale,840*scale),(name,im[:2])
 assert sample(im,800*scale,350*scale)==(51,170,102),name+' must paint its own wide Alpha marker'
 observed=bands(im)
 for x,y,right,bottom,count in observed:
  # Upsampling the small PNG blends its band edges. Require a solid interior
  # and exact physical height/spacing, rather than treating blended edges as pure.
  assert 24*scale<=right-x+1<=32*scale and bottom-y+1==32*scale,(name,observed)
  assert count==(right-x+1)*(bottom-y+1),(name,observed)
 assert observed[1][0]-observed[0][0]==32*scale and observed[2][0]-observed[1][0]==32*scale
 assert len({b[1] for b in observed})==1
 print(name+': own Alpha marker; ordered 32-CSS-pixel RGB bands at scale '+str(scale))
im=read('narrow')
assert im[:2]==(960,840) and sample(im,700,350)==(51,85,170),'logical resize must trigger max-width media query'
im=read('end')
assert sample(im,800,710)==(0,255,255),'End must paint the document tail'
assert all(b[4]==0 for b in bands(im)),'End must not reuse a top viewport'
assert sample(read('dark'),800,600)==(32,36,43),'actual Blitz dark media query'
assert sample(read('dark'),10,80)[0]<128,'native Kit dark theme'
assert sample(read('alpha'),10,80)[0]>128,'native Kit light theme'
for name,marker in [('beta',(221,221,102)),('switched-beta',(221,221,102)),('restored',(221,221,102)),('bad-image',(204,136,170))]:
 im=read(name)
 # Require B's distinct frame before accepting absence of A's image.
 assert sample(im,800,350)==marker,name+' must paint its own marker'
 assert all(b[4]==0 for b in bands(im)),name+' cannot contain Alpha raster pixels'
 print(name+': own current message marker; no retired RGB image')
assert all(b[4]==0 for b in bands(read('plain'))),'plain-text mode cannot retain HTML image pixels'
print('Independent native pixels: resize/media, scroll/Home/End, physical 2x scale, theme and source retirement passed.')
PYPIXELS
python3 - "$root" <<'PYOBSERVE'
from pathlib import Path
import re,sys
root=Path(sys.argv[1]);control='\n'.join(p.read_text() for p in root.glob('control.trace*'))
expected={
 'alpha':'Synthetic HTML Alpha plain alternative · Bonjour · 世界.',
 'beta':'Synthetic HTML Beta plain alternative.',
 'bad-image':'Safe plain fallback for a broken local image.',
 'plain':'Synthetic plain message without HTML.\r\n',
}
for name,body in expected.items():
 assert (root/(name+'-body.txt')).read_bytes()==body.encode(),name+' must copy its own exact Unicode alternative'
assert 'AF_INET' in control and str(root/'blocked.png') in control
assert (root/'requests.log').read_text().splitlines()==['/control']
for mode in ('initial','offline'):
 trace='\n'.join(p.read_text() for p in root.glob(mode+'.trace*'))
 assert 'socket(AF_INET' not in trace, 'application attempted IPv4/IPv6 network I/O'
 assert str(root/'blocked.png') not in trace, 'HTML opened the synthetic file URL'
 assert not re.search(r'panic|fatal|error:',(root/(mode+'-app.log')).read_text(),re.I)
print('Native cached HTML: no IP sockets or email-specified file opens; HTTP/file observer controls passed.')
PYOBSERVE
printf 'Native cached HTML/MIME, local CID isolation, modes, scroll, resize, HiDPI, theme, Seen and offline checks passed: %s\n' "$root"
