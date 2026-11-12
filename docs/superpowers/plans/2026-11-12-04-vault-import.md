# Plano 4: Vault + import do FileZilla — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Guardar sites e workspaces de forma durável e segura, e trazer os sites do FileZilla. Entregas: modelo `Site`/`Workspace` em `hoprs-core`; `hoprs-vault` com perfis TOML sem segredos, segredos no keyring, export/import cifrado e gravação do pin FTPS; `hoprs-import` com o parser do `sitemanager.xml` e o relatório de importação.

**Architecture:**
- **`hoprs-core`** ganha o modelo de domínio puro (`Site`, `Protocol`, `Workspace`), serializável com serde. Ele não depende de `hoprs-ssh` nem de `hoprs-remote`, e é o app (Plano 5) que traduz um `Site` para `SshConfig`/`FtpConfig`.
- **`hoprs-vault`** é dono dos arquivos `sites.toml`/`workspaces.toml`, com gravação atômica e versão de esquema. Os segredos passam pelo trait `SecretStore`, que tem duas implementações: `KeyringStore` (Secret Service) e `MemoryStore` (testes). O export é um arquivo único cifrado com Argon2id + XChaCha20-Poly1305.
- **`hoprs-import`** só lê o XML do FileZilla e escreve no vault pela API pública dele.

**Tech Stack:** serde 1, toml 1, uuid 1 (v4), dirs 7, keyring 4 (Secret Service via zbus; o mock de testes do keyring-core), secrecy 0.10, zeroize 1, argon2 0.6, chacha20poly1305 0.11 (XChaCha20Poly1305), rand (via `getrandom`/`OsRng`), quick-xml 0.42, base64 0.22, tempfile (dev).

**Spec:** [`docs/superpowers/specs/2026-11-11-hoprs-design.md`](../specs/2026-11-11-hoprs-design.md) §3, §4.1, §6, §7 (segredos), §10, §11 (fixtures do import). Roadmap: [`2026-11-11-00-roadmap.md`](2026-11-11-00-roadmap.md). Pendências: [`2026-11-12-01-followups.md`](2026-11-12-01-followups.md).

## Global Constraints

- **Grafo (spec §4.1):** `hoprs-vault` → `hoprs-core`; `hoprs-import` → `hoprs-vault`, `hoprs-core`. Nenhum dos dois depende de `hoprs-ssh`, `hoprs-remote`, `hoprs-transfer` ou `iced`.
- **Perfis:** ficam em `$XDG_CONFIG_HOME/hoprs/sites.toml` e `workspaces.toml`, via `dirs::config_dir()`. Eles **nunca contêm segredos**.
- **Segredos:** a senha e a passphrase de chave ficam no keyring do sistema, no serviço `hoprs/<site-id>`, com as contas `password` e `key-passphrase`. Em memória ficam só em `SecretString`.
- **Export/import cifrado:** um arquivo único com perfis, workspaces e segredos, cifrado com senha (Argon2id + XChaCha20-Poly1305).
- **Import do FileZilla:**
  - lê `~/.config/filezilla/sitemanager.xml`;
  - converte pastas em workspaces, opcionalmente, conforme a escolha passada pelo chamador;
  - traz protocolos, portas, usuários, modo passivo/ativo e pastas iniciais;
  - manda as senhas em base64 direto para o keyring;
  - produz um relatório com os importados, os ignorados e o motivo de cada um;
  - permite apagar o XML original, só por chamada explícita do app.
- **Nenhum log, `Debug`, erro, TOML ou relatório contém senha, passphrase ou o conteúdo decifrado do export.**
- `cargo fmt --all --check` e `cargo clippy --workspace --all-targets --all-features -- -D warnings` limpos em todo commit. Código e mensagens em inglês.
- Testes portáveis: `tempfile`, nunca `/tmp` literal. Nenhum teste toca o keyring real nem o `~/.config` real; os testes usam o mock do keyring e diretórios temporários.

## Review Focus

