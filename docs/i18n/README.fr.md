<p align="center">
  <img src="../assets/logo-full.svg" alt="srelens" width="380" />
</p>

<h3 align="center">La salle de contrôle Kubernetes, écrite en Rust, prête pour les ingénieurs comme pour les agents IA.</h3>

<p align="center">
  <a href="../../README.md">English</a> ·
  <a href="README.zh-CN.md">简体中文</a> ·
  <a href="README.ja.md">日本語</a> ·
  <a href="README.ko.md">한국어</a> ·
  <a href="README.es.md">Español</a> ·
  <a href="README.pt-BR.md">Português</a> ·
  <a href="README.de.md">Deutsch</a> ·
  <b>Français</b>
</p>

<p align="center">
  <i>Ceci est une traduction. Le <a href="../../README.md">README anglais</a> fait foi : en cas de divergence, fiez-vous à la version anglaise.</i>
</p>

<p align="center">
  srelens est un espace de travail Kubernetes open source et local-first pour les SRE,
  les ingénieurs plateforme et les ingénieurs DevOps. Investiguez, analysez et agissez
  en toute sécurité sur vos clusters — depuis une <b>application de bureau</b>, depuis une
  <b>interface terminal</b> ou via un <b>serveur MCP</b> que votre agent IA peut appeler.
  Un seul cœur en Rust pur fait tourner les trois.
</p>

<p align="center">
  <a href="https://srelens.com">Site web</a> ·
  <a href="#installation">Installation</a> ·
  <a href="#application-de-bureau">Application de bureau</a> ·
  <a href="#interface-terminal-srectl">Interface terminal</a> ·
  <a href="#serveur-mcp">Serveur MCP</a> ·
  <a href="../USAGE.md">Guide d'utilisation</a> ·
  <a href="../DEVELOPMENT.md">Guide du développeur</a> ·
  <a href="../../CONTRIBUTING.md">Contribuer</a>
</p>

<p align="center">
  <a href="https://github.com/srelens/srelens/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/srelens/srelens?logo=github&color=eab308"></a>
  <a href="https://www.reddit.com/r/srelens/"><img alt="Reddit: r/srelens" src="https://img.shields.io/reddit/subreddit-subscribers/srelens?label=r%2Fsrelens&logo=reddit&logoColor=white&color=FF4500"></a>
  <a href="https://deepwiki.com/srelens/srelens"><img alt="Ask DeepWiki" src="https://deepwiki.com/badge.svg"></a>
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
    <img alt="L'application de bureau srelens dans son nouveau design : une vue d'ensemble du cluster avec la capacité des nœuds, les workloads qui ne sont pas prêts et les objets par kind" src="../assets/screenshots/gui-overview-dark.webp" width="100%">
  </picture>
</p>

---

## Choisissez votre interface

