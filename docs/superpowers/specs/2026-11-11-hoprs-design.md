# hoprs — Design Spec

- **Data:** 2026-11-11
- **Status:** aguardando revisão
- **Referência visual:** [`assets/2026-11-11-hoprs-mockup-v2.html`](assets/2026-11-11-hoprs-mockup-v2.html) (mockup aprovado; abrir no navegador)

## 1. Propósito

Um cliente desktop em Rust, open source (MIT/Apache-2.0), que junta **SSH, SFTP, FTP e FTPS** numa interface limpa e rápida. Ele substitui o FileZilla no uso diário do autor e serve à comunidade.

**Problema:** a interface do FileZilla é poluída e confusa, e ele não tem terminal SSH. Hoje o autor alterna entre ferramentas diferentes dependendo do projeto (alguns usam SFTP, outros FTP, outros SSH).

**Critérios de sucesso da v1:**
- Todos os sites do autor importados do FileZilla e utilizáveis sem voltar para ele.
- Um lugar só para abrir terminal SSH, arquivos SFTP e arquivos FTP/FTPS.
- Metas de desempenho (seção 9) cumpridas e medidas na CI.

**Fora de escopo da v1:** Windows e macOS (o código é portável, mas não é testado nem distribuído), sincronização de diretórios, busca remota recursiva, pré-visualização de arquivos, túneis/port forwarding, split panes no terminal, snippets, gravação de sessão.

## 2. Plataforma e stack

| Item | Escolha |
|---|---|
| Plataforma alvo v1 | Linux (Ubuntu) |
| Linguagem | Rust stable (workspace Cargo) |
| GUI | `iced` (arquitetura Elm, renderização wgpu) |
| Runtime assíncrono | `tokio` |
| SSH / SFTP | `russh` + `russh-sftp` |
| FTP / FTPS | `suppaftp` (feature async + rustls) |
| Terminal | `alacritty_terminal` (modelo VT), widget próprio baseado em `iced_term` |
| Keyring | `keyring` (Secret Service no Linux) |
| Fila persistente | SQLite via `rusqlite` |
| Observação de arquivos | `notify` |
| Segredos em memória | `secrecy` + `zeroize` |
| Erros | `thiserror` (núcleo), mensagens legíveis na UI |
| Idioma da UI | Inglês, com i18n preparado (strings centralizadas) |

## 3. Conceitos do domínio

- **Site:** uma conexão salva. Tem nome, host, porta, usuário, protocolo, método de autenticação e opções (pasta remota inicial, pasta local inicial, modo passivo/ativo etc.).
  - **Protocolo `ssh`:** abre **terminal**, **arquivos (SFTP)** ou **ambos em abas**, reaproveitando a mesma conexão autenticada.
  - **Protocolo `ftp` / `ftps-explicit` / `ftps-implicit`:** abre **arquivos**. FTP é de primeira classe, com a mesma qualidade de UI e de fila que o SFTP.
- **Workspace:** um pacote nomeado de sites (ex.: *Shop* = shop-prod SSH + shop-staging SFTP + shop-cdn FTPS). Um site pode estar em vários workspaces. **Open workspace** conecta todos em paralelo e restaura as abas e os diretórios salvos. Uma falha num site não impede os outros.
- **Sessão:** uma conexão viva com um site. Pode ter várias abas (terminal, arquivos).
- **Transferência:** um item da fila (arquivo ou pasta) entre local e um site.

## 4. Arquitetura

### 4.1 Crates

```
hoprs/
├── crates/
│   ├── hoprs-core/      # tipos comuns: Site, Protocol, Workspace, RemotePath, RemoteEntry, Error
│   ├── hoprs-vault/     # perfis TOML + keyring + export/import cifrado
│   ├── hoprs-remote/    # trait RemoteFs + impls sftp.rs e ftp.rs
│   ├── hoprs-ssh/       # sessão SSH: auth, PTY, keepalive, known_hosts
│   ├── hoprs-transfer/  # motor de transferências: fila, workers, retomada, conflitos
│   ├── hoprs-import/    # importador do FileZilla (sitemanager.xml)
│   └── hoprs-app/       # binário iced: telas, widgets, tema
```

