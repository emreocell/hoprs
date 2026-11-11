# Plano 1: Fundação + motor de transferências — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Workspace Cargo do hoprs com `hoprs-core`, a abstração `RemoteFs` (com `MemFs`, `LocalFs` e `FaultFs`) e o motor de transferências completo (`hoprs-transfer`), tudo testado sem rede e sem GUI.

**Architecture:** O motor é um *actor* tokio: uma task dona do estado recebe comandos por `mpsc` e publica eventos por `broadcast`. Ele transfere entre dois `RemoteFs` por site (local e remoto). Assim, os testes usam `MemFs` nos dois lados, e os planos 2 e 3 só precisam fornecer `SftpFs`/`FtpFs`. A fila persiste em SQLite.

**Tech Stack:** Rust stable (edition 2024), tokio 1.54, tokio-util 0.7 (`CancellationToken`), async-trait 0.1, thiserror 2, rusqlite 0.42 (feature `bundled`), tempfile 3 (dev), criterion 0.7 (dev).

**Spec:** [`docs/superpowers/specs/2026-11-11-hoprs-design.md`](../specs/2026-11-11-hoprs-design.md). Roadmap: [`2026-11-11-00-roadmap.md`](2026-11-11-00-roadmap.md).

## Global Constraints

- Plataforma alvo: Linux (Ubuntu). O código não pode usar APIs exclusivas de Linux nos crates deste plano. Os testes precisam passar também no Windows (usar `tempfile`, nunca `/tmp` literal).
- Nenhum crate deste plano depende de `iced`.
- Grafo de dependências: todos → `hoprs-core`. `hoprs-transfer` → `hoprs-remote`. Nenhuma outra aresta.
- Erros: um único `hoprs_core::Error` com categoria, mensagem legível e detalhe técnico (spec §10).
- Progresso agregado a **≤ 10 Hz** antes de sair do motor (spec §4.4).
- Workers por site: padrão **4**, faixa **1..=16** (spec §5).
- Buffer de cópia: **256 KB** (spec §5, FTP. Também usado como padrão genérico do motor).
- Política de conflito padrão: **`Ask`**. Políticas: `Ask`, `Overwrite`, `Skip`, `OverwriteIfNewer`, `Resume` (spec §5).
- Fila em SQLite. Ao reabrir, itens incompletos voltam como **pausados** (spec §5).
- Logs e mensagens de erro nunca contêm conteúdo de arquivo.
- `cargo fmt --check` e `cargo clippy --all-targets -- -D warnings` limpos em todo commit.
- Licença dupla MIT OR Apache-2.0 em todos os crates.

## Review Focus

1. **Arquivo de 0 bytes:** precisa ser criado no destino com tamanho 0 e terminar `Done`, nunca ser pulado nem ficar preso em `Active`. → teste em Task 8.
2. **Nomes com espaços, unicode e ponto inicial** (`"relatório final.pdf"`, `".env"`): o caminho de destino precisa ser idêntico ao da origem após `join`. → testes em Task 1 e Task 10.
3. **Origem some ou falha no meio da transferência** (`NotFound` ao abrir, erro de I/O na leitura): só aquele item vai para `Failed`, os outros continuam, e o worker é liberado. → teste em Task 9.
4. **Pasta vazia e pasta com links simbólicos:** a pasta vazia é criada no destino e termina `Done`. Symlinks são marcados `Skipped` com o motivo `"symlinks are not followed"` e não derrubam a pasta. → testes em Task 10.
5. **Reinício do app com itens `Active` ou `AwaitingDecision`:** todos voltam como `Paused`. Ao retomar, `Active` continua do offset do destino, e `AwaitingDecision` volta a perguntar. → teste em Task 12.

---

## Estrutura de arquivos

```
Cargo.toml                         # [workspace], deps compartilhadas
rust-toolchain.toml                # channel = "stable", components rustfmt, clippy
.github/workflows/ci.yml           # fmt, clippy, test (ubuntu-latest)
LICENSE-MIT, LICENSE-APACHE
crates/hoprs-core/src/
  lib.rs                           # re-exports
  path.rs                          # RemotePath
  error.rs                         # Error, ErrorKind, Result
  entry.rs                         # RemoteEntry, EntryKind
  ids.rs                           # SiteId
crates/hoprs-remote/src/
  lib.rs                           # trait RemoteFs, BoxAsyncRead/Write
  local.rs                         # LocalFs
  mem.rs                           # MemFs          (feature "testing")
  fault.rs                         # FaultFs<F>     (feature "testing")
crates/hoprs-transfer/src/
  lib.rs                           # re-exports
  model.rs                         # TransferId, Direction, TransferSpec, TransferState, TransferItem, ConflictPolicy
  conflict.rs                      # decide()
  store.rs                         # QueueStore (SQLite)
  copy.rs                          # copy_file()
  progress.rs                      # ProgressTracker (agregação 10 Hz)
  engine/mod.rs                    # TransferEngine (handle), EngineConfig, Event, Resolution
  engine/actor.rs                  # loop do actor: comandos, despacho, workers
  engine/worker.rs                 # execução de um item (arquivo ou pasta)
crates/hoprs-transfer/tests/
  common/mod.rs                    # helpers: engine_with_memfs(), wait_for()
  engine_files.rs, engine_dirs.rs, engine_control.rs, engine_recovery.rs
crates/hoprs-transfer/benches/engine.rs
```

---

