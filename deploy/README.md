# Deploying a client

Runbook for the hosting model in [OPERATIONS.md](../docs/OPERATIONS.md) (D-01): one
`host` binary and one SQLite database per client on our own VPS in the UAE, systemd socket
activation, Caddy for TLS, Litestream to S3-compatible storage (MinIO) on a second UAE VPS.

What is in this folder:

| File | Goes to | Purpose |
|---|---|---|
| `Dockerfile` | build only | Builds the release binary (and, optionally, a container image) |
| `systemd/white-label@.service` | `/etc/systemd/system/` | The app of client `%i`: restore if needed, migrate, serve |
| `systemd/white-label@.socket` | `/etc/systemd/system/` | The listening socket, so restarts refuse nobody |
| `systemd/litestream@.service` | `/etc/systemd/system/` | Continuous replication of the client's database |
| `litestream/litestream.yml` | `/etc/white-label/<client>/litestream.yml` | Replica to MinIO |
| `litestream/litestream.env.example` | `/etc/white-label/<client>/litestream.env` | The client's MinIO key |
| `env.production.example` | `/etc/white-label/<client>/env` | Every production variable, commented |
| `Caddyfile.example` | `/etc/caddy/Caddyfile` | TLS and proxy for the client's domain |

**Status.** Sections are marked *tested here* (run in the build sandbox, outside systemd and
without Docker, Caddy, Litestream or MinIO) or *not yet tested*. The table in
[OPERATIONS.md](../docs/OPERATIONS.md) section 12 has the full picture. Read it before relying
on a step.

## 0. Layout on the VPS

```
/opt/white-label/releases/<tag>/host      one binary per release, never edited
/opt/white-label/clients/<client>/host    symlink to the client's release  (rollout = move the link)
/etc/white-label/<client>/                env, client.toml, litestream.yml, litestream.env  (root, 0600)
/var/lib/white-label/<client>/            app.db (+ -wal, -shm), app/ (uploads), snapshots/  (user wl-<client>)
```

`<client>` is a short lowercase name (`demo`, `acme`). Each client has its own OS user
`wl-<client>`, its own port (3100 for the first, 3101 for the next, ...), its own domain.
Views, translations and `public/` files are compiled into the binary; **nothing but the
binary, `client.toml` and `env` is needed next to it.** (*Tested here*: the binary served the
sign-in page and `app.css` from a directory with no `resources/` or `public/`, with
`APP_DEBUG=false`. The embedded files are used only when `APP_DEBUG` is off.)

## 1. Prepare a host (once per VPS)

```bash
sudo apt-get update
sudo apt-get install -y caddy sqlite3 curl
# Litestream 0.3.x (releases: https://github.com/benbjohnson/litestream/releases)
LS=0.3.13
curl -fsSL -o /tmp/litestream.deb \
  https://github.com/benbjohnson/litestream/releases/download/v$LS/litestream-v$LS-linux-amd64.deb
sudo dpkg -i /tmp/litestream.deb && sudo systemctl disable --now litestream   # we use litestream@<client>
sudo install -d /opt/white-label/releases /opt/white-label/clients /etc/white-label /var/lib/white-label
sudo cp deploy/systemd/white-label@.service deploy/systemd/white-label@.socket \
        deploy/systemd/litestream@.service /etc/systemd/system/
sudo systemctl daemon-reload
```

`sqlite3` is used for snapshots and checks; the Debian package is enough. The `.deb` installs
`/usr/bin/litestream`; the units call `/usr/local/bin/litestream`: `sudo ln -s /usr/bin/litestream /usr/local/bin/litestream`.
*Not yet tested* (no apt, Caddy or Litestream in the sandbox).

## 2. Get the binary

From CI (`.github/workflows/release.yml`, on a tag): download the `host-<tag>-linux-x86_64`
artifact. It is built in the Dockerfile's `build` stage on Debian 12, so it runs on Debian 12+
and Ubuntu 22.04+. Or build it yourself from the repository root:

```bash
docker build -f deploy/Dockerfile --target binary --output type=local,dest=dist .   # dist/host
# or, without Docker, on a Debian 12 / Ubuntu 22.04 build machine:
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --release -p host --locked   # target/release/host
```

