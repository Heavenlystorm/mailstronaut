#!/usr/bin/env bash
# Sourced by native acceptance scripts: a disposable, unprivileged Dovecot serving
# synthetic mail over TLS on loopback. Callers set `artifacts`, `imap_user` and
# `imap_password`, and call `dovecot_cleanup` from their trap.
: "${artifacts:?artifact directory}" "${imap_user:?synthetic user}" "${imap_password:?synthetic password}"
# Callers may set `dovecot_extra` to append to the server configuration.
dovecot_root="$artifacts/dovecot"
mail_root="$dovecot_root/mail/$imap_user"
mkdir -p "$dovecot_root/run" "$dovecot_root/state" "$mail_root/new" "$mail_root/cur" "$mail_root/tmp"
imap_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
user_name=$(id -un)
group_name=$(id -gn)
certificates=$(realpath crates/mailstronaut-mail/tests/certs)
printf '%s:{PLAIN}%s\n' "$imap_user" "$imap_password" > "$dovecot_root/passwd"
cat > "$dovecot_root/dovecot.conf" <<CONF
dovecot_config_version = 2.4.0
dovecot_storage_version = 2.4.0
protocols = imap
listen = 127.0.0.1
base_dir = $dovecot_root/run
state_dir = $dovecot_root/state
log_path = $dovecot_root/dovecot.log
info_log_path = $dovecot_root/dovecot.log
default_internal_user = $user_name
default_login_user = $user_name
default_internal_group = $group_name
first_valid_uid = 1
auth_mechanisms = plain login
ssl = required
ssl_server_cert_file = $certificates/localhost.pem
ssl_server_key_file = $certificates/localhost-key.pem
mail_driver = maildir
mail_path = $dovecot_root/mail/%{user}
mail_uid = $user_name
mail_gid = $group_name
passdb file {
  driver = passwd-file
  passwd_file_path = $dovecot_root/passwd
}
userdb static {
  driver = static
  fields {
    uid = $user_name
    gid = $group_name
    home = $dovecot_root/mail/%{user}
  }
}
namespace inbox {
  inbox = yes
  separator = /
  mailbox Sent {
    auto = subscribe
    special_use = \\Sent
  }
  mailbox Drafts {
    auto = subscribe
    special_use = \\Drafts
  }
  mailbox Trash {
    auto = subscribe
    special_use = \\Trash
  }
  mailbox Archive {
    auto = subscribe
    special_use = \\Archive
  }
}
# Notice other connections' changes within a second instead of the default 30.
mailbox_idle_check_interval = 1s
${dovecot_extra:-}
service imap-login {
  chroot =
  inet_listener imap {
    port = 0
  }
  inet_listener imaps {
    port = $imap_port
    ssl = yes
  }
}
service anvil {
  chroot =
}
service stats {
  chroot =
}
CONF
seed_message() {
  # A Maildir delivery: new/ needs no index, and Dovecot assigns the UID.
  cat > "$mail_root/new/$1.synthetic"
}
dovecot_pid=''
dovecot_start() {
  dovecot -F -c "$dovecot_root/dovecot.conf" > "$dovecot_root/stdout.log" 2>&1 & dovecot_pid=$!
  for _ in {1..100}; do
    python3 -c 'import socket,sys; socket.create_connection(("127.0.0.1",int(sys.argv[1])),1).close()' "$imap_port" 2> /dev/null && return 0
    kill -0 "$dovecot_pid"; sleep .1
  done
  return 1
}
dovecot_stop() {
  if [[ -n $dovecot_pid ]]; then kill "$dovecot_pid" 2>/dev/null || true; wait "$dovecot_pid" 2>/dev/null || true; dovecot_pid=''; fi
}
dovecot_cleanup() { dovecot_stop; }
# The synthetic maildir itself is the observer, independently of the client's database.
server_flags() {
  python3 - "$mail_root" "$1" "$2" <<'PY'
import glob, os, sys
root, folder, marker = sys.argv[1:]
base = root if folder == 'INBOX' else os.path.join(root, '.' + folder)
for path in glob.glob(os.path.join(base, 'cur', '*')) + glob.glob(os.path.join(base, 'new', '*')):
    if marker in open(path, errors='replace').read():
        print(os.path.basename(path).rpartition(':2,')[2])
        break
else:
    sys.exit(1)
PY
}