1. **`sites.toml` corrompido ou editado à mão com erro:** `Vault::open` falha com erro claro (`Protocol`, com arquivo e linha no detail). O arquivo **não é sobrescrito**, e nada é resetado em silêncio. Campos desconhecidos são ignorados e preservados na próxima gravação, por compatibilidade com versões futuras. → testes na Task 2.
2. **Queda no meio de uma gravação:** a escrita é atômica (arquivo temporário no mesmo diretório, `fsync` e `rename`), então o arquivo antigo continua íntegro. Uma versão de esquema maior que a conhecida é recusada. → testes na Task 2.
3. **Senha de export errada ou arquivo adulterado:** o resultado é o erro `Auth` com a mensagem "Wrong password or damaged file". Nada é importado e o vault fica inalterado. → testes na Task 4.
4. **Variações do XML do FileZilla:**
   - senhas cifradas com senha mestra (`encoding="crypt"`): o site entra sem senha e o relatório diz por quê;
   - nomes com acentos, pastas aninhadas, protocolo desconhecido (ignorado, com motivo), `Port` ausente (porta padrão do protocolo) e `RemoteDir` no formato codificado do FileZilla;
   - XML inválido: erro claro, sem import parcial.

   → testes na Task 5.
5. **Keyring indisponível** (sem Secret Service, por exemplo numa sessão sem D-Bus): gravar um segredo dá erro claro (`Io`, "System keyring unavailable"), o perfil continua salvo e nada entra em pânico. Remover um site apaga os segredos dele e o tira de todos os workspaces. → testes na Task 3.

---

## Estrutura de arquivos

```
crates/hoprs-core/src/site.rs           # Site, Protocol, SshOptions, FtpOptions, FtpMode, OpenAs
crates/hoprs-core/src/workspace.rs      # Workspace, WorkspaceSite, SavedTab, TabKind
crates/hoprs-core/src/ids.rs            # SiteId::new(), WorkspaceId (modificado)
crates/hoprs-vault/src/
  lib.rs                                # Vault (fachada), re-exports
  profiles.rs                           # leitura/gravação de sites.toml e workspaces.toml
  atomic.rs                             # gravação atômica
  secrets.rs                            # SecretKind, trait SecretStore, KeyringStore, MemoryStore
  export.rs                             # formato do arquivo cifrado, export/import
crates/hoprs-import/src/
  lib.rs                                # re-exports
  filezilla.rs                          # parse do sitemanager.xml → ParsedSite/ParsedFolder
  apply.rs                              # aplica no Vault → ImportReport
crates/hoprs-import/tests/fixtures/     # sitemanager-*.xml de versões diferentes do FileZilla
```

---

### Task 1: Modelo de domínio em `hoprs-core`

**Files:**
- Create: `crates/hoprs-core/src/site.rs`, `crates/hoprs-core/src/workspace.rs`
- Modify: `crates/hoprs-core/src/ids.rs`, `crates/hoprs-core/src/lib.rs`, `crates/hoprs-core/Cargo.toml` (serde com `derive`, uuid com `v4`), `Cargo.toml` raiz (dependências do workspace deste plano)

**Interfaces:**
- **IDs (`ids.rs`):**
  - `SiteId::new() -> SiteId` gera um UUID v4 em hex com hífens. O `SiteId(pub String)` existente continua igual e ganha `Serialize`/`Deserialize` transparente (`#[serde(transparent)]`).
  - `pub struct WorkspaceId(pub String)` tem os mesmos derives e `WorkspaceId::new()`.
