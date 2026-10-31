#!/usr/bin/env bash
# Synthetic-only syscall observers in a private network namespace; no host DNS.
# Build the renderer unit-test executable in the normal pinned shell first.
set -euo pipefail
binary=$(realpath "${2:-${1:?prebuilt renderer unit-test executable}}")
if [[ ${1:-} != --inside ]]; then
  root=$(mktemp -d "$PWD/target/blitz-privacy.XXXXXX")
  mkdir -p "$root/home" "$root/cache" "$root/config" "$root/data" "$root/runtime"
  chmod 700 "$root" "$root/runtime"
  touch "$root/synthetic-only"
  cp assets/inline-fixture.png "$root/sentinel.png"
  printf 'div{background:red}\n' > "$root/sentinel.css"
  printf 'SYNTHETIC FONT BYTES, NOT A USER FILE\n' > "$root/sentinel.font"
  printf '<h1>SYNTHETIC FILE CONTENT</h1>\n' > "$root/sentinel.html"
  printf 'nameserver 127.0.0.1\noptions timeout:1 attempts:1\n' > "$root/resolv.conf"
  printf 'hosts: dns\n' > "$root/nsswitch.conf"
  bwrap --unshare-user --uid 0 --gid 0 --cap-add CAP_NET_BIND_SERVICE \
    --unshare-net --unshare-pid --die-with-parent --new-session \
    --ro-bind / / --bind "$root" "$root" --proc /proc --dev /dev \
    --tmpfs /run --tmpfs /tmp --tmpfs /etc \
    --ro-bind /etc/fonts /etc/fonts \
    --ro-bind "$root/resolv.conf" /etc/resolv.conf \
    --ro-bind "$root/nsswitch.conf" /etc/nsswitch.conf \
    --setenv HOME "$root/home" --setenv XDG_CACHE_HOME "$root/cache" \
    --setenv XDG_CONFIG_HOME "$root/config" --setenv XDG_DATA_HOME "$root/data" \
    --setenv XDG_RUNTIME_DIR "$root/runtime" --unsetenv DBUS_SESSION_BUS_ADDRESS \
    --setenv MAILSTRONAUT_RENDER_TRACE_ROOT "$root" \
    bash "$PWD/experiments/p01-blitz/privacy.sh" --inside "$binary"
  printf 'Passive Blitz resource observation passed: %s\n' "$root"
  exit 0
fi
root=${MAILSTRONAUT_RENDER_TRACE_ROOT:?private fixture root}
[[ -f $root/synthetic-only ]]
server=''
cleanup() { if [[ -n "$server" ]]; then kill "$server" 2>/dev/null || true; wait "$server" 2>/dev/null || true; fi; }
trap cleanup EXIT
# A loopback-only DNS NXDOMAIN fixture and HTTP redirect endpoint are observers,
# not a custom production resolver or fetcher. Both live inside the private netns.
python3 - "$root" <<'PY' > "$root/observer.log" 2>&1 &
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import socket,sys,threading
root=Path(sys.argv[1])
lock=threading.Lock()
def record(value):
    with lock:
        with (root/'requests.log').open('a') as f: f.write(value+'\n')
class Http(BaseHTTPRequestHandler):
    def do_GET(self):
        record('http '+self.path)
        self.send_response(302); self.send_header('Location','/redirect-target'); self.end_headers()
    def log_message(self,*args): pass
http=ThreadingHTTPServer(('127.0.0.1',0),Http)
dns=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); dns.bind(('127.0.0.1',53))
def serve_dns():
    while True:
        packet,peer=dns.recvfrom(4096)
        record('dns')
        # Echo this synthetic question with an NXDOMAIN response and no answers.
        if len(packet)>=12:
            dns.sendto(packet[:2]+b'\x81\x83'+packet[4:6]+b'\0\0\0\0\0\0'+packet[12:],peer)
threading.Thread(target=serve_dns,daemon=True).start()
(root/'http-port').write_text(str(http.server_port))
http.serve_forever()
PY
server=$!
for _ in {1..100}; do [[ -s $root/http-port ]] && break; kill -0 "$server"; sleep .05; done
[[ -s $root/http-port ]]
for mode in control denied; do
  MAILSTRONAUT_RENDER_TRACE_MODE=$mode timeout 45 \
    strace -ff -s 4096 -e trace=%network,%file -o "$root/$mode.trace" \
    "$binary" privacy::trace_resource_boundary --ignored --exact --nocapture \
    > "$root/$mode.log" 2>&1
done
python3 - "$root" <<'PY'
from pathlib import Path
import re,sys
root=Path(sys.argv[1])
traces={mode:'\n'.join(p.read_text() for p in root.glob(mode+'.trace*')) for mode in ('control','denied')}
network=re.compile(r'\b(?:socket|socketpair|connect|bind|listen|accept4?|sendto|sendmsg|sendmmsg|recvfrom|recvmsg|recvmmsg)\(')
assert network.search(traces['control']), 'network observation positive control missing'
assert 'SOCK_DGRAM' in traces['control'] and 'SOCK_STREAM' in traces['control']
requests=(root/'requests.log').read_text().splitlines()
assert 'dns' in requests and 'http /control' in requests, requests
assert all(line in ('dns','http /control') for line in requests), requests
assert not network.search(traces['denied']), 'renderer attempted a socket/network operation'
for name in ('sentinel.png','sentinel.css','sentinel.font','sentinel.html'):
    path=str(root/name)
    assert path in traces['control'], 'file observation control missing: '+name
    assert path not in traces['denied'], 'renderer accessed synthetic URL file: '+name
print('Actual resolver/UDP, TCP/redirect and four readable-file controls observed.')
print('Denied matrix: zero socket/network operations; zero accesses to email-specified synthetic files.')
PY
cat "$root/control.log" "$root/denied.log"
