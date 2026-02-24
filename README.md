<div align="center">

<h2>MX Bikes Mod Manager</h2> 

A fast desktop mod manager for **MX Bikes**, built with Rust + egui.

Drag and drop `.zip`, `.pkz`, or `.pnt` files, choose where they belong, and install/uninstall mods from one place.

</div>

---

## Features

- Drag-and-drop install flow for `.zip`, `.pkz`, and `.pnt`
- No database: installed mods are read directly from your filesystem
- Supports key MX Bikes `Documents/.../mods` locations
- Per-category installed-mod lists with uninstall actions
- Auto-refresh via filesystem watcher (with manual Refresh fallback)

---

## Download

Download prebuilt binaries from GitHub Releases:  
[Latest Release](https://github.com/shohamc1/mxbmm/releases/latest)

Pick the asset for your OS:

- `mxbmm-windows-x86_64.zip`
- `mxbmm-macos-x86_64.zip`
- `mxbmm-linux-x86_64.tar.gz`

---

## First Launch

On startup, MXBMM auto-detects your mods root:

- Default: `Documents/PiBoSo/MX Bikes/mods`
- Override with env var: `MXBMM_MODS_ROOT`

Examples:

```bash
# Linux
MXBMM_MODS_ROOT="/custom/path/to/mods" ./mxbmm
```

```bash
# macOS (launch bundle executable directly, optional)
MXBMM_MODS_ROOT="/custom/path/to/mods" ./MXBMM.app/Contents/MacOS/MXBMM
```

```powershell
# Windows PowerShell
$env:MXBMM_MODS_ROOT = "D:\Games\MXB\mods"
.\mxbmm.exe
```

---

## How To Use

1. Launch MXBMM.
2. Drag one file (`.zip`, `.pkz`, or `.pnt`) into the app window.
3. In **Pending Install**:
   - Pick **Install location**
   - Set **Install name**
   - Optionally add **Version** and **Notes**
4. Click **Install**.
5. Open **Installed Mods** dropdowns to view installed items.
6. Click **Uninstall** next to a mod to remove it.

---

## Troubleshooting

- **No mods shown**
  - Verify **Mods root path** points to the correct `.../MX Bikes/mods` folder.
  - Click **Refresh**.

- **Watcher unavailable**
  - Your OS may block file watcher setup in some directories; manual **Refresh** still works.

- **Install fails with "Destination already exists"**
  - Choose a different install name or remove the existing destination first.
