# Installing Photolake

This guide assumes no technical background. It takes about 15 minutes, most of it waiting
for downloads.

## What you need

- A Linux computer. Ubuntu 22.04 or newer, Linux Mint, Pop!_OS, Debian 12+ or Fedora 39+ all work.
- About **10 GB** of free space for the app, plus room for your photos.
- An internet connection during setup.
- *Optional:* an **NVIDIA graphics card**. It makes bird identification about 10× faster.
- *Optional:* an **external hard drive** for backups.

Everything else gets installed for you.

## What gets installed, and why

| Piece | What it is | Where it comes from |
|---|---|---|
| **Docker** | Runs Photolake in sealed "containers", so it can't clutter or break the rest of your system and updates are clean. | Docker's official script (`get.docker.com`) |
| **NVIDIA Container Toolkit** (GPU only) | Lets those containers use your graphics card. | NVIDIA's official package repository |
| **Photolake** | The app: web page, card importer, bird identification and backups. | Prebuilt images from GitHub (`ghcr.io`) |
| **RustFS** | The storage engine that holds your photo files and the Iceberg tables. | Docker Hub (`rustfs/rustfs`) |
| **Bird models** (~1.4 GB) | The bird detector (RT-DETR) and species identifier (BioCLIP 2), ready to run. | GitHub (`ghcr.io`), downloaded once |

You never need to install Rust, Node or Python. Those are only for people changing the code.

## Step 1: Open a terminal

Press **Ctrl + Alt + T**, or search your app menu for "Terminal".

## Step 2: Download and run the installer

Copy these two lines into the terminal and press Enter:

```bash
curl -fsSL https://github.com/OWNER/photolake/releases/latest/download/install.sh -o install.sh
bash install.sh
```

The installer explains each step as it goes. Here is what it will ask:

1. **"Install Docker now?"** Only appears if you don't have Docker. Press Enter for yes, then type your
   computer password. Your password doesn't show as you type; that's normal.
   - Afterwards it asks you to **log out and back in** (or restart). Do that, open a terminal again,
     and run `bash install.sh` a second time. It picks up where it left off.
2. **"Install nvidia-container-toolkit now?"** Only appears with an NVIDIA card. Say yes for faster
   bird identification.
3. **A password for Photolake.** Choose one with at least 8 characters. You'll use it to sign in, and it's
   what protects your photos if you turn on phone access later.

When it finishes, your browser opens Photolake at **http://localhost:7878**. Sign in with that password.
Photolake also appears in your app menu.

## Step 3: First things to set up (in the web page)

Open **Settings & backup**:

- **Backups.** Plug in your external drive, click **Add target**, and pick the drive. From then on,
  Photolake backs up after every import, every night, and whenever you plug that drive in.
- **Bird identification** (optional):
  - Paste a free **eBird API key** from https://ebird.org/api/keygen.
  - Set your **home region**, for example `US-CO`.
  - With both set, species are ranked by what's actually seen near you, the way Merlin does.

## Using it

- **Import:** insert your camera's SD card or plug in the camera. Photos import automatically,
  you get a notification, and birds are identified.
- **Photos already on your computer:** copy them into `~/Photolake/import`, then click
  **Imports → Import folder** and choose `/import`.
- **Browse, edit and explore:** use Library, Map and Analytics. Click any photo to edit details,
  correct a species, add notes or set a location.

## Turning it off and on

| You want to… | Run in a terminal |
|---|---|
| Turn it off | `photolake-ctl stop` |
| Turn it back on | `photolake-ctl start` |
| Is it running? What's the address? | `photolake-ctl status` |
| Stop it from starting when the computer boots | `photolake-ctl autostart off` |
| See every command | `photolake-ctl help` |

Once stopped, it stays off, even after a restart, until you run `photolake-ctl start`. Stopping is always safe:
a half-finished import picks up where it left off next time.

## Where your things are

```
~/Photolake/
  storage/   your photo files and the Iceberg tables (this is the big one)
  data/      the catalog (catalog.db), bird models, caches
  config/    config.toml (settings and the password hash)
  import/    drop folders here to import them
  .env       container settings and storage keys (private)
```

Back up `~/Photolake` by adding a backup target in Settings. That's the supported way: it copies
everything consistently while Photolake runs.

## Troubleshooting

| Problem | Fix |
|---|---|
| `permission denied ... docker.sock` | You haven't logged out and back in since Docker was installed. Do that. |
| The page doesn't load | Run `photolake-ctl status`. If it's stopped, run `photolake-ctl start`. If it won't start, run `photolake-ctl logs`. |
| Settings says bird ML runs on "CPU" even though you have an NVIDIA card | Run `bash install.sh` again and accept the NVIDIA toolkit. Check `nvidia-smi` works. |
| A card isn't imported | Check that **Import automatically** is on in Settings, and that the card shows up in your file manager. |
| Forgot the password | Run `photolake-ctl passwd`. |

## Uninstalling

Run `photolake-ctl uninstall`. This removes the app but **keeps your photos** in `~/Photolake`.
Running the installer again later picks them back up.
