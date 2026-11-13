# Plano 6: Terminal, edição remota, paleta, log técnico e desempenho — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Completar a v1 do hoprs. Este plano entrega:
- a **aba de terminal** SSH (xterm-256color, truecolor, scrollback com busca, copiar e colar, resize propagado ao PTY e reconexão inline), na mesma conexão da aba de arquivos;
- a **edição remota** com `notify`;
- a **paleta Ctrl+K**;
- o **log técnico**;
- a correção das **quedas de conexão em ociosidade** relatadas pelo autor;
- os **benchmarks das metas da spec §9** num job próprio da CI.

**Architecture:**
- **Mesma arquitetura do Plano 5:** `update -> Vec<Effect>` puro, `effects.rs` como único chamador do `BackendApi` e eventos assíncronos chegando por `Subscription`.
- **Terminal em duas camadas:**
  - um **núcleo puro** (`hoprs-app/src/terminal/`), que envolve o `alacritty_terminal::Term`, codifica teclado e colagem em bytes, e cuida de seleção e busca. É testável sem janela.
  - um **widget iced** que desenha a grade de células.
- **PTY:** é um canal da sessão SSH já existente (spec §4.3), aberto pelo `SessionManager`.
- **Abas:** passam a ser **por sessão**, e não mais "uma aba por site". Um site SSH pode ter abas de arquivos e de terminal sobre uma conexão só.

**Tech Stack:**
- iced 0.14 (já em uso);
- `alacritty_terminal`: modelo VT (spec §2), na versão estável mais recente compatível com o MSRV do workspace;
- `notify` com debounce próprio de 500 ms;
- `tracing-subscriber`: uma `Layer` própria para o log em memória;
- `criterion`, que já é usado no `hoprs-transfer`;
- `tc netem` nos contêineres de benchmark;
- os crates do projeto.

O widget de terminal segue a abordagem do `iced_term`: renderiza a grade do `alacritty_terminal` com `canvas` ou `advanced::Widget`. Se o `iced_term` for compatível com iced 0.14 e tiver licença MIT/Apache, partes dele podem ser vendorizadas com atribuição. Se não for, o widget é escrito do zero.

**Spec:** [`docs/superpowers/specs/2026-11-11-hoprs-design.md`](../specs/2026-11-11-hoprs-design.md), seções:
- §3 (sessão com várias abas);
- §4.3 (PTY, keepalive);
- §5 (edição remota);
- §7;
- §8 (aba de terminal, Ctrl+K, log técnico);
- §9 (metas);
- §10;
- §11 (benchmarks com `tc netem` em job separado).

Referências: mockup [`assets/2026-11-11-hoprs-mockup-v2.html`](../specs/assets/2026-11-11-hoprs-mockup-v2.html) e pendências em [`2026-11-12-01-followups.md`](2026-11-12-01-followups.md) (Plano 2 "Before Plan 6", Plano 5 "Later").

## Global Constraints

- **Grafo de dependências (spec §4.1):** só `hoprs-app` depende de `iced`, `alacritty_terminal` e `notify`. Nenhum crate novo no workspace. Os benchmarks ficam em `benches/` dos crates existentes e em um job da CI.
- **Idioma e textos:** UI em inglês, com todas as strings visíveis em `crates/hoprs-app/src/ui/strings.rs`, em sentence case.
- **Visual (spec §8, mockup v2):**
  - destaque branco só em seleção, ação principal, aba ativa, foco e progresso;
  - JetBrains Mono no terminal e nos dados, Inter no resto;
  - cantos de 4 px, linhas de 1 px, nenhuma sombra;
  - nenhum modal fora de segurança e credenciais (a paleta é um overlay de navegação, não um diálogo de erro).
- **Terminal (spec §8):**
  - `TERM=xterm-256color`, com truecolor (`COLORTERM=truecolor` quando o servidor aceitar a variável);
  - scrollback padrão de **10 000 linhas** (configurável de 1 000 a 100 000);
  - busca no scrollback;
  - copiar e colar: **Ctrl+Shift+C / Ctrl+Shift+V**, com bracketed paste quando o programa remoto o ativa;
  - resize propagado ao PTY com debounce de **100 ms**;
  - fonte (tamanho de **10 a 24 px**, padrão **13**) e tema configuráveis;
  - faixa inline com "Reconnect" quando a conexão cai.
- **Edição remota (spec §5):**
  - baixa para `$XDG_RUNTIME_DIR/hoprs/edit/<session>/<path>` (fallback: o diretório de cache do usuário);
  - abre com o editor configurado (padrão `xdg-open`);
  - observa com `notify` e reenvia a cada gravação, com debounce de **500 ms**;
  - mostra um aviso discreto;
  - ao fechar a sessão, pergunta sobre pendências.
- **Keepalive (spec §4.3):** SSH a cada **30 s**, configurável. FTP com `NOOP` na conexão de controle ociosa.
- **A thread da UI nunca bloqueia:**
  - os bytes do PTY são aplicados ao `Term` em lotes por mensagem;
  - o `notify` roda fora da UI;
  - arquivo, vault e keyring sempre via effect ou `spawn_blocking`.
- **Logs:** nunca contêm senha, passphrase, conteúdo de arquivo ou **bytes do terminal**. O filtro padrão continua `info,suppaftp=warn`.
- `cargo fmt --all --check` e `cargo clippy --workspace --all-targets --all-features -- -D warnings` limpos. Código e identificadores em inglês.

## Review Focus

1. **Saída enorme ou lixo no terminal:** um `cat` de arquivo binário, `yes`, UTF-8 inválido ou sequências de escape malformadas.
   - A UI continua respondendo (outras abas, a fila e a digitação).
   - Nada entra em pânico.
   - A saída não se perde em silêncio. Se o canal transbordar, a aba mostra isso inline, sem fechar escondido.
   - → testes na Task 3 (núcleo) e na Task 4 (lotes e backpressure).
