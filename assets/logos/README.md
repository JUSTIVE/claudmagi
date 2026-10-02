# Logos

Two kinds of mark. One says what a node *is* — `claude.svg` for a session,
`github.svg` for a pull request, `linear.svg` for an issue (#79, #80). The
other rides beside the Claude one and says where that session is *running*:
`warp.svg`, `orca.svg`, `anthropic.svg` for the Claude desktop app, and
`cloud.svg` for a session that is not on this machine at all (#120, #122).

The desktop app wears Anthropic's mark rather than Claude's. Claude's is
already in the slot next door, and two of them side by side read as a mistake
rather than as an answer.

| File | Where it came from |
|------|--------------------|
| `claude.svg`, `github.svg`, `linear.svg`, `warp.svg`, `anthropic.svg` | [simple-icons](https://github.com/simple-icons/simple-icons), whose icon files are CC0-1.0. |
| `orca.svg` | Orca's own `Contents/Resources/app.asar.unpacked/resources/logo.svg`, which simple-icons does not carry. |
| `cloud.svg` | Drawn here. It stands for a place rather than a product, so there is no mark to borrow. |

The marks themselves remain the trademarks of their owners and are used here
only to point at those services.

gpui renders an SVG as a mask and `logos::outline` flattens it to the same
path commands a glyph uses, so only the silhouette matters: the colour comes
from the label beside it. A replacement has to be a single-colour SVG with a
`viewBox`; a gradient would arrive as a flat blob, and ink that sits in one
corner of its own box arrives tiny. Both are checked by the tests in
`src/logos.rs`.
