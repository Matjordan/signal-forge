# Debug/test sessions

A session groups an engineering run in an ordinary filesystem folder:

```
Pressure-test/
  session.json
  workspace.json
  notes.txt
  rx-1-1.bin
  bridge-1-1.jsonl
  trigger-1-1.jsonl
```

Open **Session…** (or **Ctrl+Shift+E**), select **New session**, enter a name, and choose a new or empty folder. **Choose folder…** opens a folder picker; entering a new folder path creates it. Existing nonempty folders are rejected without replacing their files. Creating a session snapshots the current workspace and finalizes pre-existing recording/capture jobs. Connections remain open.

New RX recording, bridge capture, and triggered capture paths default into the active session, including subsequently opened terminals/bridges. Suggestions use numbered names and writers still refuse to overwrite existing files. You can edit the path or choose another output through the existing file dialogs. Recording modes and capture behavior remain unchanged.

## Notes and artifacts

The session panel contains a multiline notes editor. Changes autosave after one second to UTF-8 `notes.txt`; **Save session**, **Ctrl+S**, close, and normal application shutdown also save notes and the workspace context. Notes are limited to 1 MiB. **Copy notes** copies their current contents, including unsaved edits.

Successfully started RX recordings and both capture types are indexed immediately. Workspace exports are also indexed. **Add artifact…** associates an existing file such as a test report or externally generated log. The index is limited to 4,096 entries, keeps one entry per path, and does not copy, rename, or delete artifacts. A worker failure can leave an unfinished indexed recording: check that recording/capture's status and footer for integrity.

**Open file**, **Open containing folder**, **Open session folder**, and **Copy path** locate the data. Open actions use the Linux desktop's `xdg-open`; if unavailable or unsuccessful, an error appears and copying paths remains available. Missing/moved files stay indexed and are labelled explicitly. They do not prevent reopening the session. Use **Add artifact…** to associate a file at its new location.

## Close, reopen, and portability

**Close session** finalizes active RX recordings and captures, saves notes/context, and records the close time. It leaves endpoints and bridges connected, but ongoing capture/recording must be started again explicitly. Closing never deletes session data. Normal application shutdown also finalizes the session; save failures prevent a normal window close so unsaved edits remain available.

Select **Open session** and choose a folder containing `session.json`. All required files are validated before replacing the workspace; invalid/future formats and malformed configuration fail with the current workspace intact. Opening ends current recordings, disconnects current endpoints, removes session-owned virtual pairs/bridges, and restores saved terminal layout and settings disconnected. Explicitly reconnect; playback, repeats, and bridges never resume automatically. Sessions themselves are opened explicitly after restarting Signal Forge.

`session.json` is version-1 readable JSON with the session name, creation/open/close Unix nanosecond timestamps as decimal strings, `workspace.json`/`notes.txt` references, and each artifact's kind, added time, and path. Endpoint configuration is in the referenced workspace. Paths inside the session are relative, including replay sources in workspace settings; moving/copying the entire folder preserves those references. External artifacts retain absolute paths and may need reassociation on another machine. Local device paths, SSH hosts, and ephemeral PTYs remain machine-specific configuration.

Saves use temporary files, synchronization, and atomic replacement. Files edited externally are preserved: conflicting saves display an error rather than overwrite them. Copy any unsaved notes, then **Reload session files from disk** to adopt valid external changes; this discards unsaved notes and leaves terminals unchanged. Repair invalid files before reload. If files are missing or unwritable, restore them or their permissions and retry. Session content is ordinary readable data, so it can also be inspected or repaired outside Signal Forge.
