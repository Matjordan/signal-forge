# Maintaining bundled help

`docs/user-guide.md` is the source for the offline guide, search, concise tooltips,
and expanded explanations. `src/help.rs` embeds it at compile time; no runtime
file or network fetch is needed. Packaged documentation also includes this source.

Each topic has this structure:

```
<!-- topic:stable.topic.id -->
## Human-readable title
> One-line concise tooltip.

The first paragraph is the expanded explanation. Explain the behavior and when
it is useful without requiring another topic to understand the control.

Further workflow steps, examples, and caveats follow here.

Related: [Other topic title](#other-topic-title).
```

Keep IDs stable when moving or renaming sections: controls and navigation target
the ID. Related links use the Markdown heading anchor and are checked by tests.
The renderer supports paragraphs, numbered steps, inline code, bold emphasis,
and the Related topic links. Keep other Markdown constructs out of topic bodies
unless the renderer is extended to support them.

Attach a topic with `app/help_ui::explain(response, "stable.topic.id")`, or register
a shared control key in `topic_for_control` for `theme::control` users. Do not copy
long explanations into widget code. The first paragraph and tooltip remain shared
with the full guide. Input is sampled before widgets consume keys; dialog scope
changes and disappearing targets invalidate pending/visible extended help.

Run `cargo test --all-targets` for IDs, links, search, hover timing/dismissal,
focus preservation and popup/viewer bounds. Run `scripts/help-smoke.py` under
Xvfb for the real ten-second interaction, searchable navigation, related links,
Back/history, modal control help, minimum window size, and no accidental TX.