### Task 1: Workspace, CI e `RemotePath`

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.github/workflows/ci.yml`, `LICENSE-MIT`, `LICENSE-APACHE`, `README.md` (3 linhas: nome, uma frase, link da spec)
- Create: `crates/hoprs-core/Cargo.toml`, `crates/hoprs-core/src/lib.rs`, `crates/hoprs-core/src/path.rs`

**Interfaces:**
- Produces: `hoprs_core::RemotePath` com
  - `RemotePath::new(s: &str) -> Result<RemotePath, PathError>`: aceita apenas caminhos absolutos POSIX. Normaliza `//` → `/`, remove `.` e a barra final (exceto na raiz). Resolve `..` e retorna `PathError::EscapesRoot` se subir acima de `/`. Retorna `PathError::NotAbsolute` para caminho relativo.
  - `root() -> RemotePath`, `as_str(&self) -> &str`, `join(&self, name: &str) -> RemotePath` (`name` é um único componente: entra cru, sem normalização, e entra em pânico se contiver `/` ou for `.`/`..`), `parent(&self) -> Option<RemotePath>` (`None` na raiz), `file_name(&self) -> Option<&str>`.
  - Deriva `Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord`. Implementa `Display`.
  - `PathError` (enum `thiserror`): `NotAbsolute`, `EscapesRoot`.

- [ ] **Step 1: Criar o workspace**

`Cargo.toml` raiz com `[workspace] resolver = "3"`, `members = ["crates/*"]`, `[workspace.package] edition = "2024", license = "MIT OR Apache-2.0"` e `[workspace.dependencies]` com as versões do Tech Stack. `ci.yml` com um job `ubuntu-latest`: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`.

- [ ] **Step 2: Escrever os testes de `RemotePath` (falhando)** em `crates/hoprs-core/src/path.rs`, `mod tests`

```rust
#[test] fn normalizes() {
    assert_eq!(RemotePath::new("//var//www/./site/").unwrap().as_str(), "/var/www/site");
    assert_eq!(RemotePath::new("/a/b/../c").unwrap().as_str(), "/a/c");
    assert_eq!(RemotePath::new("/").unwrap().as_str(), "/");
}
#[test] fn rejects_relative_and_escape() {
    assert_eq!(RemotePath::new("var/www"), Err(PathError::NotAbsolute));
    assert_eq!(RemotePath::new("/.."), Err(PathError::EscapesRoot));
}
#[test] fn join_parent_file_name() {
    let p = RemotePath::root().join("var").join("relatório final.pdf");
    assert_eq!(p.as_str(), "/var/relatório final.pdf");
    assert_eq!(p.file_name(), Some("relatório final.pdf"));
    assert_eq!(p.parent().unwrap().as_str(), "/var");
    assert_eq!(RemotePath::root().parent(), None);
    assert_eq!(RemotePath::root().join(".env").as_str(), "/.env");
}
#[test] #[should_panic] fn join_rejects_separator() { RemotePath::root().join("a/b"); }
```

- [ ] **Step 3: Rodar e ver falhar**

Run: `cargo test -p hoprs-core`
Expected: erro de compilação (`RemotePath` não definido).

- [ ] **Step 4: Implementar `RemotePath` e `PathError` em `path.rs`, re-exportar em `lib.rs`**

- [ ] **Step 5: Rodar e ver passar**

Run: `cargo test -p hoprs-core && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: 4 testes passam. Clippy e fmt sem saída de erro.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml rust-toolchain.toml .github LICENSE-* README.md crates/hoprs-core
git commit -m "feat(core): workspace, CI and RemotePath"
```

---

### Task 2: `Error`, `RemoteEntry` e `SiteId`

**Files:**
- Create: `crates/hoprs-core/src/error.rs`, `crates/hoprs-core/src/entry.rs`, `crates/hoprs-core/src/ids.rs`
- Modify: `crates/hoprs-core/src/lib.rs` (re-exports)

**Interfaces:**
- Produces:
  - `enum ErrorKind { Network, Auth, Permission, Conflict, Protocol, HostKey, Tls, Io, NotFound }` (`Clone, Copy, Debug, PartialEq, Eq, Hash`). `NotFound` é um acréscimo à lista da spec §10, necessário para distinguir "destino não existe" na retomada.
  - `ErrorKind::as_str(self) -> &'static str` em snake_case (`"network"`, `"not_found"`…) e `ErrorKind::parse(s: &str) -> Option<ErrorKind>`, usados pelo SQLite.
  - `struct Error { kind: ErrorKind, message: String, detail: Option<String> }` com `thiserror` (`#[error("{message}")]`), campos públicos, `Clone, Debug, PartialEq`.
  - Construtores: `Error::new(kind, message: impl Into<String>) -> Error` e `Error::with_detail(self, detail: impl Into<String>) -> Error`.
  - `impl From<std::io::Error> for Error`: `NotFound` → `ErrorKind::NotFound`, `PermissionDenied` → `Permission`, `ConnectionReset | ConnectionAborted | BrokenPipe | TimedOut` → `Network`, o resto → `Io`. `message` = `io_err.to_string()`.
  - `type Result<T> = std::result::Result<T, Error>`.
  - `enum EntryKind { File, Dir, Symlink }`, `struct RemoteEntry { name: String, path: RemotePath, kind: EntryKind, size: u64, modified: Option<SystemTime>, mode: Option<u32> }` (`Clone, Debug, PartialEq`).
  - `struct SiteId(pub String)` (`Clone, Debug, PartialEq, Eq, Hash`), com `impl Display` imprimindo a string interna.

- [ ] **Step 1: Testes (falhando)** em `error.rs` `mod tests`