**Regras:**
- Só `hoprs-app` depende do `iced`. Todo o resto é testável sem GUI.
- Um único runtime tokio. A thread da UI nunca bloqueia em I/O.
- Grafo de dependências (A → B significa "A depende de B"): todos → `core`. `remote` → `ssh`. `transfer` → `remote`. `import` → `vault`. `app` → `transfer`, `remote`, `ssh`, `vault`, `import`. Nenhuma outra aresta.

### 4.2 `RemoteFs`

A abstração central. O navegador de arquivos e o motor de transferências não sabem qual protocolo está por trás.

```rust
#[async_trait]
pub trait RemoteFs: Send + Sync {
    async fn list(&self, path: &RemotePath) -> Result<Vec<RemoteEntry>>;
    async fn stat(&self, path: &RemotePath) -> Result<RemoteEntry>;
    async fn open_read(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncRead>;
    async fn open_write(&self, path: &RemotePath, offset: u64) -> Result<BoxAsyncWrite>;
    async fn mkdir(&self, path: &RemotePath) -> Result<()>;
    async fn rename(&self, from: &RemotePath, to: &RemotePath) -> Result<()>;
    async fn remove_file(&self, path: &RemotePath) -> Result<()>;
    async fn remove_dir(&self, path: &RemotePath) -> Result<()>;
    async fn chmod(&self, path: &RemotePath, mode: u32) -> Result<()>;
}
```

Implementações: `SftpFs` (russh-sftp), `FtpFs` (suppaftp) e `MemFs` (testes, em `hoprs-remote` atrás da feature `testing`).

### 4.3 Sessões

- **SSH:** uma conexão `russh` por sessão. O terminal é um canal com PTY. O SFTP é um canal de subsistema. Os workers de transferência abrem canais SFTP extras **na mesma conexão**, sem reautenticar.
- **FTP:** um pool por sessão: uma conexão de controle para navegação e até N conexões para transferências, autenticadas sob demanda.
- **Estados:** `Connecting → Connected → Reconnecting → Disconnected(reason)`. Uma queda dispara reconexão com backoff exponencial (até 5 tentativas). Enquanto isso, a fila daquele site fica **pausada**, sem falhar.
- **Keepalive:** SSH a cada 30 s (configurável). FTP com `NOOP` na conexão de controle ociosa.

### 4.4 Comunicação UI ↔ núcleo

```
iced (UI)                              tokio (núcleo)
  Message::Command(..) ──mpsc──▶  SessionManager ─▶ Session ─▶ RemoteFs
                                   TransferEngine ─▶ workers
  Subscription ◀──── broadcast ─── Event::{Progress, DirListed, StateChanged, TransferFailed, ...}
```

- Os comandos são enums tipados. Os eventos carregam IDs (`SessionId`, `TransferId`, `TabId`).
- **O progresso é agregado a ~10 Hz** no núcleo antes de ir para a UI.

## 5. Motor de transferências

- **Fila persistente** em SQLite (`$XDG_DATA_HOME/hoprs/queue.db`). Sobrevive ao fechamento do app e a quedas de conexão. Ao reabrir, os itens incompletos aparecem como pausados.
- **Expansão preguiçosa de pastas:** enfileirar uma pasta cria um item que é listado e expandido de forma incremental, enquanto os workers consomem os arquivos.
- **Workers por site:** padrão de 4, configurável de 1 a 16.
- **SFTP:** pipelining de requisições (padrão 64 em voo × 32 KB por arquivo).
- **FTP:** modo binário, stream direto com buffer de 256 KB.
- **Retomada:** offset baseado no tamanho do arquivo de destino (`REST` no FTP, offset no SFTP). Validação final por tamanho. O mtime é preservado quando o protocolo permite (`MFMT`/`setstat`).
- **Conflitos:** política por fila: `ask` (padrão), `overwrite`, `skip`, `overwrite_if_newer`, `resume`. Em modo `ask`, a pergunta aparece **inline na linha da fila**, com "apply to all". Ela não abre um modal.
- **Operações:** pausar, retomar, cancelar, tentar de novo e pular, por item e em lote.
- **Edição remota:** baixa para `$XDG_RUNTIME_DIR/hoprs/edit/<session>/<path>`, abre com o editor configurado (padrão: `xdg-open`) e observa com `notify`. A cada gravação (com debounce de 500 ms) reenvia e mostra um aviso discreto. Ao fechar a sessão, pergunta sobre pendências.

