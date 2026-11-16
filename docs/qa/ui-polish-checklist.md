# Plan 13 checklist: UI polish

One run per release that touches the icons, context menus, drag and drop, or resizable
panels. AUTOMATED rows run in `cargo test --workspace`; NEEDS HUMAN rows need a person and a
real display.

## AUTOMATED

| Check | Result |
| --- | --- |
| Every icon is in the font, the font family is `Phosphor`, and `phosphor-glyphs.txt` matches the `Icon` enum (`crates/hoprs-app/src/ui/icons.rs` tests) | Pass |
| File icons by extension (`for_entry`) | Pass |

## NEEDS HUMAN

1. **Icon rendering.** Run hoprs at 100%, 150% and 200% scale on Linux, macOS and Windows.
   Expect: icons are crisp and sit on the same baseline as the label next to them.