```rust
#[test] fn io_mapping() {
    use std::io::{Error as Io, ErrorKind as K};
    assert_eq!(Error::from(Io::from(K::NotFound)).kind, ErrorKind::NotFound);
    assert_eq!(Error::from(Io::from(K::PermissionDenied)).kind, ErrorKind::Permission);
    assert_eq!(Error::from(Io::from(K::ConnectionReset)).kind, ErrorKind::Network);
    assert_eq!(Error::from(Io::from(K::InvalidData)).kind, ErrorKind::Io);
}
#[test] fn kind_roundtrip() {
    for k in [ErrorKind::Network, ErrorKind::Auth, ErrorKind::Permission, ErrorKind::Conflict,
              ErrorKind::Protocol, ErrorKind::HostKey, ErrorKind::Tls, ErrorKind::Io, ErrorKind::NotFound] {
        assert_eq!(ErrorKind::parse(k.as_str()), Some(k));
    }
}
#[test] fn display_is_message_only() {
    let e = Error::new(ErrorKind::Auth, "Wrong password").with_detail("SSH_MSG_USERAUTH_FAILURE");
    assert_eq!(e.to_string(), "Wrong password");
    assert_eq!(e.detail.as_deref(), Some("SSH_MSG_USERAUTH_FAILURE"));
}
```

- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-core`. Expected: erro de compilação.
- [ ] **Step 3: Implementar `error.rs`, `entry.rs`, `ids.rs` e os re-exports**
- [ ] **Step 4: Rodar e ver passar.** Run: `cargo test -p hoprs-core`. Expected: 7 testes passam.
- [ ] **Step 5: Commit.** `git commit -am "feat(core): Error, RemoteEntry, SiteId"` (adicionar os arquivos novos antes).

---

### Task 3: Trait `RemoteFs` + `MemFs`

**Files:**
- Create: `crates/hoprs-remote/Cargo.toml` (feature `testing = []`), `crates/hoprs-remote/src/lib.rs`, `crates/hoprs-remote/src/mem.rs`

**Interfaces:**
- Consumes: `RemotePath`, `RemoteEntry`, `EntryKind`, `Error`, `ErrorKind`, `Result` (Tasks 1–2).
- Produces:
  - `pub type BoxAsyncRead = Box<dyn tokio::io::AsyncRead + Send + Unpin>;` e `pub type BoxAsyncWrite = Box<dyn tokio::io::AsyncWrite + Send + Unpin>;`
  - `#[async_trait] pub trait RemoteFs: Send + Sync` com os métodos da spec §4.2, exatamente: `list`, `stat`, `open_read(path, offset)`, `open_write(path, offset)`, `mkdir`, `rename`, `remove_file`, `remove_dir`, `chmod(path, mode: u32)`, mais `async fn set_modified(&self, path: &RemotePath, mtime: SystemTime) -> Result<()>` com implementação padrão `Ok(())`.
  - Semântica de contrato (documentar no trait): `list` não inclui `.`/`..` e retorna symlinks como `EntryKind::Symlink` sem segui-los. `stat` de um caminho inexistente → `ErrorKind::NotFound`. `open_write` com `offset == 0` cria ou trunca. Com `offset > 0`, exige que o arquivo exista com tamanho ≥ `offset`, trunca para `offset` e escreve a partir dali. `mkdir` em pasta já existente → `Ok(())`.
  - `#[cfg(feature = "testing")] pub struct MemFs` (`Clone`, estado em `Arc<Mutex<..>>`), com `MemFs::new() -> MemFs` (só a raiz `/`), `add_file(&self, path: &str, data: &[u8], mtime: SystemTime)` (cria as pastas pai), `add_dir(&self, path: &str)`, `add_symlink(&self, path: &str, target: &str)`, `read_file(&self, path: &str) -> Option<Vec<u8>>`, `deny_writes_under(&self, path: &str)` (escrita, mkdir e remove sob esse prefixo → `ErrorKind::Permission`).
  - O writer do `MemFs` grava no buffer compartilhado a cada `poll_write`, sem precisar de flush.

- [ ] **Step 1: Testes do `MemFs` (falhando)** em `mem.rs` `mod tests` (`#[tokio::test]`)

```rust
let fs = MemFs::new();
fs.add_file("/a/b.txt", b"hello", UNIX_EPOCH);
let list = fs.list(&p("/a")).await.unwrap();
assert_eq!(list.len(), 1); assert_eq!(list[0].name, "b.txt"); assert_eq!(list[0].size, 5);
// leitura a partir de offset
let mut s = String::new(); fs.open_read(&p("/a/b.txt"), 2).await.unwrap().read_to_string(&mut s).await.unwrap();
assert_eq!(s, "llo");
// escrita com offset trunca e continua
let mut w = fs.open_write(&p("/a/b.txt"), 3).await.unwrap(); w.write_all(b"p!").await.unwrap();
assert_eq!(fs.read_file("/a/b.txt").unwrap(), b"help!");
// not found, permission, symlink
assert_eq!(fs.stat(&p("/nope")).await.unwrap_err().kind, ErrorKind::NotFound);
fs.deny_writes_under("/ro");
assert_eq!(fs.mkdir(&p("/ro/x")).await.unwrap_err().kind, ErrorKind::Permission);
fs.add_symlink("/a/link", "/a/b.txt");
assert!(fs.list(&p("/a")).await.unwrap().iter().any(|e| e.name == "link" && e.kind == EntryKind::Symlink));
```

Testes separados para: `mkdir` idempotente, `rename` de arquivo, `remove_dir` de pasta não vazia → `ErrorKind::Io`, `open_write(path, 0)` criando um arquivo novo vazio (`read_file` == `b""`).

- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-remote --features testing`. Expected: erro de compilação.
- [ ] **Step 3: Implementar o trait em `lib.rs` e `MemFs` em `mem.rs`.** O estado é um `BTreeMap<RemotePath, Node>`, com `Node::{File{data, mtime, mode}, Dir{mtime}, Symlink{target}}`. `list` filtra por `parent() == path`.
- [ ] **Step 4: Rodar e ver passar.** Run: o mesmo comando. Expected: todos os testes passam.
- [ ] **Step 5: Commit.** `git commit -m "feat(remote): RemoteFs trait and MemFs"`

---

### Task 4: `LocalFs`

**Files:**
- Create: `crates/hoprs-remote/src/local.rs`
- Test: `crates/hoprs-remote/tests/local.rs`

**Interfaces:**
- Produces: `pub struct LocalFs { root: PathBuf }`, `LocalFs::new(root: impl Into<PathBuf>) -> LocalFs`. Mapeia `RemotePath` `/a/b` → `root.join("a").join("b")`. Em produção, `root = "/"`. Nos testes, `root` é um `tempfile::TempDir`. Implementa `RemoteFs` com `tokio::fs`, respeitando o contrato da Task 3. Usa `symlink_metadata` para não seguir links. `chmod` no Unix usa `PermissionsExt`, e no Windows é `Ok(())`. `set_modified` usa `std::fs::File::set_modified` via `spawn_blocking`.

- [ ] **Step 1: Testes (falhando)** em `tests/local.rs`: os mesmos cenários do `MemFs` (list, offset read, offset write, NotFound, mkdir idempotente, arquivo vazio) contra um `TempDir`, mais `set_modified` (escrever, setar `UNIX_EPOCH + 1_000_000s`, `stat().modified` igual).
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-remote --test local`. Expected: erro de compilação.
- [ ] **Step 3: Implementar `LocalFs`**
- [ ] **Step 4: Rodar e ver passar.** Run: o mesmo comando. Expected: todos passam.
- [ ] **Step 5: Commit.** `git commit -m "feat(remote): LocalFs"`

---

### Task 5: `FaultFs` (injeção de falhas)

**Files:**
- Create: `crates/hoprs-remote/src/fault.rs` (feature `testing`)

**Interfaces:**
- Consumes: `RemoteFs`, `MemFs`.
- Produces: `pub struct FaultFs<F: RemoteFs> { inner: F, .. }` (`Clone` quando `F: Clone`, estado de falhas em `Arc<Mutex<..>>`), com:
  - `FaultFs::new(inner: F) -> Self`
  - `fail_io_after(&self, bytes: u64, kind: ErrorKind)`: a próxima leitura **ou** escrita acumulada que passar de `bytes` retorna `io::Error` mapeável para `kind` (usar `io::Error::new(io_kind_for(kind), ..)`, de modo que `Error::from` recupere o `kind`). Dispara **uma única vez**.
  - `fail_next(&self, op: Op, kind: ErrorKind)` com `enum Op { List, Stat, OpenRead, OpenWrite, Mkdir }`: a próxima chamada daquela operação falha uma vez.
  - `set_latency(&self, d: Duration)`: atraso antes de cada operação do trait.
  - `ops(&self) -> usize`: contador de chamadas do trait (para asserts).

- [ ] **Step 1: Testes (falhando)** em `fault.rs` `mod tests`: (a) `fail_io_after(3, Network)` lendo `"hello"` produz um erro cujo `Error::from(io_err).kind == Network`, e a segunda leitura completa funciona. (b) `fail_next(Op::Stat, NotFound)` falha uma vez e depois passa. (c) com `set_latency(50ms)`, um `stat` leva ≥ 50 ms (`tokio::time::pause()` + `advance`).
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-remote --features testing fault`
- [ ] **Step 3: Implementar `FaultFs`** com wrappers `FaultRead`/`FaultWrite` que contam bytes num contador compartilhado.
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(remote): FaultFs for fault injection"`

---

### Task 6: Modelo de transferência e `decide()`

**Files:**
- Create: `crates/hoprs-transfer/Cargo.toml` (dev-dep `hoprs-remote` com `features = ["testing"]`), `src/lib.rs`, `src/model.rs`, `src/conflict.rs`

**Interfaces:**
- Consumes: `RemoteEntry`, `SiteId`, `RemotePath`, `Error`.
- Produces (`model.rs`):
  - `pub struct TransferId(pub i64)` (`Copy, Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd`)
  - `pub enum Direction { Upload, Download }`
  - `pub enum ConflictPolicy { Ask, Overwrite, Skip, OverwriteIfNewer, Resume }`
  - `pub struct TransferSpec { pub site: SiteId, pub direction: Direction, pub local: RemotePath, pub remote: RemotePath }`. O tipo (arquivo ou pasta) é descoberto pelo motor via `stat` na origem.
  - `pub enum TransferState { Queued, Active, Paused, AwaitingDecision(ConflictInfo), Done, Skipped(String), Failed(Error), Cancelled }` com `is_terminal(&self) -> bool` (`Done | Skipped | Failed | Cancelled`)
  - `pub struct ConflictInfo { pub src_size: u64, pub dst_size: u64, pub src_modified: Option<SystemTime>, pub dst_modified: Option<SystemTime> }`
  - `pub struct TransferItem { pub id: TransferId, pub parent: Option<TransferId>, pub spec: TransferSpec, pub is_dir: Option<bool>, pub size: Option<u64>, pub bytes_done: u64, pub state: TransferState }`
- Produces (`conflict.rs`): `pub enum Decision { Write { offset: u64 }, Skip(String), Ask }` e `pub fn decide(policy: ConflictPolicy, src: &RemoteEntry, dst: Option<&RemoteEntry>) -> Decision`.

**Regras de `decide`** (a tabela é o contrato):

