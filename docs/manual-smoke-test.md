# Linux release smoke test

Download the `signal-forge-linux-x86_64` artifact from a passing Actions run,
unzip the artifact wrapper, verify `sha256sum -c *.sha256`, and unpack the tarball.
Run `./install.sh` (default `~/.local`) or launch `bin/signal-forge` directly.
The package targets x86_64 Ubuntu 24.04 or a compatible newer glibc system;
it is dynamically linked, not a universal static binary. See the included
`linked-libraries.txt`. A graphical X11 or Wayland session with OpenGL is required.

1. Launch from the applications menu. Refresh devices. Open two serial ports or
   create a named PTY pair and open both paths using the app controls.
2. Open **Settings** and adjust each disconnected terminal's baud/data/parity/stop/flow settings,
   reconnect, and verify independent RX/TX using loopback plugs or peer clients.
   With physical adapters, verify nondefault baud/parity and hardware flow control
   on equipment supporting those modes. PTY CI does not validate electrical behavior.
3. Send text, escaped `\r\n`, and hex `00 FF 0D 0A`. Enter must transmit from
   the payload box; Up recalls and cycles history, Down restores the draft.
   Enter elsewhere must not transmit. Invalid hex/escapes must report an error.
4. Select the **Repeat** tool and start a finite repeat, verify its count, start continuous repeat, then stop it.
   Disconnect and close the tab during a repeat; transmission must stop.
5. Use **Presets… / Manage…** to create/select/edit a preset profile, export/import it, and verify selected and
   fixed targets. Restart alone must never transmit any preset.
6. Drag tabs to split, stack, and float windows. Toggle hex/timestamps/autoscroll.
   Save workspace (Ctrl+S), quit, and restart. Layout, settings, display options,
   and profile must return, with every terminal disconnected. Reconnect explicitly.
7. Bridge two connected endpoints. Exchange different binary streams in both
   directions. Pause/filter the inspector while capture continues. Stop capture;
   verify the JSONL footer and exact bytes. Stop the bridge; endpoints stay open.
8. Unplug a physical adapter or close a PTY peer. The affected endpoint must show
   its fault without crashing other terminals. Reconnect or close it.
9. Quit (Ctrl+Q) while pairs/bridges/repeats/capture exist. Owned PTYs must vanish,
   serial handles must close, and capture must finish or explicitly report a fault.
10. Back up workspace.json, replace it with invalid JSON, and launch. Verify a
    visible error and disabled saving. Use backup/reset recovery; confirm the
    original bytes exist in the reported recovery file and the app remains usable.

Automated CI covers the byte-flow, concurrency, keyboard, capture, persistence,
and shutdown paths without physical hardware. The physical adapter/electrical
checks above require a local bench and are not claimed as CI evidence.

11. Use **4 Tiles** with four live endpoints at 1080p, 1440p, and 900×600. Send from each pane, switch Send/Repeat/Presets, and expand Settings/Display. Confirm controls remain reachable. Retile to **2 Tiles**; connections must stay open.
12. Open each setup dialog and cancel it. No port, pair, bridge, or workspace changes should occur. Test invalid port/custom baud, duplicate pair links, identical bridge endpoints, and an invalid workspace path; errors should stay in setup.

13. Install the package into `~/.local`. Verify Signal Forge appears with its waveform icon in the KDE/GNOME launcher, menu search, task switcher and dock; the native window should show the same icon where supported. Update to a new release and verify there is one launcher and the new assets are installed. Test `share/signal-forge/uninstall.sh`; launcher/icons should disappear while workspace, presets and captures remain.
