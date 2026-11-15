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
palette-edit = Editar no editor
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
unreadable-entry = Não foi possível ler esta entrada (escrita por um hoprs mais novo?)
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
delete-site-confirm = Excluir este site? Ele sai de todos os workspaces e a senha salva é excluída.

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
terminal-overflowed = A saída veio rápido demais e o shell foi encerrado
terminal-reopen = Reabrir
terminal-restart = Reiniciar o shell
terminal-shell-closed = Shell encerrado
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