| dst | política | resultado |
|---|---|---|
| `None` | qualquer | `Write{offset:0}` |
| `Some` | `Ask` | `Ask` |
| `Some` | `Overwrite` | `Write{0}` |
| `Some` | `Skip` | `Skip("exists")` |
| `Some` | `OverwriteIfNewer` | `src.modified > dst.modified` → `Write{0}`. Senão → `Skip("not newer")`. Se algum mtime é `None` → `Write{0}` |
| `Some` | `Resume` | `dst.size < src.size` → `Write{offset: dst.size}`. `==` → `Skip("already complete")`. `>` → `Write{0}` |

- [ ] **Step 1: Testes (falhando)** em `conflict.rs`: um `#[test]` por linha da tabela (8 casos, incluindo os três ramos de `Resume` e o mtime `None`), com asserts de igualdade sobre `Decision`.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-transfer conflict`
- [ ] **Step 3: Implementar `model.rs` e `decide`**
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(transfer): model and conflict decision table"`

---

### Task 7: `QueueStore` (SQLite)

**Files:**
- Create: `crates/hoprs-transfer/src/store.rs`

**Interfaces:**
- Consumes: modelo da Task 6, `ErrorKind::as_str/parse`.
- Produces: `pub struct QueueStore` (síncrono, dono de um `rusqlite::Connection`. O actor o usa diretamente, já que as operações são curtas), com:
  - `open(path: &Path) -> Result<QueueStore>` e `open_in_memory() -> Result<QueueStore>`. Ambos rodam a migração e ativam `PRAGMA journal_mode=WAL` (exceto em memória).
  - `insert(&mut self, parent: Option<TransferId>, spec: &TransferSpec) -> Result<TransferId>` (estado `Queued`)
  - `set_state(&mut self, id: TransferId, state: &TransferState) -> Result<()>`, `set_meta(&mut self, id, is_dir: bool, size: Option<u64>) -> Result<()>`, `set_bytes_done(&mut self, id, bytes: u64) -> Result<()>`
  - `get(&self, id) -> Result<Option<TransferItem>>`, `list_all(&self) -> Result<Vec<TransferItem>>` (ordem por id), `children(&self, parent: TransferId) -> Result<Vec<TransferItem>>`
  - `recover(&mut self) -> Result<Vec<TransferItem>>`: converte `Active` e `AwaitingDecision` em `Paused` e devolve todos os não terminais.
  - `clear_finished(&mut self) -> Result<usize>`: remove itens raiz terminais e seus filhos.
- Schema (uma migração, `PRAGMA user_version = 1`):

```sql
CREATE TABLE transfers (
  id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES transfers(id) ON DELETE CASCADE,
  site TEXT NOT NULL, direction TEXT NOT NULL, local_path TEXT NOT NULL, remote_path TEXT NOT NULL,
  is_dir INTEGER, size INTEGER, bytes_done INTEGER NOT NULL DEFAULT 0,
  state TEXT NOT NULL, state_detail TEXT, error_kind TEXT, error_message TEXT, error_detail TEXT,
  conflict_json TEXT, created_at INTEGER NOT NULL);
CREATE INDEX transfers_parent ON transfers(parent_id);
```

`state` em snake_case (`queued`, `active`, `paused`, `awaiting_decision`, `done`, `skipped`, `failed`, `cancelled`). `state_detail` guarda o motivo de `Skipped`. `conflict_json` guarda o `ConflictInfo` serializado à mão (4 campos, mtime em segundos Unix).

- [ ] **Step 1: Testes (falhando)** em `store.rs` `mod tests`: (a) round-trip de cada variante de `TransferState`, incluindo `Failed(Error{kind: Permission, detail: Some(..)})` e `AwaitingDecision`. (b) `children` retorna só os filhos do pai. (c) `recover` transforma `Active` e `AwaitingDecision` em `Paused`, preserva `Done` e não o devolve. (d) `open(path)`, fechar, `open(path)` de novo e os itens persistem (`tempfile`). (e) `clear_finished` remove a raiz `Done` e os filhos dela, e mantém a raiz `Queued`.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-transfer store`
- [ ] **Step 3: Implementar `QueueStore`**
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(transfer): SQLite queue store"`

---

### Task 8: `copy_file` e `ProgressTracker`

**Files:**
- Create: `crates/hoprs-transfer/src/copy.rs`, `crates/hoprs-transfer/src/progress.rs`

**Interfaces:**
- Consumes: `RemoteFs`, `MemFs`, `FaultFs`.
- Produces:
  - `pub async fn copy_file(src: &dyn RemoteFs, src_entry: &RemoteEntry, dst: &dyn RemoteFs, dst_path: &RemotePath, offset: u64, buf_size: usize, progress: &AtomicU64, cancel: &CancellationToken) -> Result<CopyOutcome>`, com `pub enum CopyOutcome { Completed, Cancelled }`.
    - Abre `src.open_read(src_entry.path, offset)` e `dst.open_write(dst_path, offset)`. Faz o loop com buffer de `buf_size` e `progress.store(offset + copiado)` a cada bloco, checando `cancel.is_cancelled()` entre blocos. Termina com `shutdown()` no writer.
    - Pós-condição: `dst.stat(dst_path).size == src_entry.size`, senão `Error::new(Protocol, "Size mismatch after transfer").with_detail(format!("expected {}, got {}", ..))`.
    - Se `src_entry.modified` é `Some`, chama `dst.set_modified`. Uma falha aqui **não** falha a transferência (ignora o erro).
  - `pub struct ProgressTracker`: `new() -> Self`, `register(&mut self, id: TransferId, total: Option<u64>) -> Arc<AtomicU64>`, `unregister(&mut self, id)`, `sample(&mut self, now: Instant) -> Vec<ProgressUpdate>` (só os itens cujo `bytes_done` mudou desde a última amostra).
  - `pub struct ProgressUpdate { pub id: TransferId, pub bytes_done: u64, pub total: Option<u64>, pub bytes_per_sec: u64 }`. `bytes_per_sec` é a média móvel exponencial (α = 0.3) de `delta_bytes / delta_t`.