- **`site.rs`:**
  - `pub enum Protocol { Ssh, Ftp, FtpsExplicit, FtpsImplicit }`. Serializa em kebab-case (`ssh`, `ftp`, `ftps-explicit`, `ftps-implicit`). `default_port()` dá 22, 21, 21 e 990. `label()` dá `"ssh"`, `"ftp (unencrypted)"`, `"ftps (explicit)"` e `"ftps (implicit)"`.
  - `pub enum OpenAs { Terminal, Files, Both }` (kebab-case), com padrão `Both`. Só vale para `Ssh`.
  - `pub struct SshOptions { pub use_agent: bool, pub key_file: Option<PathBuf>, pub open_as: OpenAs }`, com padrão `true`, `None`, `Both`.
  - `pub enum FtpMode { Passive, Active }`, com padrão `Passive`.
  - `pub struct FtpOptions { pub mode: FtpMode, pub nat_workaround: bool, pub pinned_fingerprint: Option<String> }`, com padrão `Passive`, `true`, `None`.
  - `pub struct Site { pub id: SiteId, pub name: String, pub host: String, pub port: u16, pub user: String, pub protocol: Protocol, pub ssh: SshOptions, pub ftp: FtpOptions, pub remote_dir: Option<String>, pub local_dir: Option<PathBuf>, pub has_password: bool, pub has_key_passphrase: bool }`.
    - Os dois `has_*` só indicam que existe um segredo no keyring. O segredo em si nunca fica no `Site`.
    - `Site::new(name, host, user, protocol)` gera um id novo, usa `protocol.default_port()` e as opções padrão.
- **`workspace.rs`:**
  - `pub enum TabKind { Terminal, Files }`.
  - `pub struct SavedTab { pub kind: TabKind, pub remote_dir: Option<String>, pub local_dir: Option<PathBuf> }`.
  - `pub struct WorkspaceSite { pub site: SiteId, pub tabs: Vec<SavedTab> }`.
  - `pub struct Workspace { pub id: WorkspaceId, pub name: String, pub sites: Vec<WorkspaceSite> }`.
- **Derives:** todos os tipos derivam `Clone, Debug, PartialEq, Serialize, Deserialize`. Com `#[serde(default)]` nos campos opcionais, um TOML antigo sem um campo novo ainda carrega.

