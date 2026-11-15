# hoprs app in Brazilian Portuguese (i18n) — design

Status: draft for the author's approval (2026-11-15); errors in scope by the author's choice (option A). Request: "Interface em português - br", one of
three projects to make hoprs more attractive to the community (the others: contributor
infrastructure, then folder compare/sync; each gets its own spec). The author left the how to the
builder ("você decide").

## Intent

hoprs is i18n-ready only in shape: every user-visible string is a `&'static str` constant or a
`format!` function in `crates/hoprs-app/src/ui/strings.rs` (~417 items, ~630 call sites), all in
English. The goal is a hoprs that a Brazilian developer can run in Portuguese, and a translation
format that people who never open a `.rs` file can extend to other languages later (GitHub web
editor today, a platform such as Weblate some day). The language is a runtime choice, not a build
flag: one binary, every language.

## Goals

- The whole app UI available in English and Brazilian Portuguese (pt-BR).
- The language follows the OS by default; Settings can pin one; the change applies at once, no
  restart.
- Translations live in plain text files a translator edits without Rust.
- A missing or broken translation can never crash the app or ship silently.

## Non-goals

- Other languages beyond en and pt-BR (the format makes them a file away; none ship now).
- Translating messages that come from servers or from the OS (a server's reply text, an SSH
  banner, an `io::Error`'s OS text): they are shown as received. hoprs's **own** error messages
  are in scope (see "Errors from the core crates").
- Translating the CLI (`hoprs --version`, `--help`), the technical log panel's lines, the
  `tracing` output, packaging metadata of the `.msi`/`.dmg`, the README or the docs.
- A Weblate/Crowdin setup (project 2, contributor infrastructure, documents how to translate).
- Right-to-left layout.

## Anti-goals

- A Portuguese that reads as machine translation. It must read naturally to a Brazilian developer,
  with product terms kept in English where Brazilians use them (SSH, SFTP, FTP, FTPS, terminal,
  workspace, upload, download, host, log, keyring, ssh-agent). Same rule as the site
  (`docs/superpowers/specs/2026-11-14-site-i18n-seo-design.md` §3).
- A slower UI: views are rebuilt often; looking a string up must cost about what a constant costs.
- Truncated or overflowing Portuguese labels (Portuguese runs ~20–30 % longer than English).

## Approaches considered

1. **Fluent (`.ftl` files, `fluent-bundle` crate) — chosen.** Mozilla's localization format: plain
   text, one file per language, plural and select rules per locale, variables by name, supported
   by Weblate and Pontoon. Costs: a runtime lookup (mitigated below) and keys that the compiler
   cannot check (mitigated by tests).
2. **A Rust struct per language** (`Strings { new_site: &'static str, … }`, one `static` per
   language). Compile-time completeness and zero cost, but translators edit Rust, plurals are Rust
   code, and no translation platform reads it. Rejected: it works against the reason for the
   project (community contributions).
3. **`rust-i18n` (YAML + macro).** Simple, but weaker plural support, a proc-macro on every call
   site and a less standard format for translators. Rejected.

## Design

### Files

- `crates/hoprs-app/i18n/en.ftl` and `crates/hoprs-app/i18n/pt-BR.ftl`, embedded with
  `include_str!` (no files to install, nothing to find at run time).
- Keys are kebab-case, derived from today's names (`NEW_SITE` → `new-site`,
  `site_count` → `site-count`), grouped with comments in the same order as `strings.rs` today
  (top bar, palette, sidebar, …). English is the reference file; its comments are the context a
  translator needs (where the string shows, what each variable is).
- Variables keep their meaning by name (`{ $count }`, `{ $reason }`, `{ $host }`). Plurals use
  Fluent selectors (`{ $count -> [one] … *[other] … }`), so "1 worker / 4 workers" moves out of Rust.

### The strings API

- `strings.rs` stays the only door to user-visible text, so call sites change mechanically:
  constants become functions of the same name in snake_case (`strings::NEW_SITE` →
  `strings::new_site()`), returning `&'static str`; today's functions keep their names and
  signatures, returning `String`.
