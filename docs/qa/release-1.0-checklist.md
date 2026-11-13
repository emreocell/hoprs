# Release 1.0 checklist

One section per task; later tasks append theirs.

## Task 1: icons and window icon (2026-11-13)

| Check | Result |
| --- | --- |
| 16 px and 24 px PNGs (rendered straight from the SVG) | Pass: the "h" with its arc is still recognisable and not blurred; viewed at 1:1 |
| 256 px PNG | Pass: dark rounded square, white "h", arc ending in a dot |
| Window class | Pass: under XWayland, `xprop WM_CLASS` of the running window = `"io.github.engviniciusbs.hoprs", "io.github.engviniciusbs.hoprs"` |
| `_NET_WM_ICON` of the window (XWayland, re-verified in fix round 1) | Pass: `xwininfo -root -tree` gave the client window `0x2600004` with class `io.github.engviniciusbs.hoprs`; `xprop -id 0x2600004 _NET_WM_ICON` prints `Icon (256 x 256):` (xprop does not dump the pixels). The first pass misread it as empty because the text is on the lines after the `=` |
| Icon visibly drawn in the title bar / dock / alt-tab | NEEDS HUMAN: no screenshot shows window chrome. Steps: `cargo run --release`, look at the icon in the dock and in alt-tab (no `.desktop` is installed yet, so it must come from the window). Under native Wayland the icon comes from the `.desktop` file (Task 2) |