- [ ] **Step 1: Testes (falhando)** em `site.rs`/`workspace.rs`:
  - `protocol_serializes_kebab_case`: `toml::to_string` de um struct com `Protocol::FtpsExplicit` contém `"ftps-explicit"`, e o caminho inverso funciona.
  - `default_ports_and_labels`: os quatro protocolos dão os valores acima.
  - `site_new_uses_default_port_and_options`
  - `site_ids_are_unique`: 1000 chamadas a `SiteId::new()` sem repetição.
  - `missing_optional_fields_use_defaults`: um TOML de site só com `id`, `name`, `host`, `port`, `user` e `protocol` desserializa com `ssh`/`ftp` no padrão e `has_password = false`.
  - `workspace_roundtrip`: um workspace com dois sites e abas sai igual em TOML de ida e volta.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-core`
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar.** Run: `cargo test --workspace --all-features`
- [ ] **Step 5: Commit.** `git commit -m "feat(core): Site and Workspace domain model"`

---

### Task 2: `hoprs-vault`: perfis em TOML com gravação atômica

**Files:**
- Create: `crates/hoprs-vault/Cargo.toml`, `src/lib.rs`, `src/profiles.rs`, `src/atomic.rs`

**Interfaces:**
- **`atomic.rs`:** `pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()>`.
  - Grava num temporário no mesmo diretório (nome único), faz `sync_all` e `rename`. No Unix, faz também `fsync` do diretório.
  - Cria o diretório pai se faltar, com `0o700` no Unix. O arquivo fica com `0o600` no Unix.
  - Em erro, remove o temporário.
- **`profiles.rs`:**
  - `pub(crate) struct Profiles { pub sites: Vec<Site>, pub workspaces: Vec<Workspace> }`.
  - `pub(crate) fn load(dir: &Path) -> Result<Profiles>`:
    - Um arquivo ausente vale lista vazia.
    - Cada arquivo tem `version = 1` no topo, com `[[site]]` em um e `[[workspace]]` no outro. Uma versão maior que 1 dá `Error(Protocol, "Profile was written by a newer hoprs")`.
    - Um TOML inválido dá `Error(Protocol, "Cannot read <arquivo>")` com a mensagem do parser (linha e coluna) no detail.
    - Ids duplicados também dão `Protocol`.
  - `pub(crate) fn save_sites(dir, &Profiles) -> Result<()>` e `save_workspaces(dir, &Profiles) -> Result<()>` gravam com `write_atomic`.
  - **Campos desconhecidos:** ficam preservados. Cada `Site`/`Workspace` carregado guarda o resto da tabela TOML original, num `toml::Table` mantido em `Profiles` por id e não em core. Na gravação, esse resto volta junto. O motivo é que uma versão nova do app pode ter escrito o campo.
- **`lib.rs`:**
  - `pub fn default_dir() -> Option<PathBuf>` (`dirs::config_dir()` + `hoprs`).
  - Nesta task, a parte de perfis do `Vault` (os segredos entram na Task 3). Em `pub struct Vault`:
    - `open_profiles_only(dir: &Path) -> Result<Vault>`, construtor temporário que a Task 3 substitui por `open`.
    - `sites() -> &[Site]`, `site(&SiteId) -> Option<&Site>`.
    - `upsert_site(Site) -> Result<()>`: insere, ou substitui pelo id.
    - `workspaces() -> &[Workspace]`, `upsert_workspace(Workspace) -> Result<()>`, `remove_workspace(&WorkspaceId) -> Result<()>`.
    - Cada mutação grava na hora. Uma gravação que falha deixa o estado em memória igual ao do disco, porque a mudança é aplicada numa cópia e só substitui o estado depois do `Ok`.
    - `upsert_workspace` que referencia um `SiteId` inexistente dá `Error(Protocol, "Unknown site in workspace")`.

- [ ] **Step 1: Testes (falhando)**, todos com `TempDir`:
  - `empty_dir_loads_empty`
  - `roundtrip_sites_and_workspaces`: grava, abre de novo e compara.
  - `corrupt_sites_toml_is_protocol_and_untouched` **(Review Focus 1)**: escreve `"[[site]\nname ="` à mão. `open` dá `Protocol` com "line" no detail, e os bytes do arquivo não mudam.
  - `newer_version_is_refused` **(Review Focus 2)**: `version = 2` dá `Protocol`.
  - `unknown_fields_survive_a_save` **(Review Focus 1)**: um site com `future_field = "x"`. Depois de abrir, fazer `upsert_site` de outro site e reabrir, o arquivo ainda tem `future_field = "x"`.
  - `failed_save_leaves_memory_and_disk_unchanged` **(Review Focus 2)**: o diretório fica somente leitura no Unix (`#[cfg(unix)]`). `upsert_site` dá erro, `sites()` continua como antes e o arquivo continua íntegro.
  - `atomic_write_never_leaves_partial_file`: depois de um `write_atomic` com erro forçado no rename (o destino é um diretório), o arquivo original fica intacto e não sobra temporário.
  - `duplicate_ids_are_protocol`, `workspace_with_unknown_site_is_rejected`
  - `file_modes_are_private` (`#[cfg(unix)]`): `0o600` no arquivo e `0o700` no diretório criado.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-vault`
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(vault): TOML profiles with atomic writes"`

---

### Task 3: Segredos no keyring e a fachada `Vault`

**Files:**
- Create: `crates/hoprs-vault/src/secrets.rs`
- Modify: `crates/hoprs-vault/src/lib.rs`, `Cargo.toml` (keyring 4)

**Interfaces:**
- **`secrets.rs`:**
  - `pub enum SecretKind { Password, KeyPassphrase }`. `account()` dá `"password"` e `"key-passphrase"`.
  - `pub trait SecretStore: Send + Sync`, com:
    - `fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>>`
    - `fn set(&self, site: &SiteId, kind: SecretKind, value: &SecretString) -> Result<()>`
    - `fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()>`. Apagar algo que não existe dá `Ok`.
  - O serviço é `format!("hoprs/{}", site)`.
  - `pub struct KeyringStore`, em cima do crate `keyring` v4. Se o backend falhar ao se conectar (sem Secret Service ou sem D-Bus), o erro é `Error(Io, "System keyring unavailable")` com o erro do backend no detail.
  - `pub struct MemoryStore`, em memória (`Mutex<HashMap>`), para os testes e para o import em dry-run.
