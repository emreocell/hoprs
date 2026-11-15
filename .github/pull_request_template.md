## What and why

<!-- What this changes and why. Link the issue or discussion: "Closes #123". -->

## How it was tested

<!-- The tests you added or ran; for UI changes, a screenshot helps. -->

## Checklist

- [ ] `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` are clean
- [ ] `cargo test --workspace --all-features` passes (and the Docker integration tests, if SSH/FTP/transfers changed)
- [ ] New user-visible text is in both `crates/hoprs-app/i18n/*.ftl` files, not in the code
- [ ] `CHANGELOG.md` has a line, if users would notice the change
- [ ] Nothing new logs passwords, passphrases, keys, file contents or terminal bytes
