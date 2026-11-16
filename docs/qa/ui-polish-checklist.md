# Plan 13 checklist: UI polish

One run per release that touches the icons, context menus, drag and drop, or resizable
panels. AUTOMATED rows run in `cargo test --workspace`; NEEDS HUMAN rows need a person and a
real display.

## AUTOMATED

| Check | Result |
| --- | --- |
| Every icon is in the font, the font family is `Phosphor`, and `phosphor-glyphs.txt` matches the `Icon` enum (`crates/hoprs-app/src/ui/icons.rs` tests) | Pass |
| File icons by extension (`for_entry`) | Pass |
| Context menu model: opens at the last press, closes on Escape, outside click, resize and choice, blocked behind a dialog (`crates/hoprs-app/src/app/tests/menu.rs`); kept inside the window (`ui::menu` tests); press position published before the widget under it (`ui::pointer` tests) | Pass |
| Sidebar state saved: collapse and hide round-trip through `settings.toml`, invalid values read as shown, the ⋯ menu hides and shows a section (`settings` tests, `app/tests/import_settings.rs`); site state and icon by protocol (`ui::sidebar` tests) | Pass |

## NEEDS HUMAN

1. **Icon rendering.** Run hoprs at 100%, 150% and 200% scale on Linux, macOS and Windows.
   Expect: icons are crisp and sit on the same baseline as the label next to them.

2. **Disabled actions.** With nothing selected, look at the pane toolbars and the dialogs'
   outline buttons. Expect: enabled and disabled actions are told apart at a glance, and
   disabled outline buttons have a dashed outline.

3. **Logo.** Look at the top of the sidebar at 100% and 200% scale. Expect: the logo is
   sharp, aligned with "hoprs", and there is no version beside it.

4. **Context menu placement and closing.** Click the ⋯ button at the top of the sidebar.
   Expect: the menu opens at the pointer. Make the window small enough that the button sits
   near its right edge (or use a later menu near the right and bottom edges) and open it
   again. Expect: the menu flips to stay fully inside the window. Then check each way out:
   Escape, a click outside, a scroll and a window resize each close it, and nothing behind
   it reacts to that click.

5. **Sidebar rows.** Hover several site and workspace rows. Expect: the pencil appears only
   on the hovered row (over the protocol tag or count) and opens its form; connected sites
   show a green plug and a white name; a site that failed shows a red warning, and hovering
   the icon shows the error. Collapse and hide both sections from the caret and the ⋯ menu,
   restart, and expect the same state.
