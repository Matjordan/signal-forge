# Traffic analysis and diagnostics

Each terminal has **Display…**, **Analysis…**, and **Statistics…** menus. They
work with local and SSH endpoints and use observed monitor traffic. These tools
never change serial bytes, recording, capture, or bridge forwarding.

## Visibility and direction labels

In **Display…**, choose **RX + TX**, **RX only**, or **TX only**, and independently
set **Show RX/TX labels**. RX-only with labels and timestamps disabled gives a
clean receive-only view. Direction colors and underlying metadata stay intact.
Hidden entries stay in retained history; switching back reveals them.

Selection and Copy All use the visible representation. Changing view settings
resets text selection. Display pause and clear retain their existing behavior;
statistics continue while the display is paused and survive clearing history.

## Search, filters, and highlights

**Analysis…** offers three shared pattern modes:

- **Text**: case-sensitive literal UTF-8 bytes. Backslashes are literal.
- **Hex bytes**: pairs such as `00 FF 0D 0A`, with whitespace between bytes.
- **Byte regex**: regular expressions over bytes, including `\x00`, `\xFF`,
  `\r`, and `\n`. Unicode character classes are disabled; `(?i)` enables ASCII
  case-insensitive matching. Regex search does not require valid UTF-8 input.

Search highlights matching rows. **Next** and **Previous** scroll to matches and
wrap at the ends. Navigation turns off auto-scroll so incoming traffic does not
pull the view away from the chosen result. Search operates on visible retained
rows, including an active content filter.

The content filter displays only matching rows; an empty filter shows all rows.
Invalid filters show an error and no matching rows until corrected. Invalid
search/highlight patterns are reported and ignored. Patterns match payload bytes,
not timestamps, labels, annotations, or display escape strings.

In Line mode matching spans assembled RX reads and includes the actual recognized
line terminator. In Raw Chunks and Hex modes each original event is matched
independently, using its bytes rather than its formatted display. Patterns do not
span separate logical rows or raw events. Display-truncated line tails are not
searchable in Line mode; use retained raw events for inspection.

Add up to 32 named highlight rules, each with Text, Hex, or Byte regex and Warning,
Error, or Ready emphasis. The first matching rule colors the row; its label is
available on hover. Search matches take visual priority over highlight rules.
Patterns are limited to 4096 bytes, regex compilation has memory limits, compiled
rules are reused until edited, and indexes rebuild when traffic or settings change.
Only visible rows are laid out for rendering.

Visibility, labels, search/filter rules, highlights, and timing options save per
terminal in the workspace. Older workspaces default to RX + TX with labels shown.
Restoring a workspace still opens its endpoints disconnected and sends no traffic.

## Timing annotations

Absolute UTC timestamps remain available under **Display…**. **Analysis…** adds
independent annotations for:

- delta from the previous displayed row;
- delta from the previous RX row;
- delta from the previous TX row;
- assembled RX line duration;
- observed TX → first RX latency.

Hidden rows update RX/TX clocks, but not the previous-displayed-row clock. Line
rows use their first contributing timestamp; durations use first and last RX read
timestamps. The existing line timing hover information remains available.
Negative intervals caused by wall-clock changes or line assembly order are omitted
rather than shown as fabricated zero durations. Values use µs, ms, or seconds.

Latency pairs the **latest nonempty TX event** with the **first subsequent nonempty
RX event** on that endpoint. A new TX before RX supersedes the outstanding TX;
additional RX events do not create another sample until a new TX arrives. This
applies to manual sends, presets, repeats, files, and bridge TX. A chunked write can
produce multiple TX events, so the measurement starts at the latest event, not at
an inferred whole command. A regressing wall clock discards that sample.

This measures observed activity, not protocol-level correlation or a device
acknowledgement. SSH TX time reflects acceptance by the SSH stream; network and
host buffering contribute to observed latency. The first relevant assembled RX
row shows an annotation; a fragmented line that began before TX can contain the
response activity. Statistics retains the latest TX/RX event association and the
last 2000 samples for row annotations, plus cumulative count and mean.

## Statistics

**Statistics…** reports RX/TX bytes, recent RX/TX bytes per second, assembled RX
line count, elapsed time, longest observed gap between RX events, and last/mean
TX → RX latency. It also reports superseded TX events and clock regressions.

Counters start when the terminal is created or **Reset statistics** is clicked.
Reset leaves history, display, connection, and recording unchanged. The existing
terminal header byte counters continue to show lifetime observed totals. Line
count follows the current delimiter; changing it affects future counting.

Rates use ten fixed 100 ms buckets, approximately a one-second window measured
with a monotonic clock, and decay to zero without new traffic. Elapsed time also
uses a monotonic clock. RX gaps and latency use original event timestamps.
Long-running state stays bounded. These are best-effort monitor statistics;
dropped events can make totals and timing incomplete. The workbench's dropped-event
indicator remains the source of monitoring-overload information.