- **`Vault`:**
  - `pub fn open(dir: &Path, secrets: Arc<dyn SecretStore>) -> Result<Vault>` substitui `open_profiles_only`.
  - `set_secret(&mut self, site: &SiteId, kind, value: &SecretString) -> Result<()>`: grava no store, marca `has_password`/`has_key_passphrase = true` no `Site` e salva o perfil.
    - Se o store falhar, o flag não muda e o erro volta.
    - Se o store gravar e o perfil não, desfaz o segredo com best-effort `delete` e devolve o erro.
  - `secret(&self, site, kind) -> Result<Option<SecretString>>`.
  - `delete_secret(&mut self, site, kind) -> Result<()>` apaga o segredo e limpa o flag.
  - `remove_site(&mut self, id: &SiteId) -> Result<()>`:
    - apaga os dois segredos (falha do keyring aqui não impede a remoção; o erro volta depois, como `Io`, com o perfil já salvo);
    - remove o site de todos os workspaces;
    - grava os dois arquivos.
  - `set_ftps_pin(&mut self, site: &SiteId, fingerprint: &str) -> Result<()>` grava `ftp.pinned_fingerprint`. É o que o app chama dentro de `FtpPrompter::store_pin`. `clear_ftps_pin(&mut self, site)` existe para o caso do pin velho (pendência R5 do Plano 3).
- O `Debug` do `Vault` não imprime segredos (o `MemoryStore` não implementa `Debug` com os valores).

- [ ] **Step 1: Testes (falhando).** Com `MemoryStore`, e para `KeyringStore` usando o store mock do keyring-core (`keyring_core::set_default_store(...)`, ou o equivalente na API do v4; confira na fonte):
  - `secret_roundtrip_sets_flag`
  - `delete_secret_clears_flag`
  - `remove_site_deletes_secrets_and_workspace_refs` **(Review Focus 5)**
  - `keyring_failure_leaves_flag_unset` **(Review Focus 5)**: um `SecretStore` de teste que sempre falha com "System keyring unavailable". `set_secret` dá `Io`, `has_password` continua `false` e o perfil continua carregável.
  - `profile_save_failure_rolls_back_secret`: o diretório fica somente leitura e o segredo gravado no store é desfeito.
  - `keyring_store_uses_service_and_account`: com o mock, grava e lê de volta por `hoprs/<id>` + `password`.
  - `set_and_clear_ftps_pin`
  - `no_secret_in_toml_or_debug`: depois de `set_secret("hunter2")`, nem os arquivos TOML nem `format!("{vault:?}")` contêm `hunter2`.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-vault`
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(vault): keyring secrets and Vault facade"`

---

### Task 4: Export/import cifrado

**Files:**
- Create: `crates/hoprs-vault/src/export.rs`
- Modify: `crates/hoprs-vault/src/lib.rs`, `Cargo.toml` (argon2, chacha20poly1305)

**Interfaces:**
- **Formato do arquivo**, em binário e ordem fixa:
  - magic `b"HOPRSVLT"`;
  - `u16` de versão = 1;
  - os parâmetros do Argon2id (`m_cost` KiB `u32`, `t_cost` `u32`, `p_cost` `u32`);
  - salt de 16 bytes, aleatório;
  - nonce de 24 bytes, aleatório;
  - o ciphertext XChaCha20-Poly1305 de um JSON UTF-8 `{ "sites": [...], "workspaces": [...], "secrets": [{ "site": id, "kind": "password", "value": "..." }] }`.

  O cabeçalho inteiro (magic até o nonce) entra como AAD. A chave de 32 bytes sai do Argon2id com os parâmetros do cabeçalho. Os padrões são `m_cost = 65536` (64 MiB), `t_cost = 3` e `p_cost = 1`; os testes usam valores baixos via `ExportParams`. O JSON em claro fica em `Zeroizing<Vec<u8>>`.
