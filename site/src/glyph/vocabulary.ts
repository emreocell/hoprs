// hoprs' own words, in order. The field spells them cell by cell, spaces skipped.
export const VOCAB =
  "ssh sftp ftps ftp put get mkdir chmod 0644 0755 drwxr-xr-x -rw-r--r-- ~/ ~/.ssh/ known_hosts id_ed25519 " +
  "ssh-agent profile.toml sites.toml workspace.toml sitemanager.xml rename rm -r ls -la cd .. pwd " +
  "port=22 port=21 user@host 1GiB 100k rows queue resume retry upload download chown utf-8 passive " +
  "AUTH TLS PBSZ 0 PROT P LIST MLSD STOR RETR RNFR RNTO SIZE MDTM argon2id xchacha20-poly1305 keyring " +
  "tokio russh suppaftp alacritty iced rust ctrl+k ";

const GLYPHS = VOCAB.replace(/\s+/g, "");

/** The glyph for cell `i`: the vocabulary in order, never a space. */
export function glyphAt(i: number): string {
  const n = GLYPHS.length;
  return GLYPHS[((i % n) + n) % n];
}

/** Every distinct glyph, for the atlas. */
export const GLYPH_SET = [...new Set(GLYPHS)].join("");
