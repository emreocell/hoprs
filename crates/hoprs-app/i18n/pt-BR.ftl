# Texto da interface do hoprs, português do Brasil. As chaves e as variáveis são as de
# en.ftl; só o texto muda. Veja o cabeçalho de en.ftl para o formato.

app-title = hoprs
already-running = O hoprs já está em execução
cannot-open-profiles = Não foi possível abrir os sites salvos

# Um número de sites salvos: "1 site", "3 sites".
site-count =
    { $count ->
        [0] { $count } sites
        [one] { $count } site
       *[other] { $count } sites
    }

# A página de configurações: o idioma da interface e as opções. Os nomes dos idiomas são
# escritos no próprio idioma, então são iguais em todos os catálogos.
settings-language = Idioma
language-system = Sistema
language-english = English
language-pt-br = Português (Brasil)

## Top bar

brand = hoprs
no-sessions = Nenhuma sessão aberta
go-to-anything = Ir para qualquer coisa
go-to-anything-key = Ctrl K

## The Ctrl+K palette

palette-placeholder = Ir para uma aba, site ou workspace, ou executar uma ação
palette-workspaces-placeholder = Abrir workspace
palette-no-results = Nenhum resultado
palette-group-action = Ação
palette-group-tab = Aba
palette-group-site = Site
palette-group-workspace = Workspace
palette-kind-terminal = terminal
palette-kind-files = arquivos
palette-toggle-queue = Mostrar ou ocultar a fila
palette-toggle-log = Mostrar ou ocultar o log
palette-close-tab = Fechar aba
palette-open-workspace = Abrir workspace…
palette-edit = Abrir no editor
palette-upload = Fazer upload da seleção
palette-download = Fazer download da seleção
palette-copy = Copiar
palette-paste = Colar
palette-find = Buscar no histórico
palette-key-copy = Ctrl Shift C
palette-key-paste = Ctrl Shift V
palette-key-find = Ctrl Shift F

## Sidebar

filter-sites = Filtrar sites
workspaces = Workspaces
all-sites = Todos os sites
new = + Novo
no-workspaces = Nenhum workspace ainda
no-sites = Nenhum site ainda
import-filezilla = Importar do FileZilla
settings = Configurações
new-site = Novo site
no-matches = Nenhum site encontrado
open-workspace = Abrir workspace
edit = Editar
site-missing = Site ausente
cannot-load-sites = Não foi possível carregar os sites salvos
retry = Tentar novamente
unreadable-entry = Não foi possível ler esta entrada (escrita por uma versão mais recente do hoprs?)
open-profile-folder = Abrir a pasta do sites.toml
cannot-open-folder = Não foi possível abrir a pasta: { $reason }
workspace-site-count-partial =
    { $total ->
        [one] { $present } de { $total } site
       *[other] { $present } de { $total } sites
    }
unreadable-name-fallback = Entrada { $number } em { $file }

## Site form

site-form-new = Novo site
site-form-edit = Editar site
field-name = Nome
field-name-hint = Usa o host por padrão
field-host = Host
field-port = Porta
field-user = Usuário
field-protocol = Protocolo
field-password = Senha
password-optional = Opcional
password-replace = Digite para substituir
password-saved = Salva no keyring do sistema
forget-password = Esquecer a senha salva
ssh-options = SSH
use-agent = Usar ssh-agent
key-file = Arquivo de chave
key-file-hint = Caminho de uma chave privada (opcional)
open-as = Abrir como
open-as-terminal = Terminal
open-as-files = Arquivos
open-as-both = Terminal e arquivos
ftp-options = FTP
ftp-mode = Modo de transferência
ftp-passive = Passivo
ftp-active = Ativo
nat-workaround = Ignorar o endereço passivo enviado pelo servidor (NAT)
start-folders = Pastas iniciais
remote-dir = Pasta remota
local-dir = Pasta local
folder-hint = Opcional
host-required = O host é obrigatório
port-invalid = A porta deve ser um número de 1 a 65535
save = Salvar
saving = Salvando…
cancel = Cancelar
delete = Excluir
keep = Manter
delete-site-confirm = Excluir este site? Ele será removido de todos os workspaces e a senha salva será excluída.

## Workspace form

