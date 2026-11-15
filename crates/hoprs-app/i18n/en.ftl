# hoprs interface text, English. This file is the reference for every translation.
#
# Format (Fluent, https://projectfluent.org):
#   key = text                 one message per key; keys are kebab-case.
#   # comment                  explains a message to translators; it is never shown.
#   { $name }                  a value the app fills in. Keep the variable names exactly
#                              as they are; the order and the words around them may change.
#   Plurals use a selector, one line per form, the line with * is the default:
#       site-count =
#           { $count ->
#               [one] { $count } site
#              *[other] { $count } sites
#           }
#   Keep technical names (SSH, SFTP, FTP, FTPS) as they are.
# pt-BR.ftl must define the same keys with the same variables.

app-title = hoprs
already-running = hoprs is already running
cannot-open-profiles = Cannot open the saved sites

# A number of saved sites: "1 site", "3 sites".
site-count =
    { $count ->
        [one] { $count } site
       *[other] { $count } sites
    }
