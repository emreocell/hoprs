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