workspace-form-new = Novo workspace
workspace-form-edit = Editar workspace
workspace-sites = Sites
workspace-no-sites = Nenhum site ainda. Crie um site primeiro.
name-required = O nome é obrigatório
delete-workspace-confirm = Excluir este workspace? Os sites dele são mantidos.

## Center

no-session-title = Nenhuma sessão aberta
no-session-hint = Escolha um site na barra lateral para conectar.
keyring-unavailable = O keyring do sistema estava indisponível, então a senha salva não foi usada.
edit-site = Editar site

## Tabs

close-tab = ×
new-terminal-tab = Nova aba de terminal
new-files-tab = Nova aba de arquivos
terminal-overflowed = A saída chegou rápido demais e o shell foi encerrado
terminal-reopen = Reabrir
terminal-restart = Reiniciar o shell
terminal-shell-closed = Shell fechado
terminal-open-failed = Não foi possível abrir um shell
terminal-retry = Tentar novamente
terminal-waiting-for-connection = Aguardando a conexão
terminal-input-dropped = Parte da entrada foi descartada: o shell não a processou rápido o bastante
terminal-reconnected = — reconectado —
terminal-search-placeholder = Buscar no histórico
terminal-search-next = Próximo
terminal-search-previous = Anterior
terminal-search-close = Fechar
terminal-search-no-match = Nenhum resultado
shell-exited-code = Shell encerrado (código { $code })
shell-exited = Shell encerrado

## File panes
local = Local
col-name = Nome
col-size = Tamanho
col-modified = Modificado
col-permissions = Permissões
sort-ascending = ↑
sort-descending = ↓
parent-dir = ..
path-separator = /
home-dir = ~
no-size = -
empty-folder = Esta pasta está vazia
reconnect = Reconectar
size-unit-b = B
size-unit-kb = KB
size-unit-mb = MB
size-unit-gb = GB
size-unit-tb = TB
size-unit-pb = PB
size-unit-eb = EB
month-1 = jan
month-2 = fev
month-3 = mar
month-4 = abr
month-5 = mai
month-6 = jun
month-7 = jul
month-8 = ago
month-9 = set
month-10 = out
month-11 = nov
month-12 = dez

## Queue
transfers = Transferências
queue-active = Ativas
queue-queued = Na fila
queue-failed = Com falha
queue-done = Concluídas
queue-hide = Ocultar
queue-show = Mostrar
no-transfers = Nenhuma transferência
no-transfers-hint = Selecione arquivos e pressione Upload ou Download, arraste-os para o outro painel ou solte-os aqui.
queue-loading = Carregando transferências…
nothing-active = Nada em andamento
nothing-queued = Nada aguardando
nothing-failed = Nenhuma falha
nothing-done = Nada concluído ainda
clear-finished = Limpar concluídas e com falha
clear-finished-short = Limpar concluídas

## Technical log panel
log = Log
log-error = Erro
log-warn = Aviso
log-info = Info
log-debug = Debug
filter-log = Filtrar o log
copy-shown = Copiar exibidas
clear = Limpar
clear-hint = Esvazia só este painel; novas linhas continuam chegando.
log-level-hint = Mostra linhas com pelo menos esta gravidade. Só info e acima são registradas, a menos que o hoprs seja iniciado com RUST_LOG (por exemplo, RUST_LOG=debug).
no-log-lines = Nada registrado ainda
no-log-matches = Nenhuma linha corresponde
log-lines =
    { $count ->
        [0] { $count } linhas
        [one] { $count } linha
       *[other] { $count } linhas
    }
upload-arrow = ↑
download-arrow = ↓
waiting-for-worker = Aguardando um worker
starting = Iniciando…
transferring = Transferindo…
dismissed = Dispensada
paused = Pausada
cancelled = Cancelada
transfer-done = Concluída
file-exists = O arquivo existe —
overwrite = Sobrescrever
resume = Retomar
skip = Pular
pause = Pausar
apply-to-all = Aplicar a todos (nesta sessão)
site-paused = Pausada — conexão perdida
drop-needs-site = Abra um site conectado para soltar arquivos nele
drop-pane-busy = A pasta remota está ocupada ou ainda não foi listada; solte de novo em instantes
drop-unsupported = Não é possível fazer upload deste caminho
total-workers =
    { $workers ->
        [0] no total, { $workers } workers
        [one] no total, { $workers } worker
       *[other] no total, { $workers } workers
    }
