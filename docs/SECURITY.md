# Security

Photolake holds personal photos, often with locations, so it is locked down by default.

## Threat model

| Threat | Protection |
|---|---|
| **Someone else on your Wi-Fi or tailnet** opens Photolake | Password sign-in (Argon2-hashed). Sessions are random 256-bit tokens in `HttpOnly; SameSite=Strict` cookies, and only their SHA-256 is stored. Failed sign-ins are rate-limited. |
| **A website you visit** tries to use Photolake through your browser: CSRF, or DNS rebinding to `localhost` | Requests with an unknown `Host` name are refused, which defeats DNS rebinding. Cross-site writes are refused (`Origin` / `Sec-Fetch-Site` checks). Cookies are `SameSite=Strict`. |
| **Malicious text in your own data**, for example a species name or note containing HTML | The UI never inserts data as HTML. A strict Content-Security-Policy blocks inline scripts. `X-Frame-Options: DENY` and `nosniff` are set. |
| **The API being used to read or write arbitrary files** | Imports are only allowed from your home folder, removable drives, and the container's `/import`. Backup folders must be on a drive or in your home folder. rclone remotes are validated, so a remote can't smuggle in command-line flags. |
| **Heavy SQL queries** | SQL is admin-only and read-only (no DDL, DML or `SET`), stops after 30 s, and returns at most 5,000 rows. |
| **Read-only users changing things** | Viewer sessions can only make GET requests, and can't open settings, drives or SQL. The server enforces this; it isn't just hidden in the UI. |
| **Storage exposed on the network** | RustFS has no published ports in the container setup; only the Photolake container can reach it. Its admin console is disabled. Its keys are random per install and stored in `.env` with mode 600. |
| **Container escape or privilege misuse** | Photolake runs as your user, not root, with a read-only filesystem, `cap_drop: ALL` and `no-new-privileges`. |
| **Secrets readable by other local users** | `config.toml`, `.env` and `sessions.json` are mode 600. Data folders are mode 700. |
| **Vulnerable dependencies** | CI runs `cargo audit`, `npm audit`, and weekly Dependabot updates. Accepted exceptions are documented in `.cargo/audit.toml`. |

## Things to know

- **Home Wi-Fi mode is plain HTTP.** Anyone able to sniff your Wi-Fi could read the traffic. Use Tailscale for
  encrypted access, or keep Wi-Fi mode for trusted home networks. See [REMOTE.md](REMOTE.md).
- **Backups are not encrypted** by Photolake.
  - External drive: use a drive encrypted with LUKS. Ubuntu's Disks app can create one ("Password protect volume").
  - Cloud backups: use an rclone `crypt` remote, so the provider only sees encrypted data:
    ```bash
    rclone config      # create e.g. "b2" for Backblaze, then a "crypt" remote "b2crypt" wrapping b2:bucket/photolake
    ```
    Then add the backup target `b2crypt:` in Settings.
- **eBird lookups** send a rounded location (about 25 km) to the eBird API when a photo has GPS.
  Leave the eBird key empty to keep everything offline.
- **Map tiles** load from OpenFreeMap, and place search uses OpenStreetMap Nominatim. Both see your IP address and
  the map area you look at.

## Audit log (October 2026, v0.2.0)

| Finding | Severity | Status |
|---|---|---|
| API had no authentication; DNS rebinding and CSRF were possible against `127.0.0.1:7878` | High | Fixed: sign-in, Host allowlist, Origin checks |
| Stored XSS: species names were inserted into the map popup with `innerHTML` | High | Fixed: DOM text nodes plus CSP |
| `POST /api/imports` accepted any server path; backup targets accepted any path or rclone flags | Medium | Fixed: allowlisted roots, validation |
| Default S3 credentials, config file readable by other users, RustFS console on | Medium | Fixed: random keys, 600/700 permissions, console off, no published ports |
| SQL endpoint had no time limit | Low | Fixed: 30 s timeout, admin-only |
| `quick-xml` 0.37 (RUSTSEC-2026-0194/0195) parsed uploaded GPX files | Medium | Fixed: upgraded to 0.42 |
| `quick-xml` 0.39 inside opendal; `rsa` in the lockfile | Low | Accepted. opendal only parses replies from our own storage, and `rsa` is never compiled. See `.cargo/audit.toml`. |

## Reporting

Please open a private security advisory on the GitHub repository rather than a public issue.