- `pub struct ExportParams { pub m_cost: u32, pub t_cost: u32, pub p_cost: u32 }` (`Default` = os padrões acima).
- `Vault::export(&self, path: &Path, password: &SecretString, params: ExportParams) -> Result<()>` inclui todos os segredos existentes (lidos do store) e grava com `write_atomic`.
- `pub enum ImportMode { Merge, Replace }`:
  - `Merge`: um id que já existe é mantido, ignorado e entra no relatório.
  - `Replace`: o vault passa a ser exatamente o conteúdo do arquivo.
- `pub struct BackupImportReport { pub added_sites: usize, pub skipped_sites: Vec<String>, pub added_workspaces: usize, pub skipped_workspaces: Vec<String>, pub secrets_restored: usize, pub secret_errors: Vec<String> }`.
- `Vault::import_backup(&mut self, path: &Path, password: &SecretString, mode: ImportMode) -> Result<BackupImportReport>`:
  - Decifra tudo antes de mudar qualquer coisa.
  - Uma falha de autenticação do AEAD dá `Error(Auth, "Wrong password or damaged file")`. Magic ou versão desconhecidos dão `Protocol`.
  - Aplica primeiro os perfis (uma gravação só por arquivo) e depois os segredos.
  - Se um segredo falhar no keyring, o import continua, o site fica sem o flag e o erro entra em `secret_errors`, com o nome do site e o motivo, sem o valor. Ninguém perde um backup inteiro porque o keyring caiu no meio.
  - Parâmetros do Argon2 acima de 1 GiB de memória dão `Protocol`, como proteção contra um arquivo malicioso que esgote a memória.

- [ ] **Step 1: Testes (falhando)**, com params baixos (`m_cost = 1024`, `t_cost = 1`):
  - `export_import_roundtrip_with_secrets`: um vault com 2 sites, 1 workspace e 2 segredos, exportado e importado num vault vazio, fica igual, segredos incluídos.
  - `wrong_password_is_auth_and_changes_nothing` **(Review Focus 3)**
  - `tampered_file_is_auth` **(Review Focus 3)**: mudar um byte do ciphertext dá `Auth`, e mudar um byte do cabeçalho também dá `Auth`, porque ele é AAD.
  - `unknown_magic_or_version_is_protocol`
  - `oversized_argon_params_are_refused`
  - `merge_keeps_existing_ids`, `replace_overwrites`
  - `export_has_no_plaintext`: os bytes do arquivo não contêm o nome do site nem o segredo.
  - `keyring_failure_during_import_is_reported_not_fatal`: com um store que falha, os sites entram sem flag e `secret_errors` tem uma entrada.
- [ ] **Step 2: Rodar e ver falhar**
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(vault): encrypted export and import"`

---

### Task 5: `hoprs-import`: parser do `sitemanager.xml`

**Files:**
- Create: `crates/hoprs-import/Cargo.toml`, `src/lib.rs`, `src/filezilla.rs`, `tests/fixtures/{sitemanager-3.66.xml, sitemanager-3.40-folders.xml, sitemanager-crypt.xml, sitemanager-broken.xml}` (escritos à mão no formato real do FileZilla; o README da pasta diz que os dados são fictícios)

**Interfaces:**
- `pub fn default_sitemanager_path() -> Option<PathBuf>` (`dirs::config_dir()` + `filezilla/sitemanager.xml`).
- `pub fn parse_sitemanager(xml: &str) -> Result<ParsedSiteManager>`. Um XML inválido dá `Error(Protocol, "Cannot read sitemanager.xml")` com linha e coluna no detail, e o retorno é tudo ou nada.
- `pub struct ParsedSiteManager { pub entries: Vec<ParsedEntry> }`.
- `pub enum ParsedEntry { Site(ParsedSite), Folder { name: String, entries: Vec<ParsedEntry> } }`.
- `pub struct ParsedSite`, com os campos:
  - `name: String`, `host: String`, `port: Option<u16>`;
  - `protocol: Result<Protocol, String>`, em que `Err` é o motivo de ignorar;
  - `user: String`;
  - `password: ParsedPassword`;
  - `key_file: Option<PathBuf>`;
  - `remote_dir: Option<String>`, `local_dir: Option<PathBuf>`;
  - `mode: Option<FtpMode>`;
  - `notes: Vec<String>`.
