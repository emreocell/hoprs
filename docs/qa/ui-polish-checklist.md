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

2. **Disabled actions.** With nothing selected, look at the pane toolbars and the dialogs'
   outline buttons. Expect: enabled and disabled actions are told apart at a glance, and
   disabled outline buttons have a dashed outline.

3. **Logo.** Look at the top of the sidebar at 100% and 200% scale. Expect: the logo is
   sharp, aligned with "hoprs", and there is no version beside it.