policy-ask = perguntar
policy-overwrite = sobrescrever
policy-skip = pular
policy-overwrite-if-newer = sobrescrever se mais recente
policy-resume = retomar
session-policy = Conflitos: { $name } (nesta sessão)
policy-if-newer = se mais recente
session-policy-tag = { $name }: sessão
queue-unavailable = Fila de transferências indisponível: { $reason }. As transferências desta sessão não sobreviverão a uma reinicialização.
skip-exists = já existe
skip-not-newer = não é mais recente
skip-already-complete = já está completo
skipped = Ignorada: { $reason }

## Status bar
not-connected = Não conectado

## Status bar and connection states
connecting = Conectando…
connected = Conectado
waiting-for-you = Aguardando você
closing = Salvando as abas e fechando…
close-workspace = Fechar abas
reconnecting-now = Reconectando…
saved-folder-gone = A pasta salva { $saved } não existe mais; mostrando a pasta mais próxima.
reconnecting = Reconectando ({ $attempt }/{ $of })
disconnected = Desconectado — { $reason }
auth-none = sem autenticação
auth-agent = { $key_type } via ssh-agent
auth-key-file = chave { $key_type }
auth-password = senha
auth-keyboard-interactive = keyboard-interactive
ftp-auth-ftp = FTP (sem criptografia)
ftp-auth-ftps-explicit = FTPS explícito
ftp-auth-ftps-implicit = FTPS implícito
workspace-status = { $connected } de { $sites } em { $name }

## File actions
new-folder = Nova pasta
rename = Renomear
permissions = Permissões…
new-folder-short = Pasta
permissions-short = Perm…
key-new-folder = F7
key-rename = F2
key-delete = Del
key-edit = F4
upload = Upload
download = Download
key-transfer = Ctrl ↵
new-folder-name = Nova pasta
name-empty = O nome não pode ficar vazio
name-invalid = O nome não pode conter / nem ser . ou ..
name-invalid-windows = O nome não pode conter \ neste disco
name-spaces = O nome não pode começar nem terminar com espaço
mixed-permissions = Permissões mistas: mostrando as do primeiro item
name-taken = Já existe algo com este nome aqui
dismiss = Dispensar
creating-folder = Criando pasta…
renaming = Renomeando…
changing-permissions = Alterando permissões…
delete-folders-note = As pastas são excluídas com tudo que há dentro.
permissions-title = Permissões
perm-owner = Dono
perm-group = Grupo
perm-others = Outros
perm-read = Leitura
perm-write = Escrita
perm-execute = Execução
perm-octal = Octal
octal-invalid = Três ou quatro dígitos de 0 a 7
apply = Aplicar
delete-question-one = Excluir { $name }?
delete-question-many = Excluir { $count } itens?
and-more = e mais { $count }
deleting =
    { $done ->
        [0] Excluindo…
        [1] Excluindo… 1 item
       *[other] Excluindo… { $done } itens
    }
deleted-then =
    { $done ->
        [1] 1 item excluído; depois: { $error }
       *[other] { $done } itens excluídos; depois: { $error }
    }

## Security and credential dialogs
host-key-title = Chave de host desconhecida
host-key-intro = O hoprs nunca se conectou a este servidor. Confira a impressão digital com o administrador antes de confiar nele.
trust-and-connect = Confiar e conectar
cert-untrusted-title = Certificado não confiável
cert-untrusted-intro = Não foi possível verificar o certificado do servidor. Confira a impressão digital com o administrador antes de confiar nele.
cert-changed-title = Certificado alterado
cert-changed-intro = O certificado do servidor não é o mesmo em que você confiou antes. Alguém pode estar interceptando a conexão.
trust-certificate = Confiar neste certificado
label-host = Host
label-account = Conta
label-key-type = Tipo de chave
label-fingerprint = Impressão digital (SHA-256)
label-reason = Motivo
label-trusted-before = Confiado anteriormente
label-presented-now = Apresentado agora
label-key-file = Chave
password-title = Senha
passphrase-title = Passphrase da chave
field-passphrase = Passphrase
kbd-title = Autenticação
save-in-keyring = Salvar no keyring do sistema
save-in-keyring-hint = Salvo somente depois que a conexão for bem-sucedida
connect = Conectar
unlock = Desbloquear
continue = Continuar
# Under a dialog, when others wait behind it: "1 more waiting".
dialogs-waiting = Mais { $count } na fila
# The warning of a host known with other key types.
other-key-types = Este host é conhecido com outro tipo de chave ({ $types }). Alguém pode estar se passando por ele.
# Why a certificate needs the user's decision.
cert-reason-expired = Expirado
cert-reason-not-yet-valid = Ainda não é válido
cert-reason-wrong-name = Emitido para outro nome
cert-reason-unknown-issuer = Autoridade desconhecida (autoassinado ou emissor não confiável)

