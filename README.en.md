<div align="center">

# NovelTL

[Русский](README.md) · **English**

**Visual novel translator for Windows — translation right on top of the game**

Captures dialogue text from the running game and shows the translation on a semi-transparent box over the dialogue window.
Japanese, English, Chinese, Korean and Russian — from any of them into any other.
No manual hook codes, no command line — three wizard steps and you are playing.

[![Download](https://img.shields.io/github/v/release/Kawaakar1/NovelTL?label=Download&style=for-the-badge&color=8b5cf6)](../../releases/latest)
![Windows 10/11](https://img.shields.io/badge/Windows-10%20%2F%2011-0078D6?style=for-the-badge&logo=windows)
![Linux](https://img.shields.io/badge/Linux-x86__64-FCC624?style=for-the-badge&logo=linux&logoColor=black)
![Rust](https://img.shields.io/badge/Rust-egui-CE422B?style=for-the-badge&logo=rust)
![License GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue?style=for-the-badge)

<img src="shots/08_overlay_in_game.png" width="760" alt="Translation box over the game">

</div>

---

## Features

- 🌍 **Interface in 5 languages.** Русский, English, 日本語, 中文, 한국어 — switch in Settings → General.
- 🎯 **Automatic hook detection.** Click through 3–4 lines — the program finds the dialogue text thread by itself, and a separate thread with character names.
- 🪟 **Overlay on top of the game.** Clicks go through it, it never steals focus, it follows the game window. Size and position are set once with the mouse.
- 🔒 **Lock and menu right on the overlay.** Open the lock — move and resize the box during play; close it — the box is fixed again. The ☰ button opens the same menu as the tray icon.
- 📋 **Clipboard mode.** NovelTL translates everything you copy — for games the text hooker cannot handle, and for Linux.
- 📜 **Dialogue history.** The main window keeps the last 50 lines with original and translation — handy if you skipped a line.
- 🗺️ **5 languages, any direction.** Japanese, English, Chinese (Simplified), Korean and Russian. The target language is chosen in Settings, the source language per game (or “Auto”).
- 🌐 **DeepL and Google.** “Auto” mode: Google without a key, DeepL with a key (the free plan works too) — DeepL uses the previous lines as context, and if DeepL refuses, the program switches to Google by itself. You can also choose “DeepL only” or “Google only”.
- 🆓 **Free alternatives.** **Local AI** via [Ollama](https://ollama.com) or LM Studio — free, offline, aware of the scene context. **Microsoft Translator** — the official free tier (2 million characters per month). If the chosen translator does not answer, the line is translated by Google.
- 📖 **Glossary.** Names and terms are translated the way you set: “灯里 → Akari”. A name can have a note (gender, way of speaking) — DeepL and local AI get it.
- ⚖️ **Second translation.** A smaller line under the main one from another translator, to cross-check the meaning.
- 🧾 **Log and statistics.** Every translated line of a game is saved to a text file (easy to search); the main window shows how much you have read.
- 🧽 **Per-game filters.** Fragments that are removed from the text before translation.
- 💾 **Translation cache.** Repeated lines appear instantly and do not use up translator limits.
- 🧹 **Text cleanup.** Removes KiriKiri engine tags, doubled characters and repeated phrases.
- 🔌 **A profile for each game.** Start the game — NovelTL connects by itself, no wizard needed.
- 🛡️ **Network.** SOCKS5 / HTTP proxy with login and password, choice of network interface — in case Google limits requests through a VPN.
- 🎨 **Themes.** Strict black-and-white dark and light (or “follow Windows”), ready palettes “Violet”, “Midnight”, “Sakura”, “Forest” and **your own theme** — background, panels, text and accent from a color picker.
- 🖌️ **Overlay style.** Font, size, colors and opacity — with a live preview.
- ⌨️ **Hotkeys** work right in the game; tray icon.
- 🪶 **Lightweight.** A single Rust exe: ~6 MB installer, ~90 MB of memory, starts in under a second.

Made first of all for YUZUSOFT games on the KiriKiri engine (Senren\*Banka and others); LunaHook supports many other engines as well.

## Screenshots

<table>
<tr>
<td width="50%"><img src="shots/09_main_running.png" alt="Main window"><br><sub>Main window: the game is connected, original and translation are shown</sub></td>
<td width="50%"><img src="shots/02_wizard_step1.png" alt="Wizard, step 1"><br><sub>Wizard, step 1 — choose the game window and language</sub></td>
</tr>
<tr>
<td><img src="shots/04_wizard_step2_found.png" alt="Wizard, step 2"><br><sub>Step 2 — the program found the dialogue text by itself</sub></td>
<td><img src="shots/07_overlay_edit_mode.png" alt="Wizard, step 3"><br><sub>Step 3 — drag the box onto the dialogue window</sub></td>
</tr>
</table>

The screenshots show the Russian interface; the English one looks the same.

## Installation

1. Download `NovelTL-Setup-….exe` from the [**Releases**](../../releases/latest) page.
2. Run it. No administrator rights needed — the program installs into your user folder.
   The installer is not digitally signed, so Windows may show “Windows protected your PC” → **More info** → **Run anyway**.
3. To update, simply install the new version over the old one: game profiles and settings are kept.

> [!NOTE]
> Windows Defender is sometimes suspicious of programs that connect to other processes (this is how text capture works).
> If the game “does not respond” at step 2, add the NovelTL folder and the game folder to Defender exclusions.

### Linux

Download `NovelTL-…-linux-x86_64.tar.gz` from the [**Releases**](../../releases/latest) page, extract it and run `./noveltl` (`./install.sh` adds it to the application menu).

Differences from Windows:
- **No automatic text capture** — LunaHook exists only for Windows. In the wizard choose “Clipboard (copied text)”: NovelTL translates everything copied to the clipboard. Usually the game runs through Wine/Proton and a text hooker (for example Textractor in the same Wine) copies the lines to the clipboard.
- **The overlay is a separate always-on-top window.** Its place is set with the mouse: open the lock — drag and stretch it, close the lock — clicks go to the game. Works on X11 (and XWayland, which NovelTL selects automatically); Steam Deck — in Desktop Mode.
- No tray icon and no global hotkeys — everything is in the ☰ menu on the overlay. Keys are stored unencrypted.

Details are in `README-Linux.txt` inside the archive. The Linux build is checked automatically, but nobody has tried it with real games on Linux yet — please report in [Issues](../../issues).

## How to use

1. Start the game **in windowed mode** and get to a place with dialogue.
2. Start NovelTL → **“Add game”**. The interface language can be changed in Settings → General.
3. **Step 1.** Choose the game window and the source language. If Windows asks for administrator permission, allow it.
4. **Step 2.** Click through 3–4 lines in the game. When the dialogue text appears in the window, press **“This is the dialogue text”**.
   Wrong text? Open “Other options” and pick the right line.
5. **Step 3.** Drag the box onto the game's dialogue window, stretch it by the edges and press **“Done”**.

Next time just start NovelTL and the game — it connects automatically.

Want to look at the program without a game? Run `NovelTL.exe --demo` (or the “demo mode” shortcut in the Start menu).

### Hotkeys

| Keys | Action |
|---|---|
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>H</kbd> | hide / show the overlay |
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>R</kbd> | translate the line again, bypassing the cache |

The shortcuts can be changed in Settings.

### Languages, translator and network

- **Target language** — Settings → Translation → “Translate into”. The overlay immediately shows the current line in the new language.
- **Source language** is chosen in the wizard when adding a game, and for an existing game — in the main window next to its name. “Auto” detects the language from the text.
- **Translator** — in the same place: “Auto” (DeepL if a key is set, otherwise Google), “DeepL only”, “Google only”, “Microsoft Translator” or “Local AI”.
- **Local AI (free, offline):** install [Ollama](https://ollama.com), download a model (for example `ollama pull qwen2.5:7b`), choose “Local AI” in Settings, press “Refresh list” and pick the model. You need a graphics card with roughly 6–8 GB; the first line takes longer — the model is loaded into memory. LM Studio works too: address `http://localhost:1234`.
- **Microsoft Translator (free tier):** in the Azure portal create a “Translator” resource with the F0 tier, copy the key and region from “Keys and Endpoint” into Settings.
- **Google** works without a key. If you see “Google temporarily limited requests (code 429)” — wait, turn off the VPN, set a proxy or a DeepL key.
- **DeepL**: paste the API key into Settings → Translation. A free key ends with `:fx`. In “Auto” mode, on error 403/456 translation automatically goes through Google.
- **Proxy**: SOCKS5 or HTTP, with or without login and password. The “Check connection to Google” button shows right away whether it works.

## If the text is not captured

1. Without closing the game, click through 5–6 lines.
2. Press **“Report for setup”** in the main window — a folder with a `report_….json` file opens.
3. Create an [Issue](../../issues) and attach this file and a screenshot of the game, and describe what you see at step 2.

The report contains no DeepL key, no proxy login or password and no path to the game — it is safe to publish.

Common causes:
- **“The game did not respond within 10 seconds”** — add NovelTL and the game to Windows Defender exclusions, restart both programs.
- **The game runs as administrator** — run NovelTL the same way.
- **The overlay is not visible** — the game must be in windowed, not fullscreen mode.

## Where data is stored

Everything is next to the program, in the `data` folder:

| Path | Contents |
|---|---|
| `data\profiles` | game profiles: hook, language, overlay position, glossary |
| `data\cache` | translation cache, one file per game |
| `data\reports` | setup reports |
| `data\settings.json` | general settings |

🔒 The DeepL key and the proxy password are stored **encrypted** with Windows means (DPAPI) — they can be read only under your account on this computer. The program sends no data anywhere except the dialogue text to the chosen translator.

## Building from source

Requirements: [rustup](https://rustup.rs) with the `x86_64-pc-windows-gnu` toolchain, MinGW-w64 (for example `winget install BrechtSanders.WinLibs.POSIX.UCRT`), and [Inno Setup 6](https://jrsoftware.org/isinfo.php) for the installer.

```powershell
cd noveltl-rs
powershell -ExecutionPolicy Bypass -File build.ps1            # tests + program + installer
powershell -ExecutionPolicy Bypass -File build.ps1 -NoInstaller
```

The ready folder is `noveltl-rs\dist\NovelTL`, the installer is in `dist_installer\`.
The program structure is described in [`noveltl-rs/ARCHITECTURE.md`](noveltl-rs/ARCHITECTURE.md) (in Russian).

The first NovelTL version was written in Python (PySide6); it has been removed from the repository and remains in the git history (last commit with it: `ae7a790`, version 0.4.3).

## Credits and license

Text capture — [**LunaHook**](https://github.com/HIllya51/LunaHook) from the [LunaTranslator](https://github.com/HIllya51/LunaTranslator) project (by HIllya51), used without changes. It is based on Textractor and MinHook. Their license texts are in [`files/licenses`](files/licenses).

NovelTL is distributed under the **GPL-3.0-or-later** license — see [LICENSE](LICENSE).

NovelTL does not modify game files. Use it with games you own legally.