*Not yet tested:* no release build and no Docker build has been run (the sandbox had less than
15 GB of free disk and no Docker daemon). The runbook below was exercised with the debug
binary, which behaves the same for everything it covers.

Install it as a release:

```bash
TAG=v0.1.0
sudo install -D -m 0755 dist/host /opt/white-label/releases/$TAG/host
```

## 3. First install of a client

Example: client `demo` (the demo company of `clients/demo-properties`), domain
`demo.palmgrove.example`, port 3100. Replace the three values for a real client.

```bash
C=demo; DOMAIN=demo.palmgrove.example; PORT=3100; TAG=v0.1.0
```

### 3.1 User, directories, binary

```bash
sudo useradd --system --home-dir /var/lib/white-label/$C --shell /usr/sbin/nologin wl-$C
sudo install -d -o wl-$C -g wl-$C -m 0750 /var/lib/white-label/$C /var/lib/white-label/$C/snapshots
sudo install -d -m 0700 /etc/white-label/$C
sudo install -d /opt/white-label/clients/$C
sudo ln -sfn /opt/white-label/releases/$TAG/host /opt/white-label/clients/$C/host
```

### 3.2 client.toml

Start from the demo client and edit `[client]` (name, domain, locales), `[branding]`,
`[modules] enabled` and the `[[legal_entities]]` (trade licence, TRN, address):

```bash
sudo install -m 0600 clients/demo-properties/client.toml /etc/white-label/$C/client.toml
sudoedit /etc/white-label/$C/client.toml
```

The host refuses a module that is not built yet (error `[modules] ... is not built yet`).
The `enabled` list decides which `<module>:install` commands exist (3.6).

### 3.3 env

```bash
KEY="base64:$(head -c 32 /dev/urandom | base64)"
sed -e "s/CLIENT\.example\.ae/$DOMAIN/g" -e "s#/CLIENT\b#/$C#g" \
    -e "s#base64:CHANGE-ME#$KEY#" -e "s#^APP_PORT=.*#APP_PORT=$PORT#" \
    deploy/env.production.example | sudo install -m 0600 /dev/stdin /etc/white-label/$C/env
sudoedit /etc/white-label/$C/env      # MAIL_*, MAIL_FROM_NAME, TRUSTED_HOSTS, anything client specific
```

**Store `APP_KEY` with the client's backups** (password manager or the control plane's secret
store), not only on the VPS: a restore on a new host needs the same key.

If the client's SMTP is not ready, set `MAIL_MAILER=log` for the first install only; mails,
including the verification link of 3.7, are then written to the journal. Switch to `smtp`
before go-live (see section 8, mail).

### 3.4 Socket port and Litestream

```bash
sudo install -d /etc/systemd/system/white-label@$C.socket.d
printf '[Socket]\nListenStream=\nListenStream=127.0.0.1:%s\n' "$PORT" \
  | sudo tee /etc/systemd/system/white-label@$C.socket.d/listen.conf >/dev/null
sed "s/CLIENT/$C/g" deploy/litestream/litestream.yml | sudo install -m 0600 /dev/stdin /etc/white-label/$C/litestream.yml
sudo install -m 0600 deploy/litestream/litestream.env.example /etc/white-label/$C/litestream.env
sudoedit /etc/white-label/$C/litestream.yml     # endpoint and bucket of the backup VPS
sudoedit /etc/white-label/$C/litestream.env     # the client's MinIO key
sudo systemctl daemon-reload
```

On the backup VPS (once per client), create the client's MinIO user and a policy that allows
only the prefix `white-label-backups/$C/*`. *Not yet tested.*

### 3.5 Migrate and install defaults

Run the commands as the client's user, with its environment. The helper `wl-run` loads
`/etc/white-label/$C/env` the way systemd does:

```bash
wl-run() { sudo systemd-run --quiet --wait --collect --pipe -p User=wl-$C -p WorkingDirectory=/var/lib/white-label/$C \
             -p EnvironmentFile=/etc/white-label/$C/env /opt/white-label/clients/$C/host "$@"; }
wl-run migrate
wl-run db:check
wl-run wl:install            # legal entities from client.toml, approval policies, compliance rules
wl-run property:install      # the vertical's ticket categories, checklists, SOPs
for m in documents notify invoicing ticketing sales hr; do wl-run $m:install; done   # only the modules in client.toml
```