## Changed host key, in the tab
host-key-changed-title = A chave do host mudou
host-key-changed-body = O servidor apresentou uma chave diferente da registrada para ele. Isso acontece quando o servidor foi reinstalado ou quando alguém intercepta a conexão. O hoprs não vai se conectar enquanto a entrada antiga estiver no known_hosts.
host-key-changed-session-body = O servidor apresentou uma chave diferente da que esta sessão verificou ao se conectar. Isso acontece quando o servidor foi reinstalado ou quando alguém intercepta a conexão. Uma nova sessão confere a chave com o known_hosts novamente.
remove-old-entry = Remover a entrada antiga…
close-and-reconnect = Fechar e reconectar
reading-known-hosts = Lendo o known_hosts…
removing-known-hosts = Removendo…
remove = Remover
connect-again = Conectar de novo
no-known-hosts-entry = O known_hosts não tem mais nenhuma entrada para este host. Conecte de novo para conferir a nova chave.
# "known_hosts line 3".
known-hosts-line = Linha { $line } do known_hosts
# The removal question: "Remove 2 lines for [h]:2222 from known_hosts? …".
remove-known-hosts-question =
    { $lines ->
    [1] Remover 1 linha de { $host } do known_hosts? Na próxima conexão, o hoprs pede que você confie na nova chave.
   *[other] Remover { $lines } linhas de { $host } do known_hosts? Na próxima conexão, o hoprs pede que você confie na nova chave.
    }
# The warning for lines that also name other hosts.
also-covers-one-host = A linha { $line } também cobre outros hosts (um padrão ou uma lista de nomes): removê-la esquece esta chave para eles também.
also-covers-other-hosts = As linhas { $lines } também cobrem outros hosts (padrões ou listas de nomes): removê-las esquece estas chaves para eles também.
# A connected tab whose site's host or port changed (an edit, a restore).
site-endpoint-changed = O host ou a porta do site mudou: esta sessão foi fechada. Reconecte para usar as novas configurações.

## Import from FileZilla
import-title = Importar do FileZilla
import-step-file = 1  Arquivo
import-step-folders = 2  Pastas
import-step-report = 3  Relatório
import-file-label = Arquivo do gerenciador de sites do FileZilla
import-file-hint = Normalmente o sitemanager.xml, na pasta de configuração do FileZilla. As senhas vão para o keyring do sistema.
import-path-required = Escolha o arquivo sitemanager.xml
choose-file = Escolher…
next = Avançar
back = Voltar
import-folders-question = Transformar as pastas do FileZilla em workspaces?
import-folders-hint = Sim: um workspace por pasta, com o caminho dela como nome e os sites da pasta. Não: o caminho da pasta vai antes do nome de cada site ("Work/Alpha").
import-folders-yes = Sim, criar workspaces
import-folders-no = Não, manter nos nomes
import-running = Importando…
import-imported = Importados
import-skipped = Ignorados
import-workspaces-created = Workspaces criados
import-workspaces-not-created = Workspaces não criados
import-failures = Falhas
import-none = Nenhum
import-failed-keep-source = Algumas entradas não puderam ser salvas: mantenha o sitemanager.xml e importe de novo depois de corrigir.
import-passwords-keep-source = O keyring recusou as senhas destes sites: mantenha o sitemanager.xml, ele ainda as tem.
delete-sitemanager = Excluir o sitemanager.xml
delete-sitemanager-confirm = Excluir o sitemanager.xml do FileZilla? O FileZilla não terá mais estes sites.
deleting-sitemanager = Excluindo…
sitemanager-deleted = sitemanager.xml excluído
done = Concluído
pick-sitemanager = Escolha o sitemanager.xml do FileZilla
pick-filter-xml = Arquivos XML