2. **Arquivo remoto alterado por outra pessoa durante a edição:** entre o download e a gravação local, o servidor muda o arquivo (tamanho ou mtime diferentes).
   - O reenvio **não** sobrescreve calado. Aparece um aviso inline com "Overwrite", "Keep remote" e "Save a copy".
   - → teste na Task 7.
3. **Conexão que cai em ociosidade** (o relato do autor):
   - Um site parado por mais tempo que o timeout ocioso do servidor, ou de um NAT, volta sozinho, sem precisar de clique.
   - A próxima ação na aba de arquivos funciona.
   - O terminal mostra a faixa e reabre o shell com um clique.
   - Uma edição pendente é reenviada depois da reconexão.
   - → testes na Task 5 (Docker com timeouts curtos) e na Task 7.
4. **Fechar sessão ou app com edições pendentes:** com reenvio em andamento ou com alterações locais ainda não enviadas, o app pergunta, inline na aba ou no fluxo de fechamento.
   - O reenvio termina ou o arquivo local é preservado.
   - O diretório temporário só é apagado depois.
   - → teste na Task 8.
5. **Arquivos e terminal na mesma sessão:**
   - Fechar uma aba não derruba a outra.
   - A reconexão restaura as duas.
   - Ao estourar o limite de canais do servidor (`MaxSessions`), aparece um erro inline claro, e não um travamento.
   - Workspaces salvos com abas de terminal reabrem com elas.
   - → testes nas Tasks 1 e 2.

---

## Estrutura de arquivos

```
crates/hoprs-ssh/src/shell.rs            # stream próprio de saída, close sem await inline
crates/hoprs-ssh/src/session.rs          # contagem de canais, TCP keepalive
crates/hoprs-remote/src/ftp/session.rs   # keepalive robusto, TCP keepalive
crates/hoprs-app/src/
  lib.rs                                 # NOVO: o app vira lib + bin fino (para os benches)
  main.rs                                # chama hoprs_app::run()
  app/tabs.rs                            # NOVO: abas por sessão (TabKind), substitui "uma aba por site"
  app/terminal.rs                        # NOVO: estado/Message da aba de terminal
  app/edit.rs                            # NOVO: edição remota (estado, pendências)
  app/palette.rs                         # NOVO: paleta Ctrl+K (comandos, filtro)
  app/log.rs                             # NOVO: painel de log técnico
  terminal/{mod.rs, input.rs, selection.rs, search.rs}   # NOVO: núcleo puro do terminal
  backend/shells.rs                      # NOVO: PTYs por sessão
  backend/edit.rs                        # NOVO: download, notify + debounce, reenvio
  logbuf.rs                              # NOVO: Layer de tracing -> ring buffer
  ui/{terminal.rs, palette.rs, log.rs}   # NOVO: views
crates/hoprs-app/benches/{ui.rs, startup.rs}            # NOVO
crates/hoprs-transfer/benches/network.rs               # NOVO (harness próprio, Docker + netem)
docker/compose.bench.yml, docker/bench/                 # NOVO: sshd/vsftpd com NET_ADMIN para netem
.github/workflows/perf.yml                              # NOVO: job de desempenho
docs/qa/plan-6-checklist.md                             # NOVO
```

---

### Task 1: Abas por sessão (pré-requisito do terminal)

**Files:**
- Create: `crates/hoprs-app/src/app/tabs.rs`
- Modify: `crates/hoprs-app/src/app.rs`, `app/lifecycle.rs`, `app/dialogs.rs`, `app/files.rs`, `ui/tabs.rs`, `ui/site_form.rs`, `backend/*`, testes

**Interfaces:**
- **`Tab` ganha `kind: TabKind`** (de `hoprs_core::TabKind`: `Files` e `Terminal`). As abas continuam identificadas por `TabId`, mas o vínculo passa a ser **com a sessão**: vários `TabId` podem compartilhar um `SessionId`.
- **Funções que pressupunham uma aba por site** (pendência "Later" do Plano 5) passam a trabalhar por aba ou por sessão: `tab_of`, `close_site_tabs`, `watched_sessions`, `abandon_site_prompts` e o descarte de prompts.
  - **Fechar uma aba** só fecha a sessão quando ela é a última aba daquela sessão.
  - **O feed de estado** é **um por sessão** (`SessionFeed`), e o estado é aplicado a todas as abas da sessão.
- **`OpenAs` é respeitado** em sites SSH:
  - `Files` abre só a aba de arquivos;
  - `Terminal` abre só a de terminal;
  - `Both` abre as duas, terminal primeiro.

  Sites FTP abrem sempre arquivos.
  - Nesta task a aba de terminal é um placeholder ("Terminal starts here"); o widget vem na Task 4.
  - O aviso "Terminal tabs arrive in a later version" sai do formulário.
- **Ação "New terminal tab"** e **"New files tab"** na aba de um site SSH conectado. Uma aba de arquivos a mais por sessão é permitida.
- **`SavedTab.kind`:** passa a ser salvo e restaurado. Uma aba `Terminal` restaurada abre um shell novo, sem o conteúdo anterior.