`wl:install` and the `<module>:install` commands are idempotent ("where the database lacks
them"), so running them again is safe. Without `wl:install` there are no legal entities and
no approval policies: the app boots, but invoicing and approvals have nothing to work with.
*Tested here*: all of these ran against a production-mode database (2 legal entities, 16
approval policies, 6 compliance rules, 18 property defaults, 25 document kinds, 101
notification types, 11 budget categories, 9 ticketing, 9 sales and 1 HR defaults; a second run
created 0). `systemd-run` itself is *not yet tested*; the sandbox ran the same commands with
the same variables from a shell.

> **Never run `db:seed` or `migrate:fresh` on a production database.** `db:seed` creates
> `admin@example.com` and `member@example.com` with the password `password123`;
> `migrate:fresh` drops every table. Both run in production without any question (*tested
> here*, on a scratch database).

### 3.6 Start

```bash
sudo systemctl enable --now white-label@$C.socket litestream@$C.service
curl -fsS http://127.0.0.1:$PORT/health        # starts the app through the socket; expect "status":"ok"
sudo systemctl status white-label@$C.service
```

The first request starts `white-label@$C.service`. Its `ExecStartPre` lines restore the
database from the replica if `app.db` is missing and a replica exists (none on a new client),
then run `migrate`. Do not start `litestream@` before the first `migrate` has created
`app.db`; if you did, restart it.

*Tested here*: the app started with the socket handed over by `systemd-socket-activate` (log
line `using the socket systemd passed (socket activation)`), and stopped cleanly on SIGTERM.
The unit files pass `systemd-analyze verify` (only the missing binaries are reported). They
have not been started by systemd itself.

### 3.7 Caddy

```bash
sed -e "s/demo\.palmgrove\.example/$DOMAIN/g" -e "s/3100/$PORT/g" deploy/Caddyfile.example \
  | sudo tee -a /etc/caddy/Caddyfile >/dev/null      # or one file per client, imported
sudo systemctl reload caddy
```

The DNS name must point at the VPS and ports 80/443 must be open, or Caddy cannot get the
certificate. *Not yet tested* (no Caddy in the sandbox; the proxy headers were simulated with
curl: `Host`, `X-Forwarded-Proto: https`, `X-Forwarded-For`).

### 3.8 The first admin

Registration is open (`/register`), and there is no other way to create a user. So the first
admin signs up, verifies the e-mail address and is then promoted:

1. Open `https://$DOMAIN/register` and sign up with the admin's e-mail address.
2. The verification mail is sent at once. With SMTP it arrives in the inbox. With
   `MAIL_MAILER=log`, take the link from the journal:
   `journalctl -u white-label@$C -n 50 | grep -o 'https://[^ ]*verify-email[^ ]*'`.
   Open it **in the browser where you signed up** (the link needs the logged-in session).
3. Promote the user:
   ```bash
   wl-run users:admin admin@client.example     # prints "<email> is an admin."
   ```
   An address that has not signed up fails with `no user with the email ...`.
4. Reload the page: the sidebar now shows the admin pages (users, activity, legal entities, property).

An unverified user is redirected to `/verify-email` (dashboard) and gets 403 on `/property`:
*tested here*, together with the whole sequence above (with the log mailer; the link in the log
is built from `APP_URL`, so it carries the public domain).

Anyone can also register as a plain `member` while `/register` is open. **Open item:** the
host does not call `without_registration()` yet (host code, not part of the deployment files).
Until it does, check what a member can see before go-live.

### 3.9 Check the install

```bash
curl -fsS https://$DOMAIN/health                          # {"database":"ok","maintenance":false,"queue":{"failed":0,"pending":0},"status":"ok"}
curl -s -o /dev/null -w '%{http_code}\n' https://$DOMAIN/login      # 200
curl -sI https://$DOMAIN/login | grep -i 'strict-transport\|set-cookie'   # HSTS; session cookie has Secure
sudo systemctl is-active white-label@$C litestream@$C
sudo -u wl-$C litestream snapshots -config /etc/white-label/$C/litestream.yml /var/lib/white-label/$C/app.db   # a snapshot exists after a minute
```

The first three are the same answers the sandbox gave (*tested here*, via simulated proxy
headers). The last two need systemd and a MinIO (*not yet tested*).

## 4. The health endpoint

Renox exposes `GET /health`: **200** `{"status":"ok", ...}` while the database answers within
2 seconds, **503** otherwise, with the queue's `pending` and `failed` counts. It answers for
any `Host` (TRUSTED_HOSTS does not apply), so `curl http://127.0.0.1:$PORT/health` works on
the VPS. There is no `/up`. Use `/health` for Caddy's `health_uri`, the host agent and
`wl-operator`; alert when `queue.failed` grows. `GET /login` returning 200 is a check of the
page rendering, not of the database. (*Tested here*: 200 with and without a trusted host.)

## 5. Release and rollout (OPERATIONS.md section 4)

Before the first ring, in CI or on a build machine:

1. All tests green; `cargo build --release -p host --locked` (CI: `release.yml`).
2. `host db:check` and `host view:check` against any database (*tested here*: "168 models
   match the schema", "Checked 200 templates: 0 problems").
3. Migration dry run against a copy of every schema version in use: restore a recent replica
   of a client (section 7, drill) and run the new binary's `migrate` on that copy.

Then, per client, in the order of the rings (canary and demo clients first, wait 24 h; early
clients, wait 48 h; the rest outside business hours; two operators for the last ring):

```bash
C=acme; PORT=3101; TAG=v0.2.0
sudo install -D -m 0755 dist/host /opt/white-label/releases/$TAG/host        # once per host
PREV=$(readlink /opt/white-label/clients/$C/host)                            # for the rollback
# 1. snapshot (consistent, safe while the app runs; never `cp` the .db file)
SNAP=/var/lib/white-label/$C/snapshots/pre-$TAG.db
sudo -u wl-$C sqlite3 /var/lib/white-label/$C/app.db ".backup '$SNAP'"
sudo -u wl-$C sqlite3 "$SNAP" 'pragma integrity_check;'                      # prints: ok
# 2. switch the binary and restart. Connections wait on the socket; the unit
#    runs `migrate` before the new binary takes the socket over.
sudo ln -sfn /opt/white-label/releases/$TAG/host /opt/white-label/clients/$C/host
sudo systemctl restart white-label@$C.service
# 3. health check (wait for the restart, up to 60 s)
for i in $(seq 30); do curl -fsS http://127.0.0.1:$PORT/health && break; sleep 2; done
wl-run migrate:status | tail -3                                              # the new migrations show "ran"
journalctl -u white-label@$C -n 30 --no-pager
```

If the health check never passes: roll back (section 6). If it passes, run the smoke test
(sign in, open the dashboard, open `/property`) and move on to the next client.

Migrations are forward-only and must work with the old binary too, for one release
(OPERATIONS.md, "Migrations"): that is what makes the rollback below possible. The unit has
`TimeoutStartSec=120`; a longer migration belongs in a job (`wl-run migrate` by hand before
the switch is the alternative).

*Not yet tested:* the sequence as a whole (systemd, sqlite3 and the symlink switch). Automatic
rollback on a failed health check (OPERATIONS section 4 step 4) is a **target**: it is not
scripted. `wl-operator` and the `deploy/rollout` tool of ARCHITECTURE section 8 do not exist yet.

## 6. Rollback

```bash
# a) The new binary is bad, the migrations are compatible with the old one:
sudo ln -sfn "$PREV" /opt/white-label/clients/$C/host        # PREV from the rollout step
sudo systemctl restart white-label@$C.service
# b) The migrations are not compatible, or data was damaged: previous binary + snapshot.
sudo systemctl stop white-label@$C.service litestream@$C.service
sudo -u wl-$C mv /var/lib/white-label/$C/app.db /var/lib/white-label/$C/app.db.bad
sudo rm -f /var/lib/white-label/$C/app.db-wal /var/lib/white-label/$C/app.db-shm
sudo -u wl-$C cp "$SNAP" /var/lib/white-label/$C/app.db
sudo ln -sfn "$PREV" /opt/white-label/clients/$C/host
sudo systemctl start litestream@$C.service white-label@$C.service
```

In (b) everything written after the snapshot is lost; the `.bad` file and the replica still
have it, so the decision to go back is a person's. Litestream starts a new generation on its
own after the database was replaced. `migrate:rollback` is a last resort: it only works when
every migration of the last batch has a `.down.sql`.

## 7. Backups and restore drills (OPERATIONS.md section 5)

What is covered:

| Item | How | State |
|---|---|---|
| Database | `litestream@<client>`, sync every 10 s | configured, **not yet run** against a MinIO |
| Nightly snapshot (30 days; monthly 12 months) | Litestream snapshots (`snapshot-interval: 24h`, `retention: 720h`) | configured, not yet run; monthly copies are **not** scripted |
| Uploads (`/var/lib/white-label/<client>/app/`) | to the backup VPS within 5 minutes | **not built** (suggest `mc mirror` or `rclone` on a systemd timer) |
| `APP_KEY`, `env`, `client.toml` | the control plane's secret store | **not built**: copy them by hand when the client is created |

Check that the replica is current (any day):

```bash
sudo -u wl-$C litestream generations -config /etc/white-label/$C/litestream.yml /var/lib/white-label/$C/app.db
sudo -u wl-$C litestream snapshots   -config /etc/white-label/$C/litestream.yml /var/lib/white-label/$C/app.db
journalctl -u litestream@$C -n 20 --no-pager
```

**Restore drill** (monthly, on a scratch host or a scratch directory; never over the live
file):

```bash
D=$(mktemp -d); sudo chown wl-$C $D
# 1. restore the latest replica (add -timestamp 2026-10-20T10:00:00Z for a point in time)
sudo -u wl-$C litestream restore -config /etc/white-label/$C/litestream.yml -o $D/app.db /var/lib/white-label/$C/app.db
# 2. integrity and content
sudo -u wl-$C sqlite3 $D/app.db 'pragma integrity_check;'
sudo -u wl-$C sqlite3 $D/app.db 'select count(*) from users; select count(*) from wl_legal_entities;'
sudo -u wl-$C sqlite3 $D/app.db 'select max(created_at) from audit_log;'     # last transaction time (check the table name)
# 3. smoke test: run the binary on the copy, another port, same env but this database
sudo systemd-run --quiet --wait --collect --pipe -p User=wl-$C -p WorkingDirectory=$D \
  -p EnvironmentFile=/etc/white-label/$C/env \
  -E DATABASE_URL="sqlite://$D/app.db?mode=rwc" -E STORAGE_PATH=$D -E APP_PORT=3199 -E SCHEDULER=false -E QUEUE_WORKERS=0 \
  /opt/white-label/clients/$C/host serve &
sleep 5; curl -fsS http://127.0.0.1:3199/health; kill %1
```

Use `SCHEDULER=false` and `QUEUE_WORKERS=0` on a drill copy: the restored database holds real
queued jobs and scheduled rules (mail, chases) that must not run twice. Record the result
(date, client, row counts, time it took) in `wl-operator` once it exists.

**Restore a client on a new host** (RTO 1 h): do sections 1 to 3.5 for the same client name
(use the saved `env` with the same `APP_KEY` and the saved `client.toml`), but do **not** run
`wl:install` or create a user; start the socket. `white-label@.service` restores `app.db`
from the replica by itself (`ExecStartPre` with `-if-db-not-exists -if-replica-exists`). If the
MinIO is unreachable the start fails, on purpose: an empty database must never be created and
replicated over the good copy. Then restore the uploads from the backup storage and check
`/health`. Change DNS last.

*Tested here:* only that the app reads a database file taken from another location and that
the files `-wal` and `-shm` matter (a plain `cp` of a live database lost the latest committed
changes in the sandbox: a user's verified e-mail and admin role). **No Litestream
replication, restore or drill has been run.**

## 8. Production-only failures met while writing this

These all came from running the real binary with `APP_ENV=production` (*tested here*).

| Symptom | Cause | Fix |
|---|---|---|
| `Error: APP_KEY must be set in production` | no `APP_KEY` | set it (3.3) |
| `client.toml: CLIENT_CONFIG is not set: name the client's client.toml` | production does not fall back to the demo client | set `CLIENT_CONFIG` |
| `client.toml: cannot read ...` | wrong path or wrong owner | path in `env`; the file is read by the process, not by systemd: give `wl-<client>` read access to `client.toml` (the directory is 0700 root in 3.1; use `chgrp wl-<client>` and `chmod 0750/0640`, or `ReadOnlyPaths=`) |
| Sign-in redirects back to the login page, no error | `APP_URL` is `https://` so the session cookie is `Secure`, and you browse over `http://` (e.g. an SSH tunnel to the port) | browse through Caddy over https |
| `400 Bad Request`, plain text, on every page except `/health` | `TRUSTED_HOSTS` is set and the `Host` header is not in it (`curl localhost:3100` has `Host: localhost`) | send the right Host: `curl -H 'Host: demo.palmgrove.example' ...` |
| Rate limits and login lock count everyone as one visitor; logs show 127.0.0.1 | `TRUSTED_PROXIES` unset behind Caddy | `TRUSTED_PROXIES=127.0.0.1`; never set it when the app port is reachable from outside |
| Sign-up answers `500`, log: `sending mail to ...: failed to lookup address information` | `MAIL_MAILER=smtp` and the host or credentials are wrong; mail is sent inside the request. **The user row is already created** (the next attempt says the address is taken) | fix `MAIL_*`; delete or verify the stray user; test SMTP on the first install with the log mailer then switch |
| Verification link in the journal says `http://127.0.0.1:3100/...` | `APP_URL` left at the default | set `APP_URL=https://<domain>` before the first sign-up |
| Verification link answers 403 | signed for another host/URL than the one you open, or expired (60 minutes), or opened in a browser that is not logged in as that user | request a new one (the `/verify-email` page has a button); open it in the same browser |
| `Error: APP_TIMEZONE: unknown time zone ...` | typo in the zone | `Asia/Dubai` |
| New client has no legal entities or approval policies | `wl:install` not run | 3.5 |
| `admin@example.com / password123` exists | someone ran `db:seed` or `migrate:fresh --seed` | remove the users, never do it again (3.5) |

CSP: `CSP=relaxed` (default) is what the pages are known to work under. `CSP=strict` has
**not** been tried against the host's pages (Alpine inline expressions and inline scripts
would break); do not enable it without a page-by-page check.

## 9. Logs and everyday commands

Logs go to stdout, so systemd keeps them in the journal. Every line of a request has its id,
and the id is in the `X-Request-Id` header of the answer (Caddy passes it through).

```bash
journalctl -u white-label@$C -f                          # follow
journalctl -u white-label@$C --since '1 hour ago' -p warning --no-pager
journalctl -u white-label@$C | grep 'id=<request id>'    # one request, from a user's report
journalctl -u white-label@$C -o cat | jq 'select(.level=="ERROR")'   # with LOG_FORMAT=json
journalctl -u litestream@$C -n 50 --no-pager             # replication
sudo tail -f /var/log/caddy/$DOMAIN.log                  # access log (JSON)
```

Raise the level temporarily with `RUST_LOG=debug,sqlx=warn` in `env` and a restart. Other
commands (all through `wl-run`): `queue:failed`, `queue:retry all`, `schedule:list`,
`migrate:status`, `db:shell`, `down` / `up` (maintenance mode; `/health` stays 200).

## 10. Container image (optional)

```bash
docker build -f deploy/Dockerfile -t white-label-host .
docker run -d --name demo -p 127.0.0.1:3100:3000 \
  -e APP_KEY=base64:... -e APP_URL=https://demo.palmgrove.example \
  -e TRUSTED_HOSTS=demo.palmgrove.example \
  -v demo-storage:/app/storage -v $PWD/clients/demo-properties/client.toml:/etc/white-label/client.toml:ro \
  white-label-host
```

The image runs `host migrate`, then `host`, as a non-root user with a `HEALTHCHECK` on
`/health`. The VPS install does not use it. *Not yet tested*: no Docker daemon was available.
Litestream is not in the image; to replicate a container's volume, run Litestream beside it
on the host with the volume's path.
