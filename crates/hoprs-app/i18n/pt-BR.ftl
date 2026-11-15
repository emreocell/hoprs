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
policy-overwrite-if-newer = sobrescrever se mais novo
policy-resume = retomar
session-policy = Conflitos: { $name } (nesta sessão)
policy-if-newer = se mais novo
session-policy-tag = { $name }: sessão
queue-unavailable = Fila de transferências indisponível: { $reason }. As transferências desta sessão não sobreviverão a uma reinicialização.
skip-exists = já existe
skip-not-newer = não é mais novo
skip-already-complete = já está completo
skipped = Ignorado: { $reason }

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
