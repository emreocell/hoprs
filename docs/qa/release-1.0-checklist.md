# Release 1.0 checklist

One section per task; later tasks append theirs.

## Task 1: icons and window icon (2026-11-13)

| Check | Result |
| --- | --- |
| 16 px and 24 px PNGs (rendered straight from the SVG) | Pass: the "h" with its arc is still recognisable and not blurred; viewed at 1:1 |
| 256 px PNG | Pass: dark rounded square, white "h", arc ending in a dot |
| Window class | Pass: under XWayland, `xprop WM_CLASS` of the running window = `"io.github.engviniciusbs.hoprs", "io.github.engviniciusbs.hoprs"` |
| Window icon visible in the title bar / dock / alt-tab | NEEDS HUMAN: `xprop _NET_WM_ICON` returned an empty value under XWayland, and iced screenshots capture only the client area, so it could not be confirmed here. Steps: `cargo run -p hoprs-app --release`, then look at the window's icon in the dock and in alt-tab (no `.desktop` file is installed yet, so the icon must come from the window itself). A generic icon means the window icon is not applied |