- [ ] **Step 1: Testes (falhando)** em `copy.rs` `mod tests`:
  - `copies_whole_file`: 1 MiB aleatório (seed fixa), `buf_size = 64 KiB`, bytes iguais, `progress == 1 MiB`, mtime do destino igual ao da origem.
  - `copies_zero_byte_file` **(Review Focus 1)**: origem `b""` → destino existe com `read_file == b""` e `Completed`.
  - `resumes_from_offset`: destino com os primeiros 400 KB corretos, `offset = 400_000`, resultado idêntico à origem.
  - `cancel_stops_early`: token cancelado após o primeiro bloco (cancelar dentro de um `FaultFs` com latência) → `Cancelled`, `progress < size`.
  - `io_error_propagates_kind`: `FaultFs::fail_io_after(100_000, Network)` na origem → `Err(e)` com `e.kind == Network`.
- [ ] **Step 2: Testes (falhando)** em `progress.rs`: com `register` + `store(1000)` em t0 e `store(3000)` em t0+100 ms, `sample` em t0+100 ms retorna `bytes_per_sec > 0` e `bytes_done == 3000`. Uma segunda `sample` sem mudança retorna vazio.
- [ ] **Step 3: Rodar e ver falhar.** Run: `cargo test -p hoprs-transfer copy progress`
- [ ] **Step 4: Implementar `copy_file` e `ProgressTracker`**
- [ ] **Step 5: Rodar e ver passar**
- [ ] **Step 6: Commit.** `git commit -m "feat(transfer): copy_file with resume/cancel and progress tracker"`

---

### Task 9: `TransferEngine` — arquivos

**Files:**
- Create: `crates/hoprs-transfer/src/engine/mod.rs`, `engine/actor.rs`, `engine/worker.rs`
- Test: `crates/hoprs-transfer/tests/common/mod.rs`, `crates/hoprs-transfer/tests/engine_files.rs`

**Interfaces:**
- Consumes: Tasks 6–8.
- Produces (`engine/mod.rs`):

```rust
pub struct EngineConfig { pub buf_size: usize, pub progress_interval: Duration, pub default_policy: ConflictPolicy }
impl Default for EngineConfig // 256 * 1024, 100 ms, ConflictPolicy::Ask
pub struct SiteEndpoints { pub local: Arc<dyn RemoteFs>, pub remote: Arc<dyn RemoteFs>, pub workers: usize } // workers clamp 1..=16
pub enum ResolveAction { Overwrite, Skip, Resume }
pub struct Resolution { pub action: ResolveAction, pub apply_to_all: bool }
#[derive(Clone, Debug)]
pub enum Event {
    Added(TransferItem),
    StateChanged { id: TransferId, state: TransferState },
    Progress(Vec<ProgressUpdate>),
    SitePaused { site: SiteId, error: Error },
}
#[derive(Clone)] pub struct TransferEngine { /* mpsc::Sender<Command>, broadcast::Sender<Event> */ }
impl TransferEngine {
    pub fn start(store: QueueStore, config: EngineConfig) -> TransferEngine; // spawna o actor (exige runtime tokio)
    pub fn subscribe(&self) -> broadcast::Receiver<Event>;               // capacidade 1024
    pub async fn register_site(&self, site: SiteId, endpoints: SiteEndpoints);
    pub async fn enqueue(&self, spec: TransferSpec) -> Result<TransferId>;
    pub async fn pause(&self, id: TransferId) -> Result<()>;
    pub async fn resume(&self, id: TransferId) -> Result<()>;
    pub async fn cancel(&self, id: TransferId) -> Result<()>;
    pub async fn retry(&self, id: TransferId) -> Result<()>;
    pub async fn resolve(&self, id: TransferId, resolution: Resolution) -> Result<()>;
    pub async fn set_policy(&self, policy: ConflictPolicy);
    pub async fn pause_site(&self, site: SiteId);
    pub async fn resume_site(&self, site: SiteId);
    pub async fn snapshot(&self) -> Vec<TransferItem>;
}
```

- Comportamento desta task (só arquivos): o actor mantém, por site, um contador de workers livres. A cada mudança, despacha o próximo item `Queued` daquele site em ordem de `id`. O worker faz `stat` na origem (`local` para `Upload`, `remote` para `Download`), `stat` no destino (`NotFound` → `None`), chama `decide` com a política atual, e aí executa `copy_file` ou devolve `Skip`/`Ask` ao actor. Ao terminar, o resultado volta ao actor como mensagem interna, e o actor persiste no store e emite `StateChanged`. Um ticker de `progress_interval` chama `ProgressTracker::sample` e emite `Progress` se não estiver vazio. **Esse é o único ponto que emite progresso.**
- Um item cujo site não está registrado continua `Queued` até `register_site`.
- `tests/common/mod.rs`: `pub fn engine_with_memfs(workers: usize) -> (TransferEngine, MemFs /*local*/, MemFs /*remote*/, broadcast::Receiver<Event>)`, que registra o site `"s"`, e `pub async fn wait_for<F: FnMut(&Event) -> bool>(rx: &mut Receiver<Event>, f: F) -> Event` com timeout de 5 s (panic no timeout).