## 6. Perfis, credenciais e import

- **Perfis:** `$XDG_CONFIG_HOME/hoprs/sites.toml` e `workspaces.toml`. **Nunca contêm segredos.**
- **Segredos** (senha, passphrase de chave) ficam no keyring do sistema, com chave `hoprs/<site-id>`.
- **Chaves SSH:** suporte a `~/.ssh/id_*`, chave por perfil e **ssh-agent** (caminho preferido).
- **Export/import cifrado:** um arquivo único com perfis, workspaces e segredos, cifrado com senha (Argon2id + XChaCha20-Poly1305). Serve para backup e para migrar de máquina.
- **Import do FileZilla:** lê `~/.config/filezilla/sitemanager.xml`, convertendo pastas em workspaces (opcional, perguntado no fluxo), protocolos, portas, usuários, modo passivo/ativo e pastas iniciais. As senhas (base64) vão direto para o keyring. O app mostra um relatório de importação (importados, ignorados e motivo) e oferece apagar o XML original.

## 7. Segurança

- **known_hosts:** lê e escreve `~/.ssh/known_hosts` no formato OpenSSH (compatível com o `ssh` da linha de comando). Um host desconhecido mostra o fingerprint SHA256 e pede confirmação. **Fingerprint alterado bloqueia a conexão.** Para continuar, é preciso remover a entrada explicitamente.
- **Ordem de autenticação SSH:** ssh-agent → chave do perfil → senha → keyboard-interactive (2FA).
- **FTPS:** explícito é o padrão sugerido. A validação usa rustls + raízes do sistema. Certificado inválido ou autoassinado gera uma pergunta mostrando o fingerprint; se aceito, fica **fixado no perfil**. Uma mudança posterior volta a perguntar.
- **FTP sem TLS:** suportado, rotulado como `ftp (unencrypted)` no perfil e na aba, sem avisos repetidos.
- **FTP passivo:** padrão. Há um contorno para `PASV` que devolve IP privado (usa o IP do host de controle). Modo ativo é opcional.
- Segredos em memória com `secrecy`/`zeroize`. Logs nunca contêm senhas, passphrases ou conteúdo de arquivos.

## 8. Interface

Referência: mockup v2 aprovado.

**Linguagem visual:**
- Tema escuro neutro (fundo ~`oklch(13% 0 0)`, superfícies em três degraus de cinza). O **destaque é branco**, usado só em seleção, ação principal, aba ativa, foco e progresso.
- Fontes: **Inter** na interface e **JetBrains Mono** nos dados (nomes de arquivo, caminhos, permissões, `user@host`, tags de protocolo). Ambas vão embutidas no binário.
- Cantos de 4px, linhas de 1px e nenhuma sombra. Textos em sentence case.
- Movimento apenas como mudança de estado (~120 ms). Respeita a preferência de movimento reduzido.