- `pub enum ParsedPassword { None, Plain(SecretString), Encrypted, AskEachTime }`.
- **Mapeamento** (tabela do FileZilla 3.x):
  - **`<Protocol>`:**
    - `0` (FTP com "explicit TLS if available") → `FtpsExplicit`, com uma nota: "FileZilla used TLS only if available; if the server has no TLS, edit the site to ftp".
    - `1` → `Ssh`. `3` → `FtpsImplicit`. `4` → `FtpsExplicit`. `6` → `Ftp`.
    - Qualquer outro valor (S3, WebDAV, Storj, Google, Azure etc.) → `Err("unsupported protocol <n>")`.
  - **`<Logontype>`:**
    - `0` (anônimo) → user `anonymous`, sem senha.
    - `1`/`4` → senha de `<Pass>`.
    - `2` (perguntar) e `3` (interativo) → `AskEachTime`.
    - `5` (key file) → `key_file` de `<Keyfile>`, sem senha.
  - **`<Pass encoding="base64">`** → `Plain`, decodificado em base64 e depois UTF-8; um base64 inválido vira nota e `None`. Com `encoding="crypt"` → `Encrypted`, e a nota "password is protected by a FileZilla master password; enter it in hoprs".
  - **`<PasvMode>`:** `MODE_ACTIVE` → `Active`, `MODE_PASSIVE` → `Passive`, `MODE_DEFAULT` → `None`.
  - **`<RemoteDir>`:** está no formato codificado do FileZilla, `"1 0 4 home 5 user"` (tipo 1 = unix, seguido de pares tamanho/segmento). Vira `"/home/user"`. Um tipo diferente de 1 vira nota e `None`.
  - **`<LocalDir>`** passa sem mudança.
  - O nome vem do texto do `<Server>` (a última linha de texto, como no FileZilla) ou de `<Name>`.

- [ ] **Step 1: Testes (falhando)** em `tests/parse.rs`, com as fixtures:
  - `parses_all_protocols`: 0, 1, 3, 4 e 6 mapeados conforme a tabela, com a nota do `0`.
  - `unknown_protocol_is_skipped_with_reason` **(Review Focus 4)**
  - `base64_password_decoded`
  - `crypt_password_is_encrypted_with_note` **(Review Focus 4)**
  - `ask_and_interactive_logons`, `anonymous_logon`, `key_file_logon`
  - `missing_port_is_none`
  - `pasv_modes`
  - `remote_dir_decoding`: `"1 0 4 home 5 user"` → `/home/user`; `"1 0 11 relatório 1 x"` (tamanhos em bytes UTF-8) → `/relatório/x`; formato inválido → nota.
  - `nested_folders_keep_structure` **(Review Focus 4)**
  - `accented_names_survive`
  - `broken_xml_is_protocol_and_all_or_nothing` **(Review Focus 4)**
  - `debug_has_no_password`: `format!("{:?}", parsed)` não contém a senha decodificada.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-import`
- [ ] **Step 3: Implementar** com `quick-xml` (leitor de eventos; sem DTD, sem entidades externas).
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(import): FileZilla sitemanager.xml parser"`

---

### Task 6: Aplicar o import no vault, relatório e documentação

**Files:**
- Create: `crates/hoprs-import/src/apply.rs`, `crates/hoprs-import/tests/apply.rs`
- Modify: `crates/hoprs-import/src/lib.rs`, `docs/superpowers/plans/2026-11-12-01-followups.md`, `CLAUDE.md`

**Interfaces:**
- `pub struct ImportOptions { pub folders_as_workspaces: bool }`.
- `pub struct ImportReport { pub imported: Vec<ImportedSite>, pub skipped: Vec<SkippedSite>, pub workspaces_created: Vec<String> }`:
  - `ImportedSite { name: String, id: SiteId, notes: Vec<String> }`;
  - `SkippedSite { name: String, reason: String }`.
