import type { Dict } from "./en";

// Portuguese (Brazil) copy for /pt-br/. Product terms stay in English where Brazilian developers
// use them (SSH, SFTP, FTP, terminal, upload, changelog). macOS and Windows steps use the names
// those systems show in pt-BR.
export const ptBr: Dict = {
  meta: {
    title: "hoprs: cliente SSH, SFTP e FTP para Linux, macOS e Windows",
    description:
      "hoprs é um cliente SSH, SFTP, FTP e FTPS gratuito e de código aberto, com terminal integrado. Importa seus sites do FileZilla. Para Linux, macOS e Windows.",
    ogLocale: "pt_BR",
    ogImageAlt: "hoprs, SSH, SFTP e FTP numa janela só: a aba de arquivos com uma pasta local e a mesma pasta num servidor.",
  },
  nav: {
    label: "Principal",
    home: "hoprs, início",
    download: "Baixar",
    otherLanguage: "English",
  },
  hero: {
    headline: "SSH, SFTP e FTP numa janela só.",
    sentence: "Um cliente de arquivos rápido e nativo para Linux, macOS e Windows.",
    downloadFor: "Baixar para",
    shotAlt:
      "Aba de arquivos do hoprs: dois arquivos locais sendo arrastados para a pasta do servidor de build, que ganha um contorno tracejado e o rótulo Upload 2 items to /home/alice/web-app; embaixo, o painel de comparação de pastas listando o que difere entre as duas pastas e um upload em andamento na fila de transferências.",
  },
  fast: {
    title: "Leve na memória, rápido na rede.",
    label: "Desempenho medido",
    startup: "até o primeiro frame",
    idle: "de memória em repouso",
    rows: (rows: string) => `p95 de renderização e layout, ${rows} linhas`,
    sftp: "SFTP, 1 GiB com 50 ms de RTT",
    measuredOn: "Medido num runner do GitHub Actions em",
    how: "Como medimos",
  },
  connection: {
    title: "Uma conexão, duas abas.",
    lead: "Uma aba de terminal e uma de arquivos usam a mesma conexão SSH. Abra mais abas na mesma sessão.",
    shotAlt:
      "Aba de terminal do hoprs na mesma conexão SSH, mostrando ls -la da pasta do projeto no servidor enquanto o upload continua na fila embaixo.",
  },
  filezilla: {
    title: "Seus sites do FileZilla vêm junto.",
    label: "O que muda quando você troca o FileZilla pelo hoprs",
    kept: "sites salvos, um clique para abrir",
    added: [
      "importa o sitemanager.xml, com um relatório do que ficou de fora",
      "senhas no chaveiro do sistema",
      "um único arquivo de backup criptografado",
      "terminal e arquivos na mesma conexão",
    ],
  },
  openSource: {
    title: "Código aberto.",
    lead: "MIT OR Apache-2.0. Feito com iced, russh, suppaftp e alacritty_terminal.",
    link: "Leia o código no GitHub",
  },
  install: {
    title: "É só instalar.",
    systems: "Sistema operacional",
    method: "Forma de instalação",
    source: "compilar",
    copy: "Copiar",
    copied: "Copiado",
    pressCtrlC: "Pressione Ctrl+C para copiar",
    linuxLead: "Ubuntu 24.04 ou mais recente, amd64.",
    macLead: "macOS 11 ou mais recente, Apple silicon e Intel.",
    download: "Baixar",
    macSteps: [
      "Abra a imagem de disco e arraste o hoprs para Aplicativos.",
      "O app não é notarizado. Abra o hoprs uma vez e vá em Ajustes do Sistema → Privacidade e Segurança → Abrir Mesmo Assim.",
    ],
    macTerminal: "Ou, no Terminal:",
    checkWith: "Confira com o",
    windowsLead: "Windows, 64 bits.",
    portable: "Portátil",
    windowsSteps: [
      "Rode o instalador ou descompacte a versão portátil onde quiser.",
      "O app não é assinado. Se o SmartScreen mostrar “O Windows protegeu o computador”, clique em Mais informações → Executar assim mesmo.",
    ],
  },
  faq: {
    title: "Perguntas frequentes.",
    items: [
      { q: "Em quais sistemas ele roda?", a: "Linux (Ubuntu 24.04 ou mais recente, amd64), macOS 11 ou mais recente e Windows 64 bits." },
      { q: "O macOS ou o Windows diz que o app não é confiável. É normal?", a: "É. Os builds não são notarizados nem assinados com um certificado pago. No macOS, abra Ajustes do Sistema → Privacidade e Segurança e clique em Abrir Mesmo Assim; ou rode xattr -dr com.apple.quarantine /Applications/hoprs.app. No Windows, em “O Windows protegeu o computador”, clique em Mais informações → Executar assim mesmo. Antes, confira o download com o SHA256SUMS." },
      { q: "Onde ficam as senhas?", a: "No chaveiro do sistema: Secret Service no Linux, Keychain (Acesso às Chaves) no macOS, Gerenciador de Credenciais no Windows. Os arquivos de sites não guardam segredos." },
      { q: "Dá para usar FTP sem criptografia?", a: "Dá. Ele aparece como ftp (unencrypted). FTPS, explícito e implícito, também é suportado." },
      { q: "Ele importa do FileZilla?", a: "Importa. Ele lê o sitemanager.xml e mostra o que foi importado e o que ficou de fora." },
      { q: "Ele coleta dados?", a: "O app, não. Ele não tem conta e só se conecta aos servidores que você adicionar, além de uma requisição ao GitHub para ver se há versão nova (dá para desligar em Configurações).  Este site conta as visitas com o Cloudflare Web Analytics, que não usa cookies nem guarda dados pessoais." },
      { q: "Como atualizo?", a: "O hoprs consulta o GitHub por uma versão nova ao abrir e uma vez por dia, e mostra um cartão com o botão Atualizar (Configurações › Geral desliga a consulta). Atualizar baixa o arquivo certo, confere com o SHA256SUMS da versão e abre o instalador do sistema. Você também pode baixar a versão nova e instalar por cima da antiga: no Linux, sudo apt install ./hoprs_<versão>-1_amd64.deb; no macOS, arraste o hoprs para Aplicativos e substitua; no Windows, rode o novo .msi (ele substitui a versão anterior) ou descompacte a versão portátil por cima da pasta antiga. Sites, workspaces, configurações e senhas salvas são mantidos." },
      { q: "Como desinstalo?", a: "Linux: sudo apt remove hoprs. macOS: arraste o hoprs para fora de Aplicativos. Windows: remova em Aplicativos instalados. Seus dados ficam; para apagá-los também, exclua ~/.config/hoprs e ~/.local/share/hoprs (Linux), ~/Library/Application Support/hoprs (macOS) ou %APPDATA%\\hoprs (Windows), e as entradas do chaveiro do serviço hoprs/<site-id> (no Gerenciador de Credenciais do Windows aparecem como password.hoprs/<site-id> e key-passphrase.hoprs/<site-id>)." },
      { q: "Como reporto um bug?", a: "Abra uma issue no GitHub. Para vulnerabilidades, use o reporte privado do GitHub (Security Advisories)." },
    ],
  },
  footer: {
    label: "Rodapé",
    releases: "Versões",
    changelog: "Changelog",
    security: "Segurança",
    license: "Licença",
    licensed: "Licenciado sob MIT OR Apache-2.0.",
    madeBy: "Feito por",
    updated: "Atualizado em",
  },
};