**Layout:**
- **Barra superior:** marca, abas (uma por aba de sessão, com o rótulo `site` + tag de protocolo) e o botão **Ctrl+K** (paleta de comandos: ir para site, workspace ou aba, e executar ações).
- **Barra lateral (256px):** busca de sites, seção **Workspaces** (expansível, lista os sites, botão "Open workspace"), seção **All sites** e rodapé com "Import from FileZilla" e "Settings".
- **Aba de arquivos:** painel duplo local ↔ remoto. Cada lado tem um breadcrumb clicável e uma tabela (Name, Size, Modified e, no remoto, Permissions). Pastas recebem o sufixo `/`. Há arrastar e soltar entre painéis e a partir do gerenciador de arquivos do sistema. Ações: criar pasta, renomear, excluir (com confirmação), chmod e editar.
- **Aba de terminal:** terminal xterm-256color com truecolor, scrollback com busca, copiar/colar, redimensionamento propagado ao PTY, fonte e tema configuráveis. Quando a conexão cai, aparece uma faixa inline com "Reconnect".
- **Fila de transferências:** painel inferior recolhível, com abas Active/Queued/Failed/Done, velocidade total e número de workers. Cada linha tem direção, caminho, site, barra fina, %, velocidade e ações. Erros aparecem inline com Retry/Skip.
- **Barra de status:** estado da sessão ativa, `user@host:port`, método de autenticação e estado do workspace.
- **Log técnico:** painel opcional, recolhido por padrão.

**Estados obrigatórios** em toda região que carrega dados: carregando (skeleton), vazio (com a ação que preenche), erro (o que quebrou + ação) e cheio.

**Widget de lista virtualizada:** um widget próprio que renderiza apenas as linhas visíveis, com altura de linha fixa. É requisito para a meta de 100 mil entradas.

## 9. Desempenho (metas da v1)

| Métrica | Meta |
|---|---|
| Tempo de abertura até a janela interativa | < 500 ms |
| Memória ociosa (sem sessões) | < 80 MB |
| Lista remota de 100 mil entradas | rolagem sem travar (≥ 60 fps) |
| SFTP 1 GB com 50 ms de RTT | ≥ 90% do throughput do `scp` na mesma condição |
| 10 mil arquivos pequenos (SFTP e FTP) | não pior que o `lftp` com o mesmo paralelismo |
| Atualização de progresso para a UI | ≤ 10 Hz agregados |

## 10. Erros

- O núcleo usa um enum `hoprs_core::Error` com **categoria** (`Network`, `Auth`, `Permission`, `Conflict`, `Protocol`, `HostKey`, `Tls`, `Io`), **mensagem legível** e **detalhe técnico**.
- Onde cada erro aparece: transferência → linha da fila; conexão → aba do site + barra de status; ação de arquivo → mensagem inline no painel. **Nada de modais em cascata.** Modal só para decisões de segurança (host key, certificado).

## 11. Testes

- **Unitários** em todos os crates do núcleo.
- **`MemFs` + injeção de falhas** (queda, lentidão, permissão negada) para o motor de transferências, de forma determinística.
- **Integração** via `docker compose`: OpenSSH (SFTP + shell), vsftpd (FTP + FTPS explícito) e pure-ftpd (FTPS implícito, passivo com NAT). Cobre conectar, listar, transferir (pequeno, grande, pasta), retomar após kill, chmod, rename, known_hosts (novo e alterado) e certificado autoassinado.
- **Import:** fixtures de `sitemanager.xml` (versões diferentes do FileZilla, com pastas, protocolos variados).
- **Benchmarks:** `criterion` para o motor. Benchmarks de integração com `tc netem` para as metas da seção 9, rodando em job separado na CI.
- **GUI:** funções `update` testadas como funções puras. Verificação visual manual por release, com checklist.
- **CI (GitHub Actions, Ubuntu):** `cargo fmt --check`, `clippy -D warnings`, testes unitários, integração (Docker) e benchmarks.

## 12. Ordem sugerida de construção (insumo para o plano)

1. Workspace Cargo, `hoprs-core`, CI básica.
2. `hoprs-remote` com `MemFs` + `hoprs-transfer` (fila, workers, retomada, conflitos), totalmente testado sem rede.
3. `SftpFs` + `hoprs-ssh` (auth, known_hosts) + testes Docker.
4. `FtpFs` (FTP/FTPS, pool, passivo/NAT) + testes Docker.
5. `hoprs-vault` (TOML, keyring, export/import) + `hoprs-import` (FileZilla).
6. `hoprs-app`: shell, tema, barra lateral, workspaces, aba de arquivos com lista virtualizada, fila.
7. Aba de terminal.
8. Edição remota, paleta Ctrl+K, polimento, benchmarks de desempenho.