## Settings
settings-title = Configurações
settings-transfers = Transferências
settings-workers = Workers por site
settings-workers-hint = Transferências paralelas por site, de 1 a 16. Vale para as próximas conexões.
settings-policy = Quando um arquivo já existe
settings-policy-hint = Vale para as próximas transferências. Perguntar mostra a escolha na fila.
settings-keepalive = Keepalive
settings-keepalive-hint = Com que frequência uma conexão ociosa é verificada, para que servidores e roteadores a mantenham aberta. Vale para as próximas conexões.
# A keepalive interval: "30 s".
seconds = { $secs } s
settings-unreadable = Não foi possível ler o settings.toml, então valem os padrões. As alterações duram até o hoprs fechar e não são salvas, para manter o arquivo como está: corrija ou apague o arquivo para salvar as configurações.
settings-save-failed = Não foi possível salvar o settings.toml
settings-terminal = Terminal
settings-font-size = Tamanho da fonte
settings-scrollback = Linhas de histórico
settings-scrollback-hint = Vale para os terminais abertos daqui em diante.
settings-theme = Tema
theme-hoprs-dark = hoprs escuro
settings-policy-ask = Perguntar
settings-policy-overwrite = Sobrescrever
settings-policy-skip = Pular
settings-policy-overwrite-if-newer = Sobrescrever se mais recente
settings-policy-resume = Retomar
decrease = −
increase = +

## Backup
backup = Backup
backup-hint = Um arquivo criptografado com todos os sites, workspaces e senhas salvas. Guarde a senha: sem ela o backup não pode ser aberto.
export-backup = Exportar backup…
restore-backup = Restaurar backup…
backup-file = Arquivo
backup-password = Senha
backup-password-confirm = Senha de novo
backup-path-required = Escolha um arquivo
backup-password-required = Digite uma senha
backup-passwords-differ = As senhas não coincidem
export = Exportar
export-overwrite-confirm = Este arquivo já existe. Sobrescrevê-lo com o novo backup?
restore = Restaurar
exporting = Criptografando o backup… isso leva alguns segundos
restoring = Descriptografando o backup… isso leva alguns segundos
restore-mode = Modo
restore-merge = Mesclar
restore-merge-hint = Adiciona o que é novo; os sites e workspaces que você tem continuam.
restore-replace = Substituir
restore-replace-hint = Tudo passa a ser o do backup: sites, workspaces e senhas que não estão nele são removidos.
restore-replace-confirm = Substituir todos os sites, workspaces e senhas salvas pelos do backup?
replace = Substituir
export-failed = O backup não foi gravado
restore-failed = Nada foi restaurado
backup-secret-errors = Senhas não restauradas
backup-missing-secrets = Salvos sem a senha (ausente do keyring)
backup-skipped-unreadable = Não exportadas (entradas ilegíveis)
backup-skipped-sites = Sites que já existem (mantidos)
backup-skipped-workspaces = Workspaces que já existem (mantidos)
backup-file-name = hoprs-backup.hoprs
pick-export = Salvar o backup do hoprs
pick-restore = Escolher um backup do hoprs
# "Exported 12 sites, 3 workspaces and 9 passwords."
export-done = Exportados { $sites }, { $workspaces } e { $secrets }.
workspace-count =
    { $count ->
    [0] { $count } workspaces
    [one] { $count } workspace
   *[other] { $count } workspaces
    }
password-count =
    { $count ->
    [0] { $count } senhas
    [one] { $count } senha
   *[other] { $count } senhas
    }
# "Restored 2 sites, 1 workspace and 2 passwords."
restore-done = Restaurados { $sites }, { $workspaces } e { $secrets }.

