# Signal Forge

A serial-port workbench for working with multiple physical and virtual serial connections in one application.

## Target UI

> Concept mockup showing the intended finished application. The implemented UI may evolve as Signal Forge is built.

![Signal Forge target UI](./a_detailed_widescreen_dark_themed_desktop_applicat.png)

The workspace is designed around **tileable port windows**, allowing several serial terminals to be monitored and controlled side-by-side in the same application window.

## Planned Features

- Multiple serial ports open simultaneously
- Tileable terminal windows
- Independent baud rate, parity, data bits, stop bits, and flow-control settings
- Text and hex transmission
- Explicit `\r`, `\n`, and `\r\n` handling
- Repeated/timed message sending
- Reusable preset messages and device profiles
- Virtual serial-port pairs
- Full-duplex bridging between endpoints
- Live bridge traffic inspection
- ASCII and hex traffic views
- Timestamps and traffic direction
- Capture/export
- Saved workspaces and layouts

## Initial Platform

The first target is Linux, implemented in Rust with egui/eframe.