- [ ] **Step 1: Testes (falhando)** em `engine_files.rs`:
  - `uploads_single_file`: `local` com `/w/a.bin` (300 KB) → `enqueue(Upload, local=/w/a.bin, remote=/srv/a.bin)` → `wait_for StateChanged{Done}`. `remote.read_file("/srv/a.bin")` é igual à origem.
  - `downloads_single_file`: o espelho, com `Download`.
  - `respects_worker_limit`: `workers = 2`, 6 arquivos, `FaultFs` com latência de 20 ms no remoto. Nunca há mais de 2 itens `Active` simultâneos (contar pelos eventos `StateChanged`).
  - `progress_is_throttled`: arquivo de 8 MiB, com `buf_size` de 4 KiB e latência no `FaultFs`. O número de eventos `Progress` é ≤ (duração / 100 ms) + 2.
  - `missing_source_fails_only_that_item` **(Review Focus 3)**: 3 arquivos, um deles inexistente → aquele item termina `Failed(kind NotFound)` e os outros dois `Done`.
  - `read_error_mid_transfer_fails_item`: `fail_io_after(50_000, Io)` → o item termina `Failed(kind Io)` e o próximo item da fila termina `Done`.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-transfer --test engine_files`
- [ ] **Step 3: Implementar `engine/mod.rs` (handle + tipos), `actor.rs` (loop `tokio::select!` sobre comandos, resultados de workers e ticker) e `worker.rs` (fluxo de arquivo)**
- [ ] **Step 4: Rodar e ver passar.** Run: o mesmo comando, mais `cargo test -p hoprs-transfer`.
- [ ] **Step 5: Commit.** `git commit -m "feat(transfer): engine actor with file transfers"`

---

### Task 10: Expansão de pastas

**Files:**
- Modify: `crates/hoprs-transfer/src/engine/worker.rs`, `engine/actor.rs`
- Test: `crates/hoprs-transfer/tests/engine_dirs.rs`

**Interfaces:**
- Consumes: Task 9.
- Produces: o comportamento de pasta (sem mudar a API pública):
  - Quando o `stat` da origem é `Dir`, o worker faz `mkdir` no destino e `list` na origem, e devolve a lista ao actor, que insere um item filho por entrada (`parent = Some(id)`, caminhos via `join(name)`). Symlink vira um filho `Skipped("symlinks are not followed")`. Os filhos entram na fila normalmente, e subpastas são expandidas quando um worker as pega (expansão preguiçosa). O worker da pasta é liberado logo após listar.
  - Uma pasta fica `Active` enquanto tiver filhos não terminais. Quando o último filho termina, ela vira `Done` se todos os filhos estão `Done | Skipped`, ou `Failed(Error::new(Io, "N items failed"))` se algum falhou. Essa regra sobe recursivamente.
  - `Event::Added` é emitido para cada filho.

- [ ] **Step 1: Testes (falhando)** em `engine_dirs.rs`:
  - `uploads_tree`: `/w/site/{index.html, .env, "relatório final.pdf", css/app.css, img/}` (com `img/` vazia) → todos os arquivos com conteúdo idêntico sob `/srv/site/...`, `/srv/site/img` existe **(Review Focus 2 e 4)**, e a raiz termina `Done`.
  - `symlink_is_skipped`: `/w/site/link` é um symlink → filho `Skipped("symlinks are not followed")`, a raiz termina `Done` **(Review Focus 4)**.
  - `child_failure_fails_parent`: `deny_writes_under("/srv/site/css")` → `app.css` termina `Failed(Permission)`, e a raiz termina `Failed` com a mensagem `"1 items failed"`. Os outros arquivos terminam `Done`.
  - `large_tree_is_lazy`: 5.000 arquivos em 50 pastas, `workers = 4`. O primeiro `StateChanged{Done}` de um arquivo chega antes de todos os `Added` (mostra que não houve listagem completa antecipada), e tudo termina `Done`.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-transfer --test engine_dirs`
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(transfer): lazy directory expansion"`

---

### Task 11: Controle — pausar, retomar, cancelar, tentar de novo e conflitos

**Files:**
- Modify: `crates/hoprs-transfer/src/engine/actor.rs`, `engine/worker.rs`, `crates/hoprs-remote/src/fault.rs` (`pub fn read_offsets(&self) -> Vec<u64>`, os offsets de cada `open_read`)
- Test: `crates/hoprs-transfer/tests/engine_control.rs`

**Interfaces:**
- Consumes: Tasks 9–10.
- Produces: a semântica dos comandos:
  - `pause(id)`: item `Queued` → `Paused`. Item `Active` → cancela o token do worker e o resultado `Cancelled` vira `Paused`. Pasta → aplica a todos os descendentes não terminais.
  - `resume(id)`: `Paused` → `Queued`. Na execução, um arquivo pausado sempre usa `ConflictPolicy::Resume` para o destino, mesmo que a política global seja outra (é a continuação do próprio item).
  - `cancel(id)`: qualquer não terminal → `Cancelled` (pasta: em cascata). O arquivo parcial no destino **não** é apagado.
  - `retry(id)`: `Failed | Cancelled` → `Queued` (pasta: os filhos `Failed | Cancelled` também). Um id não terminal → `Err(Conflict)`.
  - `Decision::Ask` → item `AwaitingDecision(ConflictInfo)`, e o worker é liberado. `resolve(id, Resolution)`: `Overwrite` → reexecuta com `Write{0}`, `Resume` → com `decide(Resume, ..)`, `Skip` → `Skipped("skipped by user")`. Com `apply_to_all = true`, define a política global equivalente (`Overwrite`/`Resume`/`Skip`) e resolve **todos** os itens em `AwaitingDecision` da mesma forma.
  - Comandos em um id inexistente → `Err(Error::new(NotFound, "Unknown transfer"))`.

