<p align="center">
  <img src="../assets/logo-full.svg" alt="srelens" width="380" />
</p>

<h3 align="center">A sala de controle do Kubernetes — feita em Rust, pronta para engenheiros e agentes de IA.</h3>

<p align="center">
  <a href="../../README.md">English</a> ·
  <a href="README.zh-CN.md">简体中文</a> ·
  <a href="README.ja.md">日本語</a> ·
  <a href="README.ko.md">한국어</a> ·
  <a href="README.es.md">Español</a> ·
  <b>Português</b> ·
  <a href="README.de.md">Deutsch</a> ·
  <a href="README.fr.md">Français</a>
</p>

<p align="center">
  <sub>Esta é uma tradução. O <a href="../../README.md">README em inglês</a> é a fonte da verdade; se os dois divergirem, vale o que está no inglês.</sub>
</p>

<p align="center">
  O srelens é um workspace de Kubernetes open source e local-first para SREs,
  engenheiros de plataforma e engenheiros de DevOps. Investigue, analise e aja com
  segurança em vários clusters — a partir de um <b>aplicativo desktop</b>, de uma
  <b>interface de terminal</b> ou de um <b>servidor MCP</b> que o seu agente de IA
  pode chamar. Os três rodam sobre o mesmo núcleo em Rust puro.
</p>

<p align="center">
  <a href="https://srelens.com">Site</a> ·
  <a href="#instalação">Instalação</a> ·
  <a href="#aplicativo-desktop">Aplicativo desktop</a> ·
  <a href="#interface-de-terminal-srectl">Interface de terminal</a> ·
  <a href="#servidor-mcp">Servidor MCP</a> ·
  <a href="../USAGE.md">Guia do usuário</a> ·
  <a href="../DEVELOPMENT.md">Guia do desenvolvedor</a> ·
  <a href="../../CONTRIBUTING.md">Contribuindo</a>
</p>

<p align="center">
  <a href="https://github.com/srelens/srelens/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/srelens/srelens?logo=github&color=eab308"></a>
  <a href="https://www.reddit.com/r/srelens/"><img alt="Reddit: r/srelens" src="https://img.shields.io/reddit/subreddit-subscribers/srelens?label=r%2Fsrelens&logo=reddit&logoColor=white&color=FF4500"></a>
  <a href="https://deepwiki.com/srelens/srelens"><img alt="Ask DeepWiki" src="https://img.shields.io/badge/Ask-DeepWiki-2563eb"></a>
</p>

<p align="center">
  <a href="https://github.com/srelens/srelens/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/srelens/srelens?display_name=release&label=release&color=22c55e"></a>
  <a href="https://github.com/srelens/srelens/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/srelens/srelens/total?label=downloads&color=3b82f6"></a>
  <a href="https://aur.archlinux.org/packages/srelens-bin"><img alt="AUR version" src="https://img.shields.io/aur/version/srelens-bin?label=aur&logo=archlinux&logoColor=white&color=1793d1"></a>
  <a href="../../LICENSE"><img alt="License: MIT" src="https://img.shields.io/github/license/srelens/srelens?color=675e80"></a>
  <a href="https://github.com/srelens/srelens/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/srelens/srelens/actions/workflows/ci.yml/badge.svg?branch=dev"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/srelens/srelens"><img alt="OpenSSF Scorecard" src="https://api.scorecard.dev/projects/github.com/srelens/srelens/badge"></a>
</p>

<p align="center">
  <img alt="Rust core" src="https://img.shields.io/badge/core-Rust-8b5cf6">
  <img alt="Tauri v2" src="https://img.shields.io/badge/desktop-Tauri_v2-e457c2">
  <img alt="Terminal UI" src="https://img.shields.io/badge/terminal-srectl-14b8a6">
  <img alt="MCP server" src="https://img.shields.io/badge/agents-MCP-fb923c">
  <img alt="Project status: stable" src="https://img.shields.io/badge/status-stable-22c55e">
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="../assets/screenshots/gui-overview-light.webp">
    <img alt="O aplicativo desktop do srelens no novo design: uma visão geral do cluster com capacidade dos nodes, workloads que não estão prontos e objetos por kind" src="../assets/screenshots/gui-overview-dark.webp" width="100%">
  </picture>