|  | Application de bureau (GUI) | Interface terminal (`srectl`) |
| --- | --- | --- |
| **Ce que c'est** | Une fenêtre native construite avec Tauri v2 et React 19 | Un binaire unique et autonome construit avec ratatui |
| **Idéal pour** | Le travail quotidien sur les clusters, l'édition YAML, la topologie, plusieurs clusters côte à côte | Les sessions SSH, les bastions, un usage 100 % clavier, vos réflexes k9s |
| **Fonctionne sur** | macOS, Linux, Windows — ou dans un navigateur grâce au [mode web](../WEB.md) | Linux (glibc et musl statique) et macOS en x86-64 et arm64 ; Windows en x86-64 |
| **Installation** | [Télécharger une version](#installation) | `brew install srelens/tap/srectl` ou [en une ligne](#installation) |
| **IA** | Assistant intégré, plus le serveur MCP dans **Settings → MCP** (Paramètres → MCP) | Assistant `:ai`, plus `srectl mcp` via stdio |

Les deux parlent directement à vos clusters, avec les identifiants de votre
kubeconfig local. Rien ne transite par un service cloud srelens.

## Pourquoi srelens ?

Dépanner Kubernetes, c'est généralement jongler entre terminaux, tableaux de bord,
éditeurs YAML, logs et contextes de cluster. srelens réunit toute cette boucle
d'investigation dans un seul espace de travail local-first.

- **Un seul espace de travail, de l'investigation à l'action** — parcourez les
  ressources, lisez les events et le YAML, suivez les logs, ouvrez des shells,
  faites du port forwarding et agissez sur le cluster sans changer d'outil.
- **Conçu pour les ingénieurs et les agents IA** — les capacités backend que vous
  utilisez dans l'application sont exactement celles qu'expose le serveur MCP intégré.
- **Accès local-first aux clusters** — srelens se connecte directement à vos
  serveurs d'API avec les identifiants de votre propre kubeconfig.
- **Des opérations sûres** — les actions destructrices sont identifiées et soumises
  à confirmation, dans l'application, dans le terminal et via MCP.
- **Léger et rapide** — un cœur Rust sur la WebView du système d'exploitation.
  srelens `v0.5.0` était livré avec un installeur macOS de **17,1 Mio** et un `.deb`
  Linux de **19,8 Mio**, contre 190,2 Mio et 146,8 Mio pour Freelens `v1.10.3` —
  soit des paquets environ **11× et 7× plus petits**. Les
  [mesures de performance de référence](../PERFORMANCE.md) expliquent comment
  reproduire ces chiffres, y compris là où l'écart est plus faible.
- **Open source** — sous licence MIT, avec un code, des versions, des issues et une
  feuille de route publics.

## Application de bureau

L'application de bureau est un espace de travail Kubernetes complet dans une
fenêtre native. Les captures d'écran ci-dessous montrent le **nouveau design**,
déployé écran par écran : activez-le dans **Settings → Appearance → Design → New
design** (Paramètres → Apparence → Design → Nouveau design), et revenez en arrière
de la même façon quand vous le souhaitez. Il propose six thèmes : Light, Paper,
Dark, Midnight, Glass et High contrast.

<table>
  <tr>
    <td width="50%"><img alt="Liste des pods filtrée sur un namespace, avec un pod en crash loop signalé en rouge" src="../assets/screenshots/gui-pods.webp"><br><sub><b>Ressources en direct</b> — tableaux alimentés par des watches, avec filtres, colonnes et actions groupées</sub></td>
    <td width="50%"><img alt="Vue détaillée d'un pod avec readiness, redémarrages, CPU, mémoire et actions rapides pour les logs, le shell et le port forward" src="../assets/screenshots/gui-pod-detail.webp"><br><sub><b>Détails des ressources</b> — statut, events, propriétaires, et logs, shell et forward en un clic</sub></td>
  </tr>
  <tr>
    <td><img alt="Flux de logs d'un pod en échec, avec une ligne fatale « connection refused » signalée en rouge" src="../assets/screenshots/gui-logs.webp"><br><sub><b>Logs</b> — flux multi-pods, instance précédente, filtres, export et résumés par IA</sub></td>
    <td><img alt="Éditeur YAML qui connaît le schéma d'un Deployment, avec validation, diff, dry run et apply" src="../assets/screenshots/gui-yaml.webp"><br><sub><b>YAML</b> — édition guidée par le schéma, dry run côté serveur, diff et server-side apply</sub></td>
  </tr>
  <tr>
    <td><img alt="Graphe de topologie des services montrant un déploiement en échec et la base de données externe qu'il appelle" src="../assets/screenshots/gui-topology.webp"><br><sub><b>Topologie</b> — qui appelle qui, et où la panne commence</sub></td>
    <td><img alt="Releases Helm avec un diff rendu entre deux révisions" src="../assets/screenshots/gui-helm.webp"><br><sub><b>Helm</b> — installation, mise à jour, rollback et désinstallation avec un diff rendu</sub></td>
  </tr>
</table>

Ce que vous pouvez y faire — le [guide d'utilisation](../USAGE.md) détaille chaque point :

- **Espace de travail multi-cluster** — détectez les contextes kubeconfig, ajoutez ou
  collez d'autres fichiers, donnez à chaque contexte un nom, un logo et une couleur,
  et passez d'un cluster à l'autre depuis le rail des clusters. Des contextes qui
  portent le même nom dans des fichiers différents restent distincts.
- **Ressources Kubernetes en direct** — workloads, réseau, stockage, RBAC, admission,
  autoscaling et custom resources, avec watches en direct, recherche, choix des
  colonnes, filtrage par namespace et actions groupées.
- **Détails des ressources et YAML** — manifestes, events, relations et métriques ;
  YAML guidé par le schéma avec validation, diffs en dry run et server-side apply.
- **Logs** — flux par pod ou par workload avec les logs de l'instance précédente,
  horodatages, réglages de tail et de fenêtre temporelle (since), couleurs par
  source, filtres par conteneur et export.
- **Terminaux et shells** — exec dans les pods, un terminal local lié au contexte,
  des conteneurs de debug éphémères pour les pods distroless, et des shells
  privilégiés sur les nœuds.
- **Port forwarding** — créez, inspectez, copiez et arrêtez des redirections de port
  sur tous les clusters ouverts.
- **Helm** — inspectez les releases, puis installez-les, mettez-les à jour, faites un
  rollback ou désinstallez-les, avec un éditeur de values et un aperçu du diff rendu.
- **Actions opérationnelles** — scale, redémarrage de rollouts, éviction ou
  suppression de pods, suspension ou déclenchement de CronJobs, cordon et drain des
  nœuds, le tout derrière des confirmations.
- **Boîte à outils** — installez et gérez `kubectl`, `krew`, `helm` et les plugins
  krew, et diagnostiquez ce qu'exige l'exec-auth d'un contexte.
- **Métriques** — CPU et mémoire des nœuds et des pods quand `metrics-server` est
  disponible.
- **Assistant IA** — interrogez-le sur le cluster que vous avez sous les yeux,
  faites-lui résumer un flux de logs ou expliquer un diff Helm.
- **Palette de commandes** — navigation au clavier (Cmd/Ctrl-K) entre les vues, les
  contextes et les ressources.
- **Mode web** — exécutez la même application comme serveur web multi-utilisateur
  dans Docker, avec connexion OIDC et isolation par utilisateur. Voir
  [docs/WEB.md](../WEB.md).

## Interface terminal (`srectl`)

`srectl`, c'est srelens dans votre terminal : un seul binaire, une navigation à la
k9s, et le même cœur Rust que l'application de bureau. Il est dans son élément en
SSH, sur un bastion, ou partout où vous n'avez pas de fenêtre graphique.

<p align="center">
  <img alt="srectl affichant un tableau de Pods en direct avec les colonnes namespace, readiness, statut, redémarrages, IP et nœud" src="../assets/screenshots/tui-pods.webp" width="100%">
</p>

<table>
  <tr>
    <td width="50%"><img alt="Arbre de relations d'une ressource, du Deployment au ReplicaSet puis au Pod, avec son conteneur, sa ConfigMap et son Service" src="../assets/screenshots/tui-tree.webp"><br><sub><b>Arbre de relations</b> — propriétaires, enfants, configuration et services dans une seule vue</sub></td>
    <td width="50%"><img alt="Flux de logs en direct d'un pod avec numéros de ligne et commandes de suivi, d'horodatage, de retour à la ligne et d'enregistrement" src="../assets/screenshots/tui-logs.webp"><br><sub><b>Logs</b> — suivi, recherche, horodatages, instance précédente et enregistrement</sub></td>
  </tr>
  <tr>
    <td><img alt="Vue d'ensemble d'une release Helm avec des onglets pour le diff des values, les révisions, le manifeste et les notes" src="../assets/screenshots/tui-helm.webp"><br><sub><b>Helm</b> — révisions, diff des values, manifeste et rollback</sub></td>
    <td><img alt="Inspecteur GPU montrant l'allocation de GPU et de VRAM par nœud et les pods qui demandent des GPU" src="../assets/screenshots/tui-gpu.webp"><br><sub><b>Inspecteur GPU</b> — allocation de GPU et de VRAM par nœud et par pod</sub></td>
  </tr>
  <tr>
    <td><img alt="Applications Argo CD avec leur état de synchronisation et de santé sur des clusters hub et spoke" src="../assets/screenshots/tui-argo.webp"><br><sub><b>Argo CD</b> — synchronisation, santé et dérive sur des clusters hub et spoke</sub></td>
    <td><img alt="Assistant IA indiquant quels nœuds ont un GPU T4 après avoir appelé l'outil listNodes" src="../assets/screenshots/tui-assistant.webp"><br><sub><b>Assistant IA</b> — interroge le cluster via des outils, avec des playbooks SRE</sub></td>
  </tr>
</table>

- **Navigation à la k9s** — `:` ouvre l'invite de commande (`:pod`, `:deploy`,
  `:svc`, `:no`, `:ns`, `:helm`, `:pf`, `:ctx` …), `/` filtre, `j`/`k` déplacent la
  sélection, `Enter` descend d'un niveau et `Esc` revient en arrière.
- **Agissez sur ce que vous voyez** — logs (`l`), shell (`s`), describe (`d`),
  YAML (`y`), édition (`e`), port forward (`f`), suppression (`Ctrl-d`) et une
  palette d'actions rapides (`x`) pour redémarrer, scaler et plus encore.
- **Investiguez** — arbre de relations (`t`), `:top` pour les points chauds en CPU et
  en mémoire, `:topo` pour la topologie, une vue d'ensemble du cluster, et du SSH
  sur les nœuds pour la remise en état.
- **Au-delà des bases** — vues Argo CD (`:argo`), GPU et VRAM (`:gpuinfo`), et
  peering BGP (`:bgp`).
- **Assistant IA** — `:ai` avec Anthropic (Claude), OpenAI, Google Gemini, tout
  endpoint compatible OpenAI comme Ollama, ou Cursor Agent.
- **Utilitaires headless** — `srectl info` liste les contextes de votre kubeconfig,
  `srectl toolbox` vérifie la présence de `kubectl`, `helm` et `krew`, et
  `srectl mcp` lance le serveur MCP via stdio.
- **Auto-mise à jour signée** — `srectl update` n'installe une version que si ses
  sommes de contrôle sont signées par une clé de release srelens intégrée au binaire.

```bash
srectl                      # démarrer sur le contexte courant
srectl -n payments          # démarrer sur la vue Pods, limitée à un namespace
srectl --context prod -A    # choisir un contexte, tous les namespaces
srectl pods/my-pod          # ouvrir directement une ressource
srectl --help               # toutes les options
```

## Installation

Toutes les versions sont disponibles sur [GitHub Releases](https://github.com/srelens/srelens/releases/latest).
Le [guide d'installation](../INSTALL.md) couvre le premier lancement, la mise à
jour, la vérification des téléchargements et la désinstallation sur chaque plateforme.

### Application de bureau

| Plateforme | Paquets | Remarques |
| --- | --- | --- |
| macOS | `.dmg` pour Apple Silicon et Intel | Signé avec un Developer ID et notarisé |
| Linux | `.AppImage`, `.deb`, `.rpm`, et [`srelens-bin`](https://aur.archlinux.org/packages/srelens-bin) sur l'AUR | L'AppImage prend en charge la mise à jour intégrée à l'application |
| Windows | `.exe`, `.msi` | Windows peut afficher un avertissement SmartScreen tant que la signature du code reste sur la feuille de route |

### Interface terminal

macOS (Homebrew) :

```bash
brew install srelens/tap/srectl
```

Linux, en une ligne :

```bash
( f="$(mktemp)" && trap 'rm -f "$f"' EXIT &&
  curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh -o "$f" &&
  sh "$f" )
```

Ou téléchargez `srectl-<version>-<target>.tar.gz` (ou `.zip` sous Windows) depuis
la page de la version, pour Linux (glibc et musl statique) et macOS en x86-64 et
arm64, et pour Windows en x86-64. Les builds macOS d'une version stable sont signés
et notarisés.

### Application web (Docker)

srelens fonctionne aussi comme serveur web multi-utilisateur dans un conteneur. Les
utilisateurs se connectent via OIDC (ou via un login de développement local pour
les essais) et chacun obtient un environnement isolé, construit uniquement à partir
des kubeconfigs qu'il a lui-même importés. La connexion aux clusters protégés par
OIDC se fait depuis le navigateur, sans `kubelogin` ni plugin exec, et les
kubeconfigs et les tokens sont chiffrés au repos sous une `SRELENS_MASTER_KEY`
obligatoire. Voir [docs/WEB.md](../WEB.md) pour le déploiement et le modèle de
sécurité complet.

## Serveur MCP

srelens inclut un serveur MCP généré à partir du même registre de capacités que le
backend : les clients compatibles MCP disposent ainsi des mêmes opérations sur les
clusters, sans couche d'intégration séparée.

Dans l'application de bureau, ouvrez **Settings → MCP** (Paramètres → MCP) pour :

- exécuter le serveur en HTTP sur loopback, protégé par un bearer token que vous
  pouvez afficher, renouveler ou révoquer — le renouvellement redémarre le serveur
  en cours d'exécution pour que le nouveau token prenne effet immédiatement, et la
  révocation l'arrête ;
- installer la CLI `srelens` pour les connexions stdio, qui ne nécessitent aucun
  token, puisque le client détient déjà vos privilèges en lançant lui-même le
  processus ;
- copier la configuration client pour les clients MCP pris en charge.

Ou lancez-le directement — depuis le binaire de bureau, ou depuis l'interface
terminal :

```sh
srelens --mcp-stdio
srelens --mcp-http 127.0.0.1:8765
srectl mcp
```

Exemple de configuration stdio :

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

Les outils destructeurs demandent une confirmation dans l'application. Les
exécutions headless n'ont aucune boîte de dialogue à afficher : elles exigent donc
`"_confirm": true` dans l'appel *et* une autorisation explicite au niveau du
processus — `--mcp-allow-destructive` pour modifier quoi que ce soit, ou
`--mcp-allow-sensitive-reads` pour lire des Secrets (`--allow-destructive` et
`--allow-sensitive-reads` sur `srectl mcp`). Les deux sont indépendantes : lire un
Secret n'implique jamais le droit de drainer un nœud. Voir [docs/MCP.md](../MCP.md)
pour le modèle de sécurité complet, y compris la vérification de l'en-tête Host, le
journal d'audit et le stockage des tokens.

> L'accès MCP utilise vos contextes de cluster authentifiés localement. Vérifiez les
> appels d'outils et appliquez des permissions RBAC Kubernetes adaptées, en
> particulier sur les clusters critiques.

## Compiler depuis les sources

### Prérequis

- [Rust](https://rustup.rs) stable
- [Node.js](https://nodejs.org) 26 (version par défaut pour le développement ; Node 24 LTS est également pris en charge). Lancez `nvm install && nvm use` pour utiliser la version indiquée dans `.nvmrc`.
- [pnpm](https://pnpm.io) 9+
- [Dépendances système de Tauri v2](https://v2.tauri.app/start/prerequisites/)
- Un cluster Kubernetes joignable pour les workflows qui dépendent d'un cluster

### Lancer en local

```sh
git clone https://github.com/srelens/srelens
cd srelens
pnpm install
pnpm dev                  # the desktop app
cargo run -p srectl       # the terminal UI
```

### Commandes utiles

| Commande | Rôle |
| --- | --- |
| `pnpm dev` | Lancer l'application de bureau en mode développement |
| `cargo run -p srectl` | Lancer l'interface terminal |
| `pnpm test` | Exécuter les tests JavaScript et TypeScript |
| `cargo test` | Exécuter les tests du workspace Rust |
| `pnpm build` | Construire le frontend de production |
| `pnpm tauri build` | Créer les binaires de bureau packagés |

Consultez le [guide du développeur](../DEVELOPMENT.md) pour l'architecture, les
standards de test et la marche à suivre pour ajouter une capacité.

## Architecture

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

Organisation du dépôt :

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

## État du projet

srelens est **stable et en développement actif**, avec des versions régulières pour
macOS, Linux et Windows. Les builds macOS sont signés avec un Developer ID et
notarisés, et les paquets pris en charge se mettent à jour sur place. De nouvelles
capacités arrivent en continu : les notes de version méritent donc un coup d'œil à
chaque mise à jour.

- [Dernière version](https://github.com/srelens/srelens/releases/latest)
- [Toutes les versions](https://github.com/srelens/srelens/releases)
- [Issues et feuille de route](https://github.com/srelens/srelens/issues)

## Communauté

- [r/srelens sur Reddit](https://www.reddit.com/r/srelens/) — annonces, questions
  et retours
- [Issues](https://github.com/srelens/srelens/issues) — bugs et demandes de
  fonctionnalités
- [Site web](https://srelens.com)

## Contribuer

Les contributions sont les bienvenues. Commencez par le
[guide de contribution](../../CONTRIBUTING.md), le
[guide du développeur](../DEVELOPMENT.md), le
[guide d'intégration des agents MCP](../MCP.md) et les
[issues ouvertes](https://github.com/srelens/srelens/issues). Merci de lire le
[Code de conduite](../../.github/CODE_OF_CONDUCT.md) avant de participer.

Les traductions se trouvent dans [`docs/i18n`](.). Le README anglais fait foi ;
une correction de traduction, ou une nouvelle langue, est la bienvenue sous forme
de pull request.

## Licence

srelens est open source, sous [licence MIT](../../LICENSE).

---

<p align="center">
  <sub>Sans lien avec Mirantis Lens ni avec le projet Freelens.</sub>
</p>