- At language load, every message without variables is formatted once into a table and leaked
  (`Box::leak`; at most one table per language for the process's life, a few tens of KB), so the
  argument-free accessors return `&'static str` from an array/map lookup. Messages with variables
  format through the bundle on each call, as `format!` does today.
- The active language is process-wide (an atomic or `RwLock` swap of the active catalog).
  Switching it replaces the catalog; the next view shows the new language.
- Fallback: a key missing in pt-BR uses English; a key missing in English (a bug) shows the key
  itself and logs a warning once. Fluent formatting errors log and fall back the same way. Nothing
  panics.
- Arrays used today (`SIZE_UNITS`, `MONTHS`) become messages too (`month-1` … `month-12`, size
  units stay as symbols `B`, `KB`, … in both languages), behind functions (`strings::month(m)`)
  whose bodies match to literal-key lookups.
- **One lookup convention:** every accessor reaches the catalog through one function with a
  string-literal key (`tr("new-site")`, `tr_args("site-count", …)`), never a computed key, so the
  coverage test can read the keys from the source. *(Builder-check ruling.)*
- Keyboard shortcuts inside a sentence are variables (`{ $key }`), so translators never touch
  them.
- **Sweep:** user-visible literals in `hoprs-app` outside `strings.rs` (view code, `effects.rs`,
  `backend/` messages that reach the UI) move into the catalog too. A test or `grep` check in CI
  is not required; the sweep is a one-time task listed in the plan. *(Builder-check ruling.)*
- Strings that today embed English reasons from other crates (the transfer engine's skip reasons
  `exists`, `not newer`, `already complete`, stored with the item) are mapped to translated text
  in the app when they match those known values; any other text shows as received.

### Errors from the core crates

Author's decision (2026-11-15, option A): every error message hoprs itself writes is translated,
including the ~200 `Error::new(kind, "…")` sites in `hoprs-core`, `hoprs-ssh`, `hoprs-remote`,
`hoprs-vault`, `hoprs-transfer` and `hoprs-import`.

- The core crates stay free of any i18n dependency. `hoprs_core::Error` gains a stable message
  **code** and its **arguments**: `code: Option<&'static str>` (a kebab-case key such as
  `auth-wrong-password`, prefixed `error-` in the catalog) and `args: Vec<(&'static str,
  String)>` (named values such as `host`, `path`). A constructor sets both, e.g.
  `Error::coded(kind, "auth-wrong-password", "Wrong password")` and `.arg("host", host)`. The
  English `message` stays as it is today: it is what logs, `Display`, tests and the fallback use.
- Every hoprs-written `Error::new` in non-test code moves to the coded form. Errors that wrap a
  server's or the OS's text keep that text as an argument (`{ $reason }`) or as `detail`; the
  hoprs-written part around it is translated.
- `hoprs-app` turns an error into user text with one function (`strings::error(&Error) ->
  String`): with a code, it looks up `error-<code>` with the arguments; without a code, or if the
  key is missing, it shows `message`. Every place that shows an error's message in the UI goes
  through it. `detail` (technical, for the log) is never translated.
- A code is part of the error's contract: changing a message's meaning means a new code. Codes
  are unique across the workspace.
- **Persistence:** the transfer store (`hoprs-transfer/src/store.rs`, SQLite) saves an item's
  error as kind / message / detail. It gains `error_code` and `error_args` (JSON) columns through
  the store's existing schema-migration path; rows written by older versions have no code and show
  their English message. Older hoprs versions reading a newer database ignore the new columns (check
  the store's forward-compatibility rule and follow it).
- **Tests:** a test in `hoprs-app` lists every code used in the workspace's non-test source (by
  reading the `Error::coded(` literals, same convention idea as the strings coverage test) and
  checks `error-<code>` exists in both `.ftl` files with the arguments the call site passes; each
  core crate's tests keep asserting on `kind` and English `message` as today; one test per crate
  family checks a representative error carries its code.

### Numbers and dates

- `ui/format.rs` formats by the active language: pt-BR uses a comma decimal separator and a dot
  for thousands (`4,1 KB`, `1.000 sites`, `51%`), English stays as today. Done by hand in
  `format.rs` (two locales, small rules), not through an ICU dependency.
- Dates keep today's shape with the language's month abbreviations (`11 nov 14:02`,
  `02 out 2024` in pt-BR; lower case, as Brazilian Portuguese writes them).
- Keyboard shortcut labels (`Ctrl K`, `Ctrl Shift C`) stay as they are in both languages; on
  macOS they keep whatever the app shows today.

### Choosing the language

- New setting `language`: `system` (default), `en`, `pt-BR`, stored in the existing settings file
  (`settings.rs`), unknown values read as `system` and are kept by save, like the other keys.
- `system` reads the OS locale with the `sys-locale` crate (small, MIT OR Apache-2.0; Linux
  `LANG`/`LC_*`, macOS and Windows user locale). Any `pt*` locale (pt-BR, pt-PT, pt) picks pt-BR;
  everything else picks English.
- Settings gets a "Language" row with three choices; each language's name is written in itself
  ("System", "English", "Português (Brasil)", the first one translated).
- The choice applies immediately, with no restart. Rule: anything a view builds from state
  (labels, tab titles, headers, palette entries, settings) switches at once; text already written
  into a terminal's stream (`TERMINAL_RECONNECTED` and the like) or already formatted into stored
  state (a notice on screen, a queue item's reason) may stay in the old language until it is
  produced again. Where a view today caches a formatted label in state, it either rebuilds it on
  the language change or formats at view time (builder's choice per case). *(Builder-check ruling.)*

### Outside the app window

- Linux `.desktop` file: add `Comment[pt_BR]=` and `GenericName[pt_BR]=` (if a GenericName
  exists). Window title stays `hoprs`.
- Native file dialogs (rfd) already follow the OS language; their titles, if hoprs sets any, go
  through the catalog.

### Layout

- Every screen is checked in pt-BR for truncation and overflow: sidebar, tabs, file panes and
  their headers, queue, settings, site and workspace forms, dialogs, palette, status bar. Fixed
  widths that only fit English are widened or made to fit (the existing `ui/fit.rs` helpers); a
  label is never cut mid-word without an ellipsis.

## Tests

- **Parity:** every message in `en.ftl` exists in `pt-BR.ftl` and vice versa; each message uses
  the same set of variables in both; both files parse without errors (Fluent's parser reports
  them).
- **Coverage:** every key that `strings.rs` looks up exists in `en.ftl`, and every message in
  `en.ftl` is looked up by `strings.rs` (no dead keys). Checked by a test that reads
  `strings.rs`'s source for its lookup calls, so the convention is one lookup function with a
  string-literal key.
- **Not English by accident:** no pt-BR value equals its English value except an allowlist
  (product terms, symbols, names such as `hoprs`, `SFTP`, `B`, `KB`).
- **Plurals and numbers:** `site_count`, `total_workers` and the like for 0, 1, 2 in both
  languages; `format::size` / `date` / `percent` in pt-BR (`4,1 KB`, `11 nov 14:02`).
- **Settings:** `language` round-trips; unknown values read as `system`; OS-locale mapping
  (`pt_BR.UTF-8`, `pt-PT`, `en_US`, `de_DE`, empty) picks the right language.
- **Fallback:** a key missing from a test catalog falls back to English, then to the key; no
  panic.
- Tests that check pt-BR text build the pt-BR catalog directly instead of switching the
  process-wide language, so parallel tests that expect English never race.
- Existing tests keep passing in English (the default when nothing sets a language). The view
  benchmark (`bench.rs`) shows no measurable regression.
- QA checklist `docs/qa/i18n-checklist.md`: NEEDS HUMAN rows to walk every screen in pt-BR on a
  real display (layout, accents rendering in Inter and JetBrains Mono, language switch live, OS
  locale pt_BR picked on first launch).

## Constraints

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D
  warnings` clean on every commit; tests pass on Linux, macOS and Windows.
- New dependencies limited to `fluent-bundle` (and what it needs: `unic-langid`,
  `intl-memoizer`) and `sys-locale`, all MIT OR Apache-2.0. Only `hoprs-app` depends on them; the core crates gain only the
  code/args fields on `Error` (no new dependencies).
- UI strings in sentence case in both languages, as today.
- CHANGELOG entry under 1.2.0 (unreleased); README mentions the language setting.

## Left to the builder

The exact Portuguese copy; the catalog's data structure; how the lookup function is named; the
order the call sites are migrated in (one area per commit is fine); how to detect the locale on
each OS beyond what `sys-locale` gives; how layout fixes are made screen by screen.