</p>

---

## Escolha sua interface

|  | Aplicativo desktop (GUI) | Interface de terminal (`srectl`) |
| --- | --- | --- |
| **O que é** | Uma janela nativa construída com Tauri v2 e React 19 | Um único binário autocontido construído com ratatui |
| **Ideal para** | Trabalho diário com clusters, edição de YAML, topologia, vários clusters lado a lado | Sessões SSH, jump hosts, fluxo só com teclado, memória muscular do k9s |
| **Roda em** | macOS, Linux, Windows — ou no navegador via [modo web](../WEB.md) | Linux (glibc e musl estático) e macOS em x86-64 e arm64; Windows em x86-64 |
| **Instalação** | [Baixe uma release](#instalação) | `brew install srelens/tap/srectl` ou [em uma linha](#instalação) |
| **IA** | Assistente integrado, além do servidor MCP em **Settings → MCP** (Configurações → MCP) | Assistente `:ai`, além do `srectl mcp` via stdio |

Os dois falam diretamente com os seus clusters usando as credenciais do seu
kubeconfig local. Nada passa por um serviço de nuvem do srelens.

## Por que o srelens?

Fazer troubleshooting no Kubernetes normalmente significa pular entre terminais,
dashboards, editores de YAML, logs e contextos de cluster. O srelens reúne esse
ciclo de investigação em um único workspace local-first.

- **Um só workspace, da investigação à ação** — navegue pelos recursos, leia
  eventos e YAML, acompanhe logs, abra shells, faça port forward e execute ações
  no cluster sem trocar de ferramenta.
- **Feito para engenheiros e agentes de IA** — as capacidades de backend que você
  usa no app são as mesmas expostas pelo servidor MCP integrado.
- **Acesso local-first aos clusters** — o srelens se conecta direto aos seus API
  servers com as credenciais do seu próprio kubeconfig.
- **Operações seguras** — ações destrutivas são identificadas e exigem
  confirmação, no app, no terminal e via MCP.
- **Pequeno e rápido** — um núcleo em Rust sobre a WebView do sistema
  operacional. O srelens `v0.5.0` saiu com um instalador para macOS de
  **17,1 MiB** e um `.deb` para Linux de **19,8 MiB**, contra 190,2 MiB e
  146,8 MiB do Freelens `v1.10.3` — cerca de **11× e 7× menores**. Os
  [baselines de desempenho](../PERFORMANCE.md) mostram como reproduzir esses
  números, inclusive onde a diferença é menor.
- **Open source** — licença MIT, com código, releases, issues e roadmap públicos.

## Aplicativo desktop

O aplicativo desktop é um workspace completo de Kubernetes em uma janela nativa.
As capturas de tela abaixo mostram o **novo design**, que está sendo liberado tela
por tela: ative-o em **Settings → Appearance → Design → New design**
(Configurações → Aparência → Design → Novo design) e volte ao anterior pelo mesmo
caminho quando quiser. São seis temas: Light, Paper, Dark, Midnight, Glass e
High contrast.

<table>
  <tr>
    <td width="50%"><img alt="Lista de Pods filtrada por um namespace, com um pod em crash loop marcado em vermelho" src="../assets/screenshots/gui-pods.webp"><br><sub><b>Recursos ao vivo</b> — tabelas alimentadas por watch, com filtros, colunas e ações em massa</sub></td>
    <td width="50%"><img alt="Detalhes de um pod com readiness, restarts, CPU, memória e ações rápidas para logs, shell e port forward" src="../assets/screenshots/gui-pod-detail.webp"><br><sub><b>Detalhes do recurso</b> — status, eventos, owners e logs, shell e forward a um clique</sub></td>
  </tr>
  <tr>
    <td><img alt="Stream de logs de um pod com falha, mostrando uma linha fatal de connection refused destacada em vermelho" src="../assets/screenshots/gui-logs.webp"><br><sub><b>Logs</b> — streams de vários pods, instância anterior, filtros, exportação e resumos com IA</sub></td>
    <td><img alt="Editor de YAML com reconhecimento de schema para um Deployment, com validação, diff, dry run e apply" src="../assets/screenshots/gui-yaml.webp"><br><sub><b>YAML</b> — edição com reconhecimento de schema, dry run no servidor, diff e server-side apply</sub></td>
  </tr>
  <tr>
    <td><img alt="Grafo de topologia de serviços mostrando um deployment com falha e o banco de dados externo que ele chama" src="../assets/screenshots/gui-topology.webp"><br><sub><b>Topologia</b> — quem chama quem, e onde a falha começa</sub></td>
    <td><img alt="Releases do Helm com um diff renderizado entre duas revisões" src="../assets/screenshots/gui-helm.webp"><br><sub><b>Helm</b> — instale, atualize, faça rollback e desinstale com um diff renderizado</sub></td>
  </tr>
</table>

O que dá para fazer nele — o [guia do usuário](../USAGE.md) detalha cada item:

- **Workspace multi-cluster** — descubra contextos do kubeconfig, adicione ou cole
  mais arquivos, dê a cada contexto um nome, um logo e uma cor, e troque de
  cluster pela barra de clusters. Contextos com o mesmo nome em arquivos
  diferentes continuam distintos.
- **Recursos do Kubernetes ao vivo** — workloads, rede, armazenamento, RBAC,
  admission, autoscaling e custom resources, com watches ao vivo, busca, seletor
  de colunas, escopo por namespace e ações em massa.
- **Detalhes de recursos e YAML** — manifests, eventos, relacionamentos e
  métricas; YAML com reconhecimento de schema, validação, diffs de dry run e
  server-side apply.
- **Logs** — streams de pods ou workloads com logs da instância anterior,
  timestamps, controles de tail e de janela de tempo (since), cores por origem,
  filtros por container e exportação.
- **Terminais e shells** — exec em pods, um terminal local com escopo de
  contexto, containers efêmeros de debug para pods distroless e shells
  privilegiados em nodes.
- **Port forwarding** — crie, inspecione, copie e encerre forwards em todos os
  clusters abertos.
- **Helm** — inspecione releases e instale, atualize, faça rollback ou desinstale
  com um editor de values e preview do diff renderizado.
- **Ações operacionais** — escale, reinicie rollouts, faça evict ou delete de
  pods, suspenda ou dispare CronJobs, faça cordon e drain de nodes, tudo
  protegido por confirmação.
- **Toolbox** — instale e gerencie `kubectl`, `krew`, `helm` e plugins do krew, e
  diagnostique os requisitos de exec-auth de um contexto.
- **Métricas** — CPU e memória de nodes e pods quando o `metrics-server` estiver
  disponível.
- **Assistente de IA** — pergunte sobre o cluster que está na sua frente, resuma
  um stream de logs ou peça a explicação de um diff do Helm.
- **Paleta de comandos** — navegação pensada para o teclado (Cmd/Ctrl-K) entre
  views, contextos e recursos.
- **Modo web** — rode o mesmo app como servidor web multiusuário no Docker, com
  login via OIDC e isolamento por usuário. Veja [docs/WEB.md](../WEB.md).

## Interface de terminal (`srectl`)

O `srectl` é o srelens no seu terminal: um único binário, navegação no estilo do
k9s e o mesmo núcleo em Rust do aplicativo desktop. Ele se sente em casa via SSH,
em um jump host ou em qualquer lugar onde não há janela.

<p align="center">
  <img alt="srectl mostrando uma tabela de Pods ao vivo com as colunas namespace, readiness, status, restarts, IP e node" src="../assets/screenshots/tui-pods.webp" width="100%">
</p>

<table>
  <tr>
    <td width="50%"><img alt="Árvore de relacionamentos do Deployment ao ReplicaSet e ao Pod, com seu container, ConfigMap e Service" src="../assets/screenshots/tui-tree.webp"><br><sub><b>Árvore de relacionamentos</b> — owners, filhos, configuração e services em uma só view</sub></td>
    <td width="50%"><img alt="Stream de logs ao vivo de um pod com números de linha e controles de follow, timestamp, quebra de linha e salvar" src="../assets/screenshots/tui-logs.webp"><br><sub><b>Logs</b> — follow, busca, timestamps, instância anterior e opção de salvar</sub></td>
  </tr>
  <tr>
    <td><img alt="Visão geral de uma release do Helm com abas para diff de values, revisões, manifest e notes" src="../assets/screenshots/tui-helm.webp"><br><sub><b>Helm</b> — revisões, diff de values, manifest e rollback</sub></td>
    <td><img alt="Inspetor de GPU mostrando a alocação de GPU e VRAM por node e os pods que requisitam GPUs" src="../assets/screenshots/tui-gpu.webp"><br><sub><b>Inspetor de GPU</b> — alocação de GPU e VRAM por node e por pod</sub></td>
  </tr>
  <tr>
    <td><img alt="Aplicações do Argo CD com status de sync e health em clusters hub e spoke" src="../assets/screenshots/tui-argo.webp"><br><sub><b>Argo CD</b> — sync, health e drift em clusters hub e spoke</sub></td>
    <td><img alt="Assistente de IA respondendo quais nodes têm uma GPU T4 depois de chamar a ferramenta listNodes" src="../assets/screenshots/tui-assistant.webp"><br><sub><b>Assistente de IA</b> — consulta o cluster por meio de ferramentas, com playbooks de SRE</sub></td>
  </tr>
</table>

- **Navegação no estilo k9s** — `:` abre o prompt de comandos (`:pod`, `:deploy`,
  `:svc`, `:no`, `:ns`, `:helm`, `:pf`, `:ctx` …), `/` filtra, `j`/`k` movem a
  seleção, `Enter` entra no item e `Esc` volta.
- **Aja sobre o que você vê** — logs (`l`), shell (`s`), describe (`d`), YAML
  (`y`), editar (`e`), port forward (`f`), delete (`Ctrl-d`) e uma paleta de
  ações rápidas (`x`) para restart, scale e mais.
- **Investigue** — árvore de relacionamentos (`t`), hotspots de CPU e memória com
  `:top`, topologia com `:topo`, uma visão geral do cluster e SSH em nodes para
  recuperação.
- **Além do básico** — views de Argo CD (`:argo`), GPU e VRAM (`:gpuinfo`) e
  peering BGP (`:bgp`).
- **Assistente de IA** — `:ai` com Anthropic (Claude), OpenAI, Google Gemini,
  qualquer endpoint compatível com OpenAI, como o Ollama, ou Cursor Agent.
- **Utilitários headless** — `srectl info` lista os contextos do seu kubeconfig,
  `srectl toolbox` verifica se há `kubectl`, `helm` e `krew`, e `srectl mcp` roda
  o servidor MCP via stdio.
- **Autoatualização assinada** — `srectl update` só instala uma release quando os
  checksums dela estão assinados por uma chave de release do srelens embutida no
  binário.

```bash
srectl                      # inicia no contexto atual
srectl -n payments          # inicia na view de Pods, com escopo em um namespace
srectl --context prod -A    # escolhe um contexto, todos os namespaces
srectl pods/my-pod          # abre direto em um recurso
srectl --help               # todas as flags
```

## Instalação

Todas as releases estão no [GitHub Releases](https://github.com/srelens/srelens/releases/latest).
O [guia de instalação](../INSTALL.md) cobre a primeira execução, a atualização,
a verificação dos downloads e a desinstalação em cada plataforma.

### Aplicativo desktop

| Plataforma | Pacotes | Observações |
| --- | --- | --- |
| macOS | `.dmg` para Apple Silicon e Intel | Assinado com Developer ID e notarizado |
| Linux | `.AppImage`, `.deb`, `.rpm` e [`srelens-bin`](https://aur.archlinux.org/packages/srelens-bin) no AUR | O AppImage suporta o atualizador integrado ao app |
| Windows | `.exe`, `.msi` | O Windows pode exibir um aviso do SmartScreen enquanto a assinatura de código continua no roadmap |

### Interface de terminal

macOS (Homebrew):

```bash
brew install srelens/tap/srectl
```

Linux, em uma linha:

```bash
( f="$(mktemp)" && trap 'rm -f "$f"' EXIT &&
  curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh -o "$f" &&
  sh "$f" )
```

Ou baixe `srectl-<version>-<target>.tar.gz` (ou `.zip` no Windows) da release,
para Linux (glibc e musl estático) e macOS em x86-64 e arm64, e para Windows em
x86-64. Os builds para macOS de uma release estável são assinados e notarizados.

### Aplicativo web (Docker)

O srelens também roda como servidor web multiusuário em um container. Os usuários
fazem login com OIDC (ou com um login local de desenvolvimento, para testes) e
cada um recebe um ambiente isolado, montado apenas a partir dos kubeconfigs que
ele mesmo enviou. Clusters protegidos por OIDC fazem login pelo navegador, sem
precisar de `kubelogin` nem de exec plugin, e kubeconfigs e tokens ficam
criptografados em repouso com uma `SRELENS_MASTER_KEY` obrigatória. Veja
[docs/WEB.md](../WEB.md) para o deploy e o modelo de segurança completo.

## Servidor MCP

O srelens inclui um servidor MCP gerado a partir do mesmo registro de capacidades
(capability registry) do backend, então clientes compatíveis com MCP têm acesso
às mesmas operações de cluster sem uma camada de integração à parte.

No aplicativo desktop, abra **Settings → MCP** (Configurações → MCP) para:

- rodar o servidor via HTTP em loopback, protegido por um bearer token que você
  pode revelar, rotacionar ou revogar — rotacionar reinicia o servidor em
  execução para que o novo token passe a valer na hora, e revogar o encerra;
- instalar a CLI `srelens` para conexões stdio, que não precisam de token, porque
  o cliente, ao iniciar o processo, já tem os seus privilégios;
- copiar a configuração de cliente para os clientes MCP suportados.

Ou inicie-o diretamente — pelo binário desktop ou pela interface de terminal:

```sh
srelens --mcp-stdio
srelens --mcp-http 127.0.0.1:8765
srectl mcp
```

Exemplo de configuração stdio:

```json
{
  "mcpServers": {
    "srelens": {
      "command": "srelens",
      "args": ["--mcp-stdio"]
    }
  }
}
```

Ferramentas destrutivas pedem confirmação no app. Execuções headless não têm
diálogo para mostrar, então exigem `"_confirm": true` na chamada *e* um opt-in no
nível do processo: `--mcp-allow-destructive` para alterar qualquer coisa, ou
`--mcp-allow-sensitive-reads` para ler Secrets (`--allow-destructive` e
`--allow-sensitive-reads` no `srectl mcp`). Os dois são independentes, então ler
um Secret nunca implica permissão para fazer drain de um node. Veja
[docs/MCP.md](../MCP.md) para o modelo de segurança completo, incluindo a
verificação do header Host, o log de auditoria e o armazenamento de tokens.

> O acesso via MCP usa os seus contextos de cluster autenticados localmente.
> Revise as chamadas de ferramentas e use permissões de RBAC do Kubernetes
> adequadas, principalmente em clusters críticos.

## Compilar a partir do código-fonte

### Pré-requisitos

- [Rust](https://rustup.rs) stable
- [Node.js](https://nodejs.org) 26 (padrão de desenvolvimento; o Node 24 LTS também é suportado). Rode `nvm install && nvm use` para usar a versão definida no `.nvmrc`.
- [pnpm](https://pnpm.io) 9+
- [Dependências de sistema do Tauri v2](https://v2.tauri.app/start/prerequisites/)
- Um cluster Kubernetes acessível para os fluxos que dependem de cluster

### Rodar localmente

```sh
git clone https://github.com/srelens/srelens
cd srelens
pnpm install
pnpm dev                  # the desktop app
cargo run -p srectl       # the terminal UI
```

### Comandos úteis

| Comando | Finalidade |
| --- | --- |
| `pnpm dev` | Inicia o aplicativo desktop em modo de desenvolvimento |
| `cargo run -p srectl` | Roda a interface de terminal |
| `pnpm test` | Roda os testes de JavaScript e TypeScript |
| `cargo test` | Roda os testes do workspace Rust |
| `pnpm build` | Gera o build de produção do frontend |
| `pnpm tauri build` | Gera os binários desktop empacotados |

Veja o [guia do desenvolvedor](../DEVELOPMENT.md) para arquitetura, padrões de
teste e como adicionar uma capacidade (capability).

## Arquitetura

```text
 Desktop app (React 19 + Tauri v2)   Terminal UI (srectl)   MCP clients   Web app
                 │                           │                  │            │
                 └───────────────┬───────────┴──────────────────┴────────────┘
                                 ▼
                         Pure-Rust core
                         ├── capability registry
                         ├── Kubernetes integration with kube-rs
                         ├── live watches, logs, exec and port forwarding
                         ├── Helm and metrics
                         └── MCP server over stdio and loopback HTTP
```

Estrutura do repositório:

```text
apps/
  desktop/               Tauri desktop app: React UI (src/) and Rust bridge (src-tauri/)
  tui/                   srectl, the terminal UI
packages/
  core/                  Shared frontend service layer
  ui-kit/                Design-system components
  ui-next/               The new desktop design
crates/
  capability/            Backend capability registry
  kube/                  Kubernetes clients, watches, actions, Helm and metrics
  mcp/                   MCP server
  registry/              Where every backend capability is registered
  server/                Web mode server
  streams/               Streaming cores shared by desktop and web
  agent/, llm/           AI assistant and agent-CLI bridge
  plugin-host/           Extension host
docs/                    Installation, usage, development and project documentation
```

## Status do projeto

O srelens é **estável e está em desenvolvimento ativo**, com releases regulares
para macOS, Linux e Windows. Os builds para macOS são assinados com Developer ID
e notarizados, e os pacotes suportados são atualizados in-place. Novas
capacidades chegam o tempo todo, então vale dar uma olhada nas release notes
quando você atualizar.

- [Última release](https://github.com/srelens/srelens/releases/latest)
- [Todas as releases](https://github.com/srelens/srelens/releases)
- [Issues e roadmap](https://github.com/srelens/srelens/issues)

## Comunidade

- [r/srelens no Reddit](https://www.reddit.com/r/srelens/) — anúncios, perguntas
  e feedback
- [Issues](https://github.com/srelens/srelens/issues) — bugs e pedidos de
  funcionalidades
- [Site](https://srelens.com)

## Contribuindo

Contribuições são bem-vindas. Comece pelo [guia de contribuição](../../CONTRIBUTING.md),
pelo [guia do desenvolvedor](../DEVELOPMENT.md), pelo
[guia de integração de agentes via MCP](../MCP.md) e pelas
[issues abertas](https://github.com/srelens/srelens/issues). Leia o
[Código de Conduta](../../.github/CODE_OF_CONDUCT.md) antes de participar.

As traduções ficam em [`docs/i18n`](.). O README em inglês é a fonte da verdade;
correções em uma tradução, ou um novo idioma, são bem-vindas como pull request.

## Licença

O srelens é open source sob a [Licença MIT](../../LICENSE).

---

<p align="center">
  <sub>Não é afiliado ao Mirantis Lens nem ao projeto Freelens.</sub>
</p>