- [ ] **Step 1: Testes (falhando)** em `update` com `FakeBackend`:
  - `open_as_both_opens_terminal_then_files_on_one_session`: duas abas, mesmo `SessionId`, um único `connect`.
  - `closing_one_tab_keeps_the_session_for_the_other` **(Review Focus 5)**
  - `closing_the_last_tab_closes_the_session`
  - `session_state_reaches_every_tab_of_the_session`
  - `workspace_restores_terminal_tabs`
  - `ftp_site_ignores_open_as`
  - `delete_site_closes_all_its_tabs`
  - `profile_reload_closes_all_tabs_of_a_vanished_site`
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-app`
- [ ] **Step 3: Implementar.** Mova a lógica de abas para `app/tabs.rs` e mantenha passando os testes existentes das Tasks 5–11 do Plano 5. Se algum teste pressupunha "uma aba por site", ajuste-o e registre a mudança no relatório.
- [ ] **Step 4: Rodar e ver passar.** Run: `cargo test --workspace --all-features`
- [ ] **Step 5: Commit.** `git commit -m "feat(app): tabs per session, honour Open as"`

---

### Task 2: PTY no `hoprs-ssh` e no backend

**Files:**
- Modify: `crates/hoprs-ssh/src/{shell.rs, session.rs}`, testes em `crates/hoprs-ssh/tests/`
- Create: `crates/hoprs-app/src/backend/shells.rs`
- Modify: `crates/hoprs-app/src/backend/{mod.rs, sessions.rs, fake.rs}`

**Interfaces** (pendências "Before Plan 6" do Plano 2):
- **`ShellChannel::into_output(self) -> (ShellHandle, impl Stream<Item = ShellOutput> + Send + 'static)`**: um stream próprio, que serve para uma `Subscription`. `ShellHandle` é clonável e tem `write`, `resize` e `close`.
  - **`close()` não aguarda o `writer.close()` inline:** sinaliza o pump e retorna.
- **Orçamento de canais:**
  - **Contagem:** o `SshSession` conta os canais abertos (metadados SFTP, streams SFTP e PTYs).
  - **Limite:** `SshConfig.max_channels` (padrão **10**, o `MaxSessions` do OpenSSH).
  - **Reserva para PTY:** os streams de transferência reservam no máximo `max_channels - 2`, deixando espaço para pelo menos **1 PTY**.
  - **Erro:** abrir um PTY além do limite dá `ErrorKind::Protocol` "Too many channels on this connection (limit N)".
  - **Recusa do servidor:** se o servidor recusar a abertura do canal, sai o mesmo erro, com o detalhe do servidor.
- **`PtyRequest` ganha `env: Vec<(String, String)>`.** O padrão é `COLORTERM=truecolor`. Um servidor que recusa `env` é ignorado em silêncio, porque é comum o `AcceptEnv` estar desligado.
- **Backend (`backend/shells.rs`):**
  - `BackendApi::open_shell(session: SessionId, cols: u16, rows: u16) -> Result<ShellId>`
  - `shell_input(ShellId, Vec<u8>)`, `shell_resize(ShellId, u16, u16)` e `close_shell(ShellId)`
  - `fn shell_output(&self, ShellId) -> Option<BoxStream<'static, ShellOutput>>`, que entrega o stream uma única vez, para a subscription da aba.
  - **Ciclo de vida:** os shells pertencem à sessão. `close(session)` fecha todos, e a reconexão da sessão **não** reabre shells sozinha (a aba decide, Task 4).
- **`FakeBackend`** ganha shells roteirizados (eco da entrada, saída scriptada e `Closed` sob comando).

- [ ] **Step 1: Testes (falhando).**
  - Unitários no `hoprs-ssh`: `channel_budget_reserves_room_for_a_pty` e `close_does_not_wait_for_the_server`.
  - Docker (`#[ignore]`):
    - `shell_output_stream_is_owned` (abre, escreve `echo hi`, lê `hi`, fecha);
    - `pty_resize_reaches_the_server` (`stty size` responde `40 120` depois de `resize(120, 40)`);
    - `too_many_channels_is_a_clear_error` (com `max_channels` baixo);
    - `colorterm_env_is_sent_or_ignored`.
  - App, com `FakeBackend`: `shells_close_with_their_session` **(Review Focus 5)**.
- [ ] **Step 2: Rodar e ver falhar**
- [ ] **Step 3: Implementar e migrar os usos** de `output()`.
- [ ] **Step 4: Rodar e ver passar.** Run: `cargo test --workspace --all-features` e a suíte `--ignored`.
- [ ] **Step 5: Commit.** `git commit -m "feat(ssh): owned shell output, channel budget; app shell backend"`

---

### Task 3: Núcleo puro do terminal

**Files:**
- Create: `crates/hoprs-app/src/terminal/{mod.rs, input.rs, selection.rs, search.rs}`
- Modify: `crates/hoprs-app/Cargo.toml` (`alacritty_terminal`)

**Interfaces:**
- **`pub struct TerminalModel`**: envolve `alacritty_terminal::Term<EventProxy>` e o parser `vte`.
  - `new(cols: u16, rows: u16, scrollback: usize) -> Self`
  - `feed(&mut self, bytes: &[u8]) -> Vec<TermEvent>`. `TermEvent` pode ser `Title(String)`, `Bell`, `ClipboardStore(String)` (OSC 52, **ignorado por padrão**, por segurança) ou `PtyWrite(Vec<u8>)`, que são respostas do terminal como DA e DSR e precisam voltar ao PTY.
  - `resize(cols, rows)`
  - `cells(&self) -> impl Iterator<Item = CellView>`: `CellView` tem linha, coluna, `char`, fg/bg em RGB já resolvidos pela paleta, negrito, itálico, sublinhado, inverso e largura (1 ou 2 para CJK e emoji).
  - `cursor() -> CursorView`
  - `scroll(delta_lines: i32)` e `scroll_to_bottom()`
  - `mode() -> TermMode`: modo de cursor da aplicação e bracketed paste.
- **`input.rs`:** `pub fn encode_key(key: &Key, mods: Modifiers, mode: TermMode) -> Option<Vec<u8>>`.
  - Cobre texto, Enter, Backspace (`0x7f`), Tab e Shift+Tab, setas (CSI ou SS3 conforme o modo da aplicação), Home, End, PgUp, PgDn, Ins, Del, F1–F12, Ctrl+letra (`0x01`–`0x1a`), Ctrl+`[`, `\`, `]`, `^` e `_`, e Alt como prefixo ESC.
  - `pub fn encode_paste(text: &str, mode: TermMode) -> Vec<u8>`: com bracketed paste ligado, envolve o texto em `ESC[200~ … ESC[201~` e remove `ESC[201~` de dentro dele.
- **`selection.rs`:**
  - seleção por células, palavra (duplo clique) e linha (triplo clique), incluindo o scrollback;
  - `selected_text() -> String` junta as linhas que quebraram por largura sem inserir `\n`.
- **`search.rs`:** `pub fn find(&TerminalModel, query: &str, from: Point, backwards: bool) -> Option<Range>` faz a busca no scrollback inteiro, sem diferenciar maiúsculas por padrão.
- **Tema:** `pub struct TermTheme { fg, bg, cursor, selection, ansi: [Color; 16] }`. O padrão "hoprs dark" é derivado da paleta do app, com fundo `paper` e texto `ink`. Os 16 ANSI são escolhidos pelo implementador, com contraste ≥ 4.5:1 sobre o fundo para as cores 1–7 e 9–15, e registrados num teste.

- [ ] **Step 1: Testes (falhando).**
  - `truecolor_and_256_colors` (`ESC[38;2;1;2;3m` e `ESC[38;5;196m` viram o RGB certo)
  - `wide_chars_take_two_cells`
  - `invalid_utf8_and_garbage_never_panic` **(Review Focus 1)**: alimenta 1 MB de bytes pseudoaleatórios com seed fixa e sequências truncadas, e verifica o estado da grade.
  - `da_and_dsr_produce_pty_write`
  - `osc52_is_ignored_by_default`
  - `encode_keys_normal_and_app_cursor_mode`
  - `ctrl_and_alt_combinations`
  - `bracketed_paste_wraps_and_strips_end_marker`
  - `selection_joins_wrapped_lines`
  - `search_finds_in_scrollback_both_directions`
  - `scrollback_is_capped`
  - `theme_contrast`
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-app terminal::`
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(app): terminal model, input encoding, selection and search"`

---

### Task 4: Widget e aba de terminal

**Files:**
- Create: `crates/hoprs-app/src/ui/terminal.rs`, `crates/hoprs-app/src/app/terminal.rs`
- Modify: `app.rs`, `effects.rs`, `main.rs`/`lib.rs` (subscription por shell), `ui/strings.rs`, `settings.rs`, `ui/settings.rs`

**Interfaces:**
- **Widget:** desenha só as células visíveis com JetBrains Mono, no tamanho de fonte das configurações. A métrica de célula vem da fonte, numa medida feita uma vez por tamanho. Desenha o cursor (bloco, com contorno quando sem foco), a seleção em `accent_wash` e as marcas da busca.
- **Eventos do widget:**
  - **Resize:** o tamanho em px vira `cols × rows` e emite `Message::TerminalResized`, com debounce de 100 ms antes de `shell_resize`.
  - **Teclado com foco:** `encode_key` gera os bytes, que vão para `shell_input`.
  - **Atalhos de cópia e colagem:** Ctrl+Shift+C copia a seleção, e Ctrl+Shift+V cola do clipboard via `encode_paste`.
  - **Mouse:** a roda rola o scrollback, e arrastar seleciona.
  - **Busca:** Ctrl+Shift+F abre a barra de busca inline, com próximo e anterior.
- **Saída em lotes:** a subscription da aba junta os `ShellOutput::Data` recebidos e entrega **no máximo um `Message::TerminalOutput` por quadro** (~16 ms), com todos os bytes acumulados. Assim, um `cat` grande vira poucas mensagens **(Review Focus 1)**.
- **Backpressure:**
  - se o pump do `ShellChannel` transbordar (`overflowed`), a aba mostra inline "Output was too fast and the shell was closed", com "Reopen";
  - o limite de pendência do pump continua o do Plano 2.
- **Reconexão:**
  - **queda da sessão:** a aba mostra a faixa "Connection lost" com "Reconnect" (spec §8). Depois de reconectar, ela abre um **shell novo** e insere a linha "— reconnected —" no scrollback;
  - **fim do shell sem queda** (`Exit`): a faixa diz "Shell exited (code N)", com "Restart shell".
- **Título da aba:** `site` + tag `ssh`. Um OSC de título remoto aparece como dica, não substitui o nome do site.
- **Settings ganha:**
  - `terminal_font_size: u16` (10..=24, padrão 13);
  - `terminal_scrollback: usize` (1 000..=100 000, padrão 10 000);
  - `terminal_theme` (só "hoprs dark" por enquanto: o campo existe para não quebrar o arquivo depois).

  Todos persistem no `settings.toml`, preservando chaves desconhecidas.
- **Ctrl+K com o foco no terminal** vai para o shell (é "kill line" no bash). Nesse caso a paleta abre com **Ctrl+Shift+K** ou pelo botão da barra superior (Task 9).

- [ ] **Step 1: Testes (falhando)** em `update` e funções puras:
  - `pixels_to_grid_size`
  - `resize_is_debounced_100ms` (tempo injetado)
  - `typing_sends_encoded_bytes`
  - `copy_paste_shortcuts`
  - `output_batches_one_message_per_frame` **(Review Focus 1)**: o adaptador de stream recebe 10 000 `Data` e entrega ≤ 2 mensagens dentro de uma janela de 16 ms simulada.
  - `overflow_shows_inline_and_reopen` **(Review Focus 1)**
  - `connection_lost_banner_then_new_shell` **(Review Focus 5)**
  - `exit_shows_restart`
  - `ctrl_k_goes_to_the_shell_when_focused`
  - `settings_terminal_fields_roundtrip`
  - Docker (`#[ignore]`): `terminal_tab_end_to_end`, que abre o site SSH com `OpenAs::Both`, digita `echo hoprs` e vê o eco no modelo.
- [ ] **Step 2: Rodar e ver falhar**
- [ ] **Step 3: Implementar.** Verificação manual com screenshots: `htop` ou `vim` em truecolor, um `cat` de 50 MB com a UI respondendo, resize da janela com `stty size` acompanhando, busca, copiar e colar. Registre tudo no `docs/qa/plan-6-checklist.md`.
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(app): terminal tab"`

---

### Task 5: Quedas em ociosidade (relato do autor)

**Files:**
- Modify: `crates/hoprs-ssh/src/{session.rs, config.rs}`, `crates/hoprs-remote/src/ftp/{session.rs, config.rs}`, `docker/compose.yml` (serviços com timeouts curtos) e os Dockerfiles, `crates/hoprs-app/src/{app/lifecycle.rs, app/files.rs, backend/sessions.rs}`, testes

**Contexto:** o autor viu sessões caírem depois de um tempo sem uso. Hoje existe keepalive: SSH via russh a cada 30 s, e FTP com `NOOP` a cada 30 s na conexão de controle ociosa. As causas prováveis são:
- **(a) FTP:** o probe é descartado quando a conexão de controle está ocupada (pendência do Plano 3), ou o servidor não conta `NOOP` como atividade;
- **(b) NAT, firewall ou roteador:** derrubam a conexão TCP ociosa sem avisar, e o sinal só aparece no próximo uso;
- **(c) desistência do app:** as 5 tentativas de reconexão se esgotam enquanto o usuário não está olhando, e a aba fica `Disconnected` esperando um clique.

**Interfaces:**
- **Diagnóstico primeiro:**
  - reproduzir com serviços Docker de timeout curto: vsftpd com `idle_session_timeout=20` e sshd com `ClientAliveInterval 5` / `ClientAliveCountMax 1`;
  - simular um corte silencioso, com o contêiner pausado (`docker pause`) ou com `iptables DROP` no contêiner;
  - registrar no relatório a causa observada para cada protocolo.
- **TCP keepalive no socket:** `SO_KEEPALIVE` com `idle=60 s`, `interval=10 s` e `count=3` (via `socket2`) nos sockets SSH e FTP (controle e dados), para que roteadores e NATs vejam tráfego.
- **FTP:**
  - o keepalive deixa de usar `try_lock`: espera o comando em curso terminar e depois manda o `NOOP` se ainda estiver ocioso;
  - `FtpConfig.keepalive_interval` passa a ser configurável pelo app.
- **Settings ganha `keepalive_secs: u64`** (10..=300, padrão 30), aplicado ao SSH e ao FTP nas próximas conexões.
- **Transições de conexão no log, em `info`:** hoje uma queda não deixa rastro no log padrão (o autor rodou o app por 33 min, viu quedas, e o terminal só mostrou a inicialização do iced e o import).
  - Toda transição de `ConnectionState` de cada sessão é logada com site, protocolo, `host:port`, estado novo, motivo (`ErrorKind` mais a mensagem) e há quanto tempo a sessão estava ociosa.
  - As tentativas de reconexão e o resultado delas vão em `info`, e os keepalives sem resposta vão em `warn`.
  - Nada de segredos.
- **Reconexão sob demanda:** quando a sessão de uma aba está `Disconnected` por queda de rede (não por host key nem por autenticação), uma ação do usuário na aba **dispara a reconexão sozinha**, e a ação é repetida depois de `Connected`. As ações que contam são listar, navegar, enfileirar ou abrir um shell. A faixa mostra "Reconnecting…" no lugar de um erro.

- [ ] **Step 1: Testes (falhando).**
  - Docker (`#[ignore]`):
    - `ftp_survives_server_idle_timeout` (60 s ociosos com `idle_session_timeout=20`, depois `list` funciona sem erro visível);
    - `ssh_survives_client_alive_checks`;
    - `silent_drop_is_detected_and_recovered` **(Review Focus 3)**.
  - Unitários:
    - `connection_transitions_are_logged_at_info` (com um subscriber de teste: uma queda e uma reconexão produzem linhas `info` com motivo e tempo ocioso);
    - `tcp_keepalive_is_set_on_sockets`, que lê `SO_KEEPALIVE` do socket;
    - `ftp_keepalive_waits_for_busy_control`.
  - App: `action_on_dropped_session_reconnects_and_retries` **(Review Focus 3)** e `host_key_or_auth_failure_does_not_auto_reconnect`.
- [ ] **Step 2: Rodar e ver falhar.** Registre a causa vista no diagnóstico.
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar.** Run: `cargo test --workspace --all-features` e a suíte `--ignored`.
- [ ] **Step 5: Commit.** `git commit -m "fix: keep idle sessions alive and reconnect on demand"`

---

### Task 6: Log técnico

**Files:**
- Create: `crates/hoprs-app/src/logbuf.rs`, `crates/hoprs-app/src/app/log.rs`, `crates/hoprs-app/src/ui/log.rs`
- Modify: `main.rs`/`lib.rs` (registro da `Layer`), `ui/layout.rs`, `app.rs`, `ui/strings.rs`

**Interfaces:**
- **`LogBuffer`:** uma `tracing_subscriber::Layer` que grava `LogLine { time, level, target, message }` num ring buffer de **5 000 linhas** (`Arc<Mutex<VecDeque>>` com lock curto, sem I/O).
  - Ela respeita o **mesmo** `EnvFilter` do terminal, incluindo o teto da `suppaftp`.
  - Os campos são formatados como `key=value`.
  - Mensagens com mais de 2 KB são cortadas.
- **Painel "Log":**
  - **Posição:** recolhido por padrão (spec §8), abre pela barra de status (um botão "Log") ou pela paleta.
  - **Atualização:** uma subscription faz polling a cada 250 ms enquanto o painel está aberto. A lista é virtualizada (reaproveita `virtual_list`), em mono.
  - **Filtros:** por nível (Error, Warn, Info, Debug) e por texto.
  - **Ações:** "Copy all" e "Clear".
- **Sem segredos:** nenhum ponto de log do app pode receber senha, passphrase, bytes do terminal ou conteúdo de arquivo.
- **Filtro padrão mais limpo:**
  - Passa a ser `info,suppaftp=warn,wgpu_hal=error,wgpu_core=warn,iced_winit=warn,iced_wgpu=warn,sctk_adwaita=error`.
  - Motivo: o autor viu dezenas de linhas de inicialização do iced e do wgpu a cada abertura, que escondem as linhas úteis. Os avisos benignos `No config found!` e `Ignoring unknown button type` somem.
  - Um `RUST_LOG` explícito continua mandando, com o teto da `suppaftp` mantido.

- [ ] **Step 1: Testes (falhando).**
  - `ring_buffer_caps_at_5000`
  - `layer_respects_the_filter_and_suppaftp_cap`
  - `default_filter_quiets_graphics_stack`: `wgpu_hal` e `iced_winit` em `info` ficam fora; `hoprs_app` em `info` entra.
  - `long_messages_are_truncated`
  - `panel_filters_by_level_and_text`
  - `no_secret_reaches_the_log`: conecta com `FakeBackend` usando uma senha distinta, digita no terminal fake, faz upload de um arquivo com conteúdo distinto e verifica que o buffer não contém nenhum dos três.
- [ ] **Step 2: Rodar e ver falhar**
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(app): technical log panel"`

---

### Task 7: Edição remota — backend

**Files:**
- Create: `crates/hoprs-app/src/backend/edit.rs`
- Modify: `crates/hoprs-app/src/backend/{mod.rs, fake.rs, sessions.rs}`, `crates/hoprs-app/Cargo.toml` (`notify`)

**Interfaces:**
- **`BackendApi::start_edit(session: SessionId, remote: RemotePath) -> Result<EditId>`:**
  - faz `stat` e download para `edit_dir/<session>/<path>`;
  - guarda o `RemoteStamp { size, mtime }` do momento do download;
  - abre o arquivo com o editor das configurações via `std::process::Command`, em `spawn_blocking`. O comando é dividido em argumentos, sem shell, e o caminho é o último argumento.
- **`edit_dir`:**
  - fica em `$XDG_RUNTIME_DIR/hoprs/edit`, com fallback no diretório de cache;
  - pastas criadas com permissão **0700** e arquivos **0600**;
  - o `<path>` remoto é saneado: componentes `..` e absolutos são proibidos, e um caminho que escaparia do diretório da sessão é recusado.
- **Observação:**
  - **Watcher:** usa `notify`, com **um watcher por diretório de sessão**.
  - **Debounce:** 500 ms por arquivo.
  - **Escrita atômica dos editores:** é tolerada, ou seja, rename sobre o arquivo e remove + create.
- **Reenvio:**
  - **Conferência antes de enviar:** faz `stat` remoto. Se `size` ou `mtime` diferirem do último `RemoteStamp` conhecido, **não envia** e emite `EditEvent::RemoteChanged { id }` **(Review Focus 2)**.
  - **Sem conflito:** faz upload direto pela `RemoteFs` (escreve num nome temporário no mesmo diretório e renomeia por cima quando o protocolo permite; senão escreve no próprio arquivo), atualiza o `RemoteStamp` e emite `EditEvent::Uploaded { id, at }`.
  - **Com a sessão caída:** a edição fica `Pending` e é reenviada automaticamente no próximo `Connected` **(Review Focus 3)**.
- **Resolução do conflito:** `resolve_edit(id, EditChoice::{Overwrite, KeepRemote, SaveCopy})`.
  - `KeepRemote` baixa de novo e substitui o arquivo local. Antes, guarda a versão local como `<nome>.local-<timestamp>`.
  - `SaveCopy` envia como `<nome>.hoprs-<timestamp>` ao lado do original.
- **`edits_pending(session) -> Vec<EditSummary>`** e **`finish_edits(session, discard: bool)`:** apagam os arquivos locais da sessão só quando não há pendência, ou quando o usuário mandou descartar.
- **`subscribe_edits()`:** stream de `EditEvent` para a UI. Os eventos são `Uploaded`, `RemoteChanged`, `Pending`, `Failed(Error)` e `Closed`.

- [ ] **Step 1: Testes (falhando)** no backend, com `MemFs` e um diretório temporário:
  - `download_goes_to_private_dir_with_0700_0600`
  - `path_traversal_is_refused`
  - `save_uploads_after_500ms_debounce` (várias gravações viram um envio)
  - `atomic_rename_saves_are_seen`
  - `remote_change_blocks_upload_and_offers_choices` **(Review Focus 2)**
  - `keep_remote_preserves_local_copy`
  - `save_copy_uploads_beside`
  - `pending_edit_uploads_after_reconnect` **(Review Focus 3)**
  - `finish_keeps_files_with_pending_changes`
  - `editor_command_is_split_without_shell`
- [ ] **Step 2: Rodar e ver falhar**
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar.** Inclui um teste Docker (`#[ignore]`) `edit_roundtrip_sftp_and_ftp`.
- [ ] **Step 5: Commit.** `git commit -m "feat(app): remote editing backend"`

---

### Task 8: Edição remota — UI

**Files:**
- Create: `crates/hoprs-app/src/app/edit.rs`
- Modify: `app/files.rs`, `app/actions.rs`, `ui/files.rs`, `ui/file_actions.rs`, `app/lifecycle.rs` (fechamento), `settings.rs`, `ui/settings.rs`, `ui/strings.rs`

**Interfaces:**
- **Ação "Edit" (F4)** no painel remoto. Fica desabilitada para pastas, links e arquivos acima de **50 MB**, com dica inline. A ação dispara `start_edit`.
- **Avisos discretos:**
  - **Faixa fina da aba de arquivos:** mostra uma linha por edição ativa, com o estado ("Editing config.php — saved 14:02", "Waiting for connection…", "Upload failed: … Retry").
  - **Toast:** cada `Uploaded` mostra um toast de 3 s (sem animação quando o sistema pede menos movimento).
  - **`RemoteChanged`:** mostra a escolha inline "Changed on the server since you opened it — Overwrite / Keep remote / Save a copy".
- **Fechamento** **(Review Focus 4):**
  - **Fechar a última aba de uma sessão com edições ativas:** aparece uma confirmação inline na aba, listando os arquivos: "Upload pending changes and close", "Close and keep local copies" ou "Cancel".
  - **Fechar a janela:** o fluxo do Plano 5 ganha um passo **antes** do save: se houver edições pendentes, a janela não fecha e mostra a mesma pergunta no centro.
    - "Upload and quit" espera os reenvios até 10 s.
    - "Quit and keep local copies" mostra onde os arquivos ficam e não apaga nada.
- **Settings ganha `editor_command: String`** (padrão `xdg-open`, validado como não vazio).

- [ ] **Step 1: Testes (falhando)** em `update`:
  - `edit_action_rules` (pasta, link, > 50 MB)
  - `uploaded_shows_toast_and_status`
  - `remote_changed_choice_inline`
  - `closing_last_tab_with_edits_asks` **(Review Focus 4)**
  - `window_close_with_pending_edits_asks_before_saving` **(Review Focus 4)**
  - `keep_local_copies_never_deletes`
  - `editor_setting_roundtrip`
- [ ] **Step 2: Rodar e ver falhar**
- [ ] **Step 3: Implementar**, com verificação manual: editar com `gedit` ou `code --wait` e gravar três vezes seguidas.
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(app): remote editing UI"`

---

### Task 9: Paleta Ctrl+K

**Files:**
- Create: `crates/hoprs-app/src/app/palette.rs`, `crates/hoprs-app/src/ui/palette.rs`
- Modify: `app.rs`, `ui/layout.rs` (o slot "Go to anything" vira o botão), `ui/strings.rs`

**Interfaces:**
- **`pub fn fuzzy_score(query: &str, candidate: &str) -> Option<u32>`:** é pura, não diferencia maiúsculas, exige que os caracteres apareçam em ordem e dá bônus a início de palavra e a sequências contíguas. O desempate é alfabético.
- **Comandos (`PaletteItem`):**
  - **Go to:** tab (as abertas, com "site · kind"), site (conecta ou foca) e workspace (abre).
  - **Actions:** New site, Import from FileZilla, Settings, Toggle queue, Toggle log, New terminal tab, New files tab, Reconnect, Close tab, Open workspace…
  - **Visibilidade:** um item só aparece quando faz sentido no estado atual. Por exemplo, "New terminal tab" exige uma aba SSH ativa conectada.
- **Overlay:**
  - **Abertura:** Ctrl+K, ou Ctrl+Shift+K com o terminal em foco, ou o botão da barra superior.
  - **Posição e foco:** fica no topo e no centro, com campo de busca focado e até 12 resultados.
  - **Teclado:** ↑/↓ navegam, Enter executa, Esc fecha.
  - **Bloqueios:** a paleta não abre enquanto houver um diálogo de segurança aberto. Ela bloqueia a entrada da janela de trás, como os diálogos do Plano 5.
- **Atalho:** cada ação continua acessível pelo seu atalho, e a paleta mostra o atalho ao lado.

- [ ] **Step 1: Testes (falhando).**
  - `fuzzy_matches_in_order_and_ranks_word_starts`
  - `palette_lists_context_actions`
  - `enter_runs_and_closes`
  - `escape_closes_without_effects`
  - `palette_blocked_while_security_dialog_open`
  - `keys_do_not_leak_to_panes_or_terminal`
- [ ] **Step 2: Rodar e ver falhar**
- [ ] **Step 3: Implementar**
- [ ] **Step 4: Rodar e ver passar**
- [ ] **Step 5: Commit.** `git commit -m "feat(app): Ctrl+K command palette"`

---

### Task 10: Benchmarks das metas da §9

**Files:**
- Create: `crates/hoprs-app/src/lib.rs` (o app passa a ser lib + bin), `crates/hoprs-app/benches/{ui.rs, startup.rs}`, `crates/hoprs-transfer/benches/network.rs`, `docker/compose.bench.yml`, `docker/bench/`, `.github/workflows/perf.yml`, `docs/perf/README.md`
- Modify: `crates/hoprs-app/src/main.rs` (só chama `hoprs_app::run()`), `crates/hoprs-app/Cargo.toml`, `crates/hoprs-transfer/Cargo.toml` (dev-deps: `hoprs-ssh`, `hoprs-remote` com `sftp`/`ftp`)

**Interfaces** (uma meta da spec por medida):

| Meta (spec §9) | Como medir | Critério |
|---|---|---|
| Abertura < 500 ms | O binário com `HOPRS_PROBE_STARTUP=1` mede do `main` ao primeiro quadro desenhado, imprime `startup_ms=<n> rss_mb=<m>` e sai. Roda sob `xvfb-run`, com o renderer tiny-skia, mediana de 5 execuções | `startup_ms < 500` |
| Memória ociosa < 80 MB | O mesmo probe, 2 s depois do primeiro quadro, sem sessões, lendo o RSS em `/proc/self/status` | `rss_mb < 80` |
| 100 mil entradas ≥ 60 fps | Bench `criterion` (`benches/ui.rs`): `view()` de uma aba com 100 000 entradas, mais o layout do `virtual_list`, num offset aleatório | p95 < **16 ms** |
| SFTP 1 GB a 50 ms de RTT ≥ 90% do `scp` | `benches/network.rs`: `tc qdisc add dev eth0 root netem delay 25ms` no contêiner sshd (25 ms em cada sentido, o que dá 50 ms de RTT); upload de 1 GB pelo `TransferEngine` + `SftpFs`, contra `scp` na mesma condição; mediana de 3 | `hoprs ≥ 0.9 × scp` |
| 10 mil arquivos pequenos (SFTP e FTP) não piores que o `lftp` | O mesmo harness: 10 000 arquivos de 4 KB, `workers=4`, contra `lftp mirror -R --parallel=4`, em SFTP e em FTP, com a mesma latência | `hoprs ≤ 1.0 × lftp` (tempo) |
| Progresso ≤ 10 Hz | Teste no `hoprs-transfer` que conta os eventos `Progress` durante 3 s de transferência com 4 workers | ≤ 31 eventos em 3 s |

- **Bench de rede:**
  - **Harness:** `harness = false` no `Cargo.toml`. Lê `HOPRS_BENCH_*` (host, portas, credenciais do `compose.bench.yml`) e sai com código ≠ 0 quando um critério falha.
  - **Saída:** imprime uma tabela em Markdown, que o workflow anexa ao `$GITHUB_STEP_SUMMARY`.
- **`compose.bench.yml`:** usa as mesmas imagens sshd e vsftpd, com `cap_add: [NET_ADMIN]` e `iproute2` instalado.
- **`perf.yml`:**
  - roda em `workflow_dispatch`, em `push` na `main` e semanalmente (`schedule`);
  - **não** faz parte das checks obrigatórias de PR;
  - instala `xvfb`, `lftp` e `openssh-client`;
  - sobe o compose de bench e roda o probe, o `cargo bench -p hoprs-app` e o bench de rede.
- **Instruções locais:** ficam em `docs/perf/README.md`.
- **Pendências de desempenho:** se uma meta falhar, o implementador investiga e corrige o que for barato. Pendências conhecidas que tocam as metas:
  - `LocalFs::list` faz `stat` entrada por entrada (1,7 s para 100 mil);
  - o FTP faz `stat` na conexão de controle;
  - o FTP manda um `NOOP` a cada reuso de conexão do pool;
  - arquivos pequenos geram 64 requisições SFTP.

  O que não for corrigido fica registrado com os números medidos.

- [ ] **Step 1: Testes (falhando).** `progress_rate_at_most_10hz`, e um teste do parser do probe (`parse_probe_line`).
- [ ] **Step 2: Rodar e ver falhar**
- [ ] **Step 3: Implementar** o lib + bin, o probe, os benches, o compose e o workflow. Rode tudo localmente e registre os números em `docs/perf/README.md`.
- [ ] **Step 4: Rodar e ver passar.** O workflow precisa passar uma vez em `workflow_dispatch` na branch: `gh workflow run perf.yml --ref plan-6-...`.
- [ ] **Step 5: Commit.** `git commit -m "perf: spec §9 benchmarks and CI job"`

---

### Task 11: Checklist visual, polimento e documentação

**Files:**
- Create: `docs/qa/plan-6-checklist.md`
- Modify: `docs/superpowers/plans/2026-11-12-01-followups.md`, `CLAUDE.md`, `README.md`, qualquer arquivo de `ui/` com ajustes de polimento

**Interfaces:**
- **O checklist (spec §11)** cobre:
  - terminal: `vim`, `htop`, `less`, `tmux`, cores 256 e truecolor, CJK e emoji, resize, busca, copiar e colar, saída de 50 MB, queda de rede e reconexão;
  - abas de arquivos e terminal na mesma sessão;
  - workspace com aba de terminal salva;
  - edição remota com conflito;
  - fechamento com edição pendente;
  - paleta;
  - log;
  - sessão ociosa por 10 minutos contra o servidor real do autor (marcado NEEDS HUMAN se não houver acesso);
  - os números do job de desempenho.

  Cada linha fica com OK, NEEDS HUMAN (com os passos) ou o problema mais o commit que corrigiu.
- **Follow-ups:**
  - marcar como resolvidas as pendências que este plano fechou: Plano 2 "Before Plan 6", "one-tab-per-site" do Plano 5, o probe de keepalive FTP do Plano 3;
  - acrescentar a seção "Plan 6".
- **`CLAUDE.md`:**
  - status: Plano 6 na branch, com o próximo passo de fazer o merge e preparar o release v1;
  - entradas novas em "Where things are".
- **`README.md`:** terminal, edição remota (com o editor configurável) e atalhos (Ctrl+K, Ctrl+Shift+C/V/F/K, F2, F4, F7, Del).

- [ ] **Step 1: Executar o checklist** com o app em release e o Docker de teste.
- [ ] **Step 2: Corrigir o que falhar**, com teste em `update` quando o problema for de lógica.
- [ ] **Step 3: Rodar tudo.** `cargo test --workspace --all-features`, a suíte `--ignored`, clippy e fmt.
- [ ] **Step 4: Commit.** `git commit -m "docs: Plan 6 QA checklist, polish and status"`

---

## Decisões deste plano

- **Ctrl+K no terminal vai para o shell.** É atalho do bash, do emacs e de outros programas. Com o terminal em foco, a paleta abre por Ctrl+Shift+K ou pelo botão. Copiar e colar no terminal usam Ctrl+Shift+C/V, como nos terminais Linux.
- **OSC 52 (o servidor escrever no clipboard) fica desligado.** Uma sessão remota não deve poder escrever no clipboard local sem pedir. Se for pedido, pode virar uma opção depois.
- **Reenvio da edição remota fora da fila de transferências.** Ele é direto pela `RemoteFs`, como a exclusão recursiva no Plano 5, porque a fila não tem política por item. O custo é que edições não aparecem na fila.
- **App como lib + bin**, para os benches acessarem `view()` e o probe de abertura. Nenhum crate novo.
- **Job de desempenho separado e não obrigatório no PR** (spec §11, "job separado"). Ele falha quando uma meta não é atingida, mas não bloqueia merges, porque runners compartilhados são ruidosos. Os números ficam no resumo do job.
- **Quedas em ociosidade:**
  - TCP keepalive no socket, keepalive do FTP sem descarte e reconexão sob demanda.
  - O número de tentativas de reconexão automática **não** aumenta: uma ação do usuário já reconecta.

## Critério de conclusão do Plano 6

- `cargo test --workspace --all-features` e a suíte `--ignored` com Docker verdes; CI verde.
- O job `perf.yml` rodou com sucesso uma vez, com os números registrados em `docs/perf/README.md`. Uma meta não atingida está corrigida ou registrada com número e causa.
- Todos os itens da Review Focus têm teste passando.
- O checklist do Plano 6 foi executado e registrado.
- O terminal funciona em `vim`, `htop` e `tmux` sobre a mesma conexão da aba de arquivos.
- A edição remota reenvia ao gravar e protege contra sobrescrita.
- A paleta e o log funcionam.
- As sessões ociosas não exigem clique para voltar.
- Follow-ups e `CLAUDE.md` atualizados.
- Próximo passo: preparar o release v1 (empacotamento, ícone, `.desktop`, versão).
