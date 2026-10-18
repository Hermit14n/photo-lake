# Using Photolake from your phone or another computer

Out of the box, only the computer Photolake runs on can open it. There are two ways to open it up.
Both require signing in with your Photolake password.

| | Home Wi-Fi | Tailscale (recommended) |
|---|---|---|
| Works at home | ✓ | ✓ |
| Works away from home | ✗ | ✓ |
| Encrypted (HTTPS) | ✗ Your password and photos cross your Wi-Fi unencrypted | ✓ Real certificate, end-to-end encrypted |
| Setup | One command | Free account + an app on each device |
| Exposes anything to the internet | No | No. Only your own signed-in devices can reach it. |

## Option A: Home Wi-Fi

```bash
photolake-ctl wifi on
```

It prints an address like `http://192.168.50.236:7878`. Open that on your phone while on the same
Wi-Fi, and sign in.

Turn it off again with `photolake-ctl wifi off`.

Use this only on a network you trust, such as your own home Wi-Fi with a WPA2/WPA3 password. On shared or public
networks, use Tailscale.

> Tip: give your computer a fixed address in your router's settings ("DHCP reservation"), so the
> address doesn't change.

## Option B: Tailscale

Tailscale builds a private network between your own devices. Photolake runs a small Tailscale
container next to it, which gets its own name (`photolake`) and an HTTPS certificate.

1. Create a free account at https://tailscale.com. Signing in with Google, Microsoft or GitHub is fine.
2. In the Tailscale admin console, under **DNS**, turn on **MagicDNS** and **HTTPS Certificates**:
   https://login.tailscale.com/admin/dns
3. Create an **auth key** at https://login.tailscale.com/admin/settings/keys. Choose "Reusable: off" and
   "Pre-approved: on".
4. On the Photolake computer, run:
   ```bash
   photolake-ctl tailscale on
   ```
   Paste the auth key when asked. The command prints your address, like
   `https://photolake.tail1234.ts.net`.
5. Install the **Tailscale app** on your phone (App Store / Play Store) and sign in with the same account.
6. Open that address in your phone's browser and sign in to Photolake.

Turn it off with `photolake-ctl tailscale off`.

## Read-only access for family and friends

In **Settings → Sign-in & remote access**, choose **Add view-only password**. People who sign in with it can
browse, search, view the map and see analytics. They can't edit, import, change settings or run SQL.

## Signing out everywhere

Changing the password (Settings, or `photolake-ctl passwd`) signs out every device.