## Remote editing
# The Edit action (remote pane).
edit-file = Editar
edit-no-folders = Não é possível editar pastas
edit-no-links = Não é possível editar links
edit-too-large = Não é possível editar arquivos com mais de 50 MB
waiting-for-connection = Aguardando a conexão…
remote-may-be-truncated = A cópia do servidor pode ficar incompleta até lá.
changed-on-server = Alterado no servidor desde que você o abriu
keep-remote = Manter a versão remota
save-a-copy = Salvar uma cópia
reload-in-editor = Agora é a versão do servidor: recarregue-a no editor.
upload-and-close = Fazer upload das alterações pendentes e fechar
close-keep-copies = Fechar e manter as cópias locais
upload-and-quit = Fazer upload e sair
quit-keep-copies = Sair e manter as cópias locais
checking-edits = Verificando as edições remotas…
stop-keep-copies = Parar e manter as cópias locais
# While "Upload" runs (an upload under way has no bound but its protocol's timeouts).
uploading-edits = Fazendo upload… (Parar para manter as cópias locais)
# The kept notice, before a remote path: "Server copy of".
server-copy-of = Cópia do servidor de
# …after it.
may-be-incomplete = pode estar incompleta
# The kept notice after a close that did not ask (the site moved, "Close and
# reconnect"): the editor's file is no longer linked to the server.
kept-saves-stopped = A sessão foi fechada: as próximas gravações no editor não serão enviadas. Suas alterações estão na cópia recuperada; abra o arquivo de novo para continuar editando.
edit-one-file = Selecione um arquivo para editar
uploaded = Enviado
editing = Editando
opening = Abrindo
close-workspace-edits-title = Algumas edições remotas deste workspace não foram enviadas
keeping-copies = Salvando as cópias locais…
close-edits-title = Algumas edições remotas não foram enviadas
quit-edits-title = Algumas edições remotas não foram enviadas
quit-edits-hint = A janela continua aberta até você escolher. Nada é excluído.
edit-unsent = não enviado
edit-conflict = alterado no servidor
editor-command = Editor
# Windows' example names `code.cmd`: the command runs without a shell, which does not
# find `.cmd` shims from a bare name.
editor-command-hint-windows = system abre os arquivos remotos no editor padrão do sistema. Ou digite um comando; o caminho do arquivo é adicionado no fim. Por exemplo: notepad++.exe, code.cmd --wait.
editor-command-hint-macos = system abre os arquivos remotos no editor padrão do sistema. Ou digite um comando; o caminho do arquivo é adicionado no fim. Por exemplo: open -W -a "Visual Studio Code" (um caminho completo também funciona).
editor-command-hint = system abre os arquivos remotos no editor padrão do sistema. Ou digite um comando; o caminho do arquivo é adicionado no fim. Por exemplo: gedit, code --wait.
# Next to the editor field when it holds `system`.
editor-system-default = Padrão do sistema
editor-command-empty = Digite um comando
settings-editing = Edição remota
# "Upload failed: Permission denied".
upload-failed = Falha no upload: { $reason }
# "Your version was saved to <path>".
your-version-at = Sua versão foi salva em { $path }
# Where "keep" puts the copies: "Unsent changes are copied to <dir>".
copies-go-to = As alterações não enviadas são copiadas para { $dir }
# "2 files could not be uploaded".
not-uploaded =
    { $count ->
    [0] Não foi possível fazer upload de { $count } arquivos
    [one] Não foi possível fazer upload de { $count } arquivo
   *[other] Não foi possível fazer upload de { $count } arquivos
    }
# The persistent notice after a close kept unsent edits: "Kept 2 unsent edits as
# recovered copies in" (the folder follows, in mono).
kept-notice =
    { $count ->
    [0] Mantidas { $count } edições não enviadas como cópias recuperadas em
    [one] Mantida { $count } edição não enviada como cópia recuperada em
   *[other] Mantidas { $count } edições não enviadas como cópias recuperadas em
    }
# The delete confirmation of a site with files open for editing.
delete-site-edits =
    { $count ->
    [0] { $count } arquivos deste site estão abertos para edição: as alterações não enviadas serão mantidas como cópias recuperadas, nunca excluídas.
    [one] { $count } arquivo deste site está aberto para edição: as alterações não enviadas serão mantidas como cópia recuperada, nunca excluídas.
   *[other] { $count } arquivos deste site estão abertos para edição: as alterações não enviadas serão mantidas como cópias recuperadas, nunca excluídas.
    }