- `pub fn apply(vault: &mut Vault, parsed: ParsedSiteManager, options: &ImportOptions) -> Result<ImportReport>`:
  - **Cada site** suportado vira `Site::new(...)`, com a porta do XML ou a padrão do protocolo, `remote_dir`/`local_dir`, `ftp.mode` e `ssh.key_file`. O nome fica igual ao do FileZilla, prefixado com o caminho da pasta quando há pastas e `folders_as_workspaces = false` (por exemplo `Clientes/loja`).
  - **Senha `Plain`** vai para `vault.set_secret(id, Password, ..)`. Se o keyring falhar, o site entra assim mesmo e a nota "password not saved: <motivo>" vai para o relatório. A senha nunca aparece no relatório.
  - **Pastas**, com `folders_as_workspaces = true`: cada pasta, com nome igual ao caminho dela, vira um `Workspace` com os sites daquela pasta e de nenhuma outra (as subpastas viram workspaces próprios). Pastas vazias não viram workspace.
  - **Duplicados:** um site com o mesmo nome, host, porta, usuário e protocolo de um site que já existe é ignorado, com motivo "already in hoprs". Assim o import pode ser repetido sem duplicar.
  - **Ordem do relatório:** a mesma do XML.
- `pub fn delete_source(path: &Path) -> Result<()>` apaga o XML original. É chamado só quando o usuário confirma na UI (Plano 5). Antes de apagar, confere que o arquivo ainda é um XML do FileZilla (começa com `<?xml` e contém `<FileZilla3`); senão dá `Protocol`.

- [ ] **Step 1: Testes (falhando)** em `tests/apply.rs`, com `Vault::open(TempDir, MemoryStore)` e as fixtures da Task 5:
  - `imports_supported_sites_with_secrets`: a senha está no store, `has_password` é verdadeiro e a senha não está no relatório nem no TOML.
  - `folders_as_workspaces_creates_workspaces` e `folders_flattened_into_names`.
  - `skipped_sites_have_reasons`: protocolo não suportado.
  - `encrypted_passwords_import_without_secret_and_note`
  - `reimport_is_idempotent`: o segundo `apply` não cria nada e lista todos como "already in hoprs".
  - `keyring_failure_still_imports_site` **(Review Focus 5)**
  - `delete_source_only_deletes_filezilla_xml`: apaga a fixture copiada para um `TempDir` e recusa um arquivo qualquer.
- [ ] **Step 2: Rodar e ver falhar**
- [ ] **Step 3: Implementar e atualizar a documentação.**
  - Nos follow-ups, a pendência do pin velho (R5 do Plano 3) passa a citar `Vault::clear_ftps_pin`.
  - Nova entrada "Before Plan 5": o app chama `Vault::set_ftps_pin` dentro de `FtpPrompter::store_pin` e traduz `Site` para `SshConfig`/`FtpConfig`.
  - No `CLAUDE.md`, o status passa a ser "Plano 4 na branch, próximo passo merge e depois o Plano 5 (app iced)".
- [ ] **Step 4: Rodar e ver passar.** Run: `cargo test --workspace --all-features`
- [ ] **Step 5: Commit.** `git commit -m "feat(import): apply FileZilla import to the vault"` e `git commit -m "docs: Plan 4 status and follow-ups"`

---

## Critério de conclusão do Plano 4

- `cargo test --workspace --all-features` verde, sem Docker e sem keyring real (mock e `MemoryStore`).
- CI verde.
- Todos os itens da Review Focus têm teste passando.
- Os follow-ups e o `CLAUDE.md` estão atualizados.
- Próximo passo: escrever o Plano 5 (app iced: shell, barra lateral com workspaces/sites, abas, painel duplo, fila, diálogos de segurança), usando `Vault`, `SshSession`, `SftpFs`, `FtpFs`, `TransferEngine` e `spawn_site_watcher`.