- [ ] **Step 1: Testes (falhando)** em `engine_control.rs`:
  - `pause_and_resume_continues_from_offset`: arquivo de 4 MiB com latência. Pausar depois do primeiro `Progress` → `Paused`. `resume` → `Done`, e o `FaultFs` do lado da origem registra um `open_read` com offset > 0 (expor no `FaultFs` uma lista `read_offsets()` de offsets abertos, acrescentada nesta task).
  - `cancel_cascades_in_folder`: pasta com 20 arquivos lentos, `cancel(raiz)` → a raiz e todos os filhos não terminais terminam `Cancelled`.
  - `retry_failed`: `fail_next(OpenWrite, Permission)` → `Failed`. `retry` → `Done`.
  - `ask_then_resolve_overwrite`: o destino existe com conteúdo diferente e a política é `Ask` → `AwaitingDecision`. `resolve(Overwrite, false)` → `Done`, com conteúdo igual à origem.
  - `apply_to_all_resolves_pending`: 3 conflitos em `AwaitingDecision`. `resolve(primeiro, Skip, apply_to_all=true)` → os 3 terminam `Skipped`, e um 4º conflito enfileirado depois também é `Skipped` sem perguntar.
  - `cancel_while_awaiting_decision`: `AwaitingDecision` → `cancel` → `Cancelled`.
  - `unknown_id_errors`: `pause(TransferId(999))` → `Err` com `kind == NotFound`.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-transfer --test engine_control`
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar.** Run: `cargo test -p hoprs-transfer`
- [ ] **Step 5: Commit.** `git commit -m "feat(transfer): pause/resume/cancel/retry and conflict resolution"`

---

### Task 12: Falha de rede, pausa do site e recuperação após reinício

**Files:**
- Modify: `crates/hoprs-transfer/src/engine/actor.rs`, `engine/mod.rs` (`TransferEngine::start` passa a chamar `store.recover()`)
- Test: `crates/hoprs-transfer/tests/engine_recovery.rs`

**Interfaces:**
- Consumes: Tasks 7, 9–11.
- Produces:
  - Erro com `kind == Network` num item: o item volta para `Queued` (sem `Failed`), o site é marcado como pausado, o motor emite `Event::SitePaused{site, error}` e nenhum outro item do site é despachado até `resume_site(site)`. Os itens `Active` do mesmo site que falharem também por `Network` voltam para `Queued` **sem** emitir um segundo `SitePaused`. Ao retomar, eles continuam do offset do destino (política `Resume`, igual à Task 11).
  - `pause_site`/`resume_site` manuais usam o mesmo mecanismo (é o gancho que a reconexão do Plano 2 vai chamar).
  - `TransferEngine::start` chama `store.recover()`. Os itens recuperados ficam `Paused` e aparecem em `snapshot()`. Nada é retomado automaticamente.

- [ ] **Step 1: Testes (falhando)** em `engine_recovery.rs`:
  - `network_error_pauses_site`: 3 arquivos, `fail_io_after(200_000, Network)` → exatamente um `SitePaused`, nenhum item `Failed`, e nenhum novo `Active` em 300 ms. `resume_site` → os 3 terminam `Done`, com conteúdo idêntico à origem.
  - `other_sites_keep_running`: dois sites, falha de rede no site A → o site B termina tudo `Done` enquanto A está pausado.
  - `restart_recovers_as_paused` **(Review Focus 5)**: store em arquivo (`tempfile`). Motor 1 com um item `Active` (lento) e outro em `AwaitingDecision`. Derrubar o motor (`drop` + `abort` da task, simulando o fechamento do app) e iniciar o motor 2 com `QueueStore::open` no mesmo arquivo → os dois itens estão `Paused` em `snapshot()`. `resume` do primeiro → `Done`, retomando de offset > 0. `resume` do segundo → volta para `AwaitingDecision`.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-transfer --test engine_recovery`
- [ ] **Step 3: Implementar.** Para permitir o teste de reinício, `TransferEngine` ganha `pub async fn shutdown(self)`, que para o actor e aguarda a task terminar, sem alterar estados no store.
- [ ] **Step 4: Rodar e ver passar.** Run: `cargo test --workspace --all-features && cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] **Step 5: Commit.** `git commit -m "feat(transfer): network-failure site pause and restart recovery"`

---

### Task 13: Benchmark do motor

**Files:**
- Create: `crates/hoprs-transfer/benches/engine.rs` (`[[bench]] harness = false`)
- Modify: `.github/workflows/ci.yml` (job `bench`, executa `cargo bench -p hoprs-transfer -- --quick` sem gate, só para garantir que compila e roda)

**Interfaces:**
- Consumes: `TransferEngine`, `MemFs`.

- [ ] **Step 1: Escrever dois benchmarks criterion:** `upload_256mib_single_file` (throughput em bytes, `Throughput::Bytes`) e `upload_10k_small_files` (10.000 arquivos de 4 KiB, `Throughput::Elements`), ambos `MemFs` → `MemFs` com 4 workers.
- [ ] **Step 2: Rodar.** Run: `cargo bench -p hoprs-transfer -- --quick`. Expected: os dois benchmarks rodam e imprimem throughput. Registrar os números no corpo do commit como linha de base.
- [ ] **Step 3: Commit.** `git commit -m "bench(transfer): engine throughput baseline"`

---

## Critério de conclusão do Plano 1

- `cargo test --workspace --all-features` verde no Ubuntu e no Windows. A CI está verde no GitHub.
- Todos os itens da Review Focus têm teste passando.
- Benchmarks com linha de base registrada.
- Próximo passo: escrever o Plano 2 (SSH + SFTP) usando as assinaturas reais de `RemoteFs`, `TransferEngine::register_site`, `pause_site` e `resume_site`.
