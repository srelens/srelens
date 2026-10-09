<p align="center">
  <img src="../assets/logo-full.svg" alt="srelens" width="380" />
</p>

<h3 align="center">Kubernetes 控制中心——基于 Rust 构建，工程师与 AI 智能体开箱即用。</h3>

<p align="center">
  <a href="../../README.md">English</a> ·
  <b>简体中文</b> ·
  <a href="README.ja.md">日本語</a> ·
  <a href="README.ko.md">한국어</a> ·
  <a href="README.es.md">Español</a> ·
  <a href="README.pt-BR.md">Português</a> ·
  <a href="README.de.md">Deutsch</a> ·
  <a href="README.fr.md">Français</a>
</p>

<p align="center">
  本文为译文，<a href="../../README.md">英文 README</a> 是唯一权威版本；两者如有出入，请以英文版为准。
</p>

<p align="center">
  srelens 是一款开源、本地优先的 Kubernetes 工作台，面向 SRE、平台工程师和
  DevOps 工程师。你可以跨集群排查问题、分析状况并安全地执行操作——无论是通过<b>桌面应用</b>、<b>终端界面</b>，还是通过
  AI 智能体可以调用的 <b>MCP 服务器</b>。三者均由同一个纯 Rust 核心驱动。
</p>

<p align="center">
  <a href="https://srelens.com">官网</a> ·
  <a href="#安装">安装</a> ·
  <a href="#桌面应用">桌面应用</a> ·
  <a href="#终端界面srectl">终端界面</a> ·
  <a href="#mcp-服务器">MCP 服务器</a> ·
  <a href="../USAGE.md">用户指南</a> ·
  <a href="../DEVELOPMENT.md">开发者指南</a> ·
  <a href="../../CONTRIBUTING.md">参与贡献</a>
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
    <img alt="采用新设计的 srelens 桌面应用：集群概览，展示节点容量、未就绪的工作负载，以及按 kind 分类的对象" src="../assets/screenshots/gui-overview-dark.webp" width="100%">
  </picture>
</p>

---

## 选择你的使用界面

|  | 桌面应用（GUI） | 终端界面（`srectl`） |
| --- | --- | --- |
| **它是什么** | 基于 Tauri v2 和 React 19 构建的原生窗口应用 | 基于 ratatui 构建的单个自包含二进制文件 |
| **最适合** | 日常集群工作、YAML 编辑、拓扑查看、多集群并排操作 | SSH 会话、跳板机、纯键盘操作、延续 k9s 的操作习惯 |
| **运行平台** | macOS、Linux、Windows——也可以通过 [Web 模式](../WEB.md)在浏览器中运行 | x86-64 和 arm64 上的 Linux（glibc 与静态 musl）和 macOS；x86-64 上的 Windows |
| **安装** | [下载发行版](#安装) | `brew install srelens/tap/srectl` 或[一行命令安装](#安装) |
| **AI** | 内置助手，以及 **Settings → MCP**（设置 → MCP）中的 MCP 服务器 | `:ai` 助手，以及通过 stdio 运行的 `srectl mcp` |

两者都使用你本地 kubeconfig 中的凭据直接连接集群，不经过任何 srelens 云服务中转。

## 为什么选择 srelens？

排查 Kubernetes 问题时，往往要在终端、仪表盘、YAML 编辑器、日志和集群上下文之间来回切换。srelens
把这整套排查流程收拢到一个本地优先的工作台中。

- **从排查到处置，一个工作台搞定**——浏览资源、查看事件和 YAML、跟踪日志、打开 Shell、转发端口、执行集群操作，全程无需切换工具。
- **为工程师与 AI 智能体而生**——你在应用中使用的后端能力，与内置 MCP 服务器对外暴露的能力完全相同。
- **本地优先的集群访问**——srelens 使用你自己的 kubeconfig 凭据，直接连接你的 API 服务器。
- **安全操作**——无论是在应用中、终端里还是通过 MCP，破坏性操作都会被识别出来，并且必须经过确认。
- **小巧快速**——Rust 核心运行在操作系统自带的 WebView 之上。srelens `v0.5.0`
  发布的 macOS 安装包为 **17.1 MiB**，Linux `.deb` 为 **19.8 MiB**，而 Freelens `v1.10.3`
  分别为 190.2 MiB 和 146.8 MiB——大约**小 11 倍和 7 倍**。[性能基线](../PERFORMANCE.md)说明了如何复现这一结果，也指出了差距较小的情形。
- **开源**——采用 MIT 许可证，代码、发行版、Issue 和路线图全部公开。

## 桌面应用

桌面应用是一个运行在原生窗口中的完整 Kubernetes 工作台。下方截图展示的是**新设计**，它正在逐个界面地推出：在
**Settings → Appearance → Design → New design**（设置 → 外观 → 设计 → 新设计）中即可开启，也随时可以用同样的方式切换回来。它提供六种主题：Light、Paper、Dark、Midnight、Glass
和 High contrast。

<table>
  <tr>
    <td width="50%"><img alt="筛选到单个命名空间的 Pod 列表，其中一个处于崩溃循环的 Pod 以红色标出" src="../assets/screenshots/gui-pods.webp"><br><sub><b>实时资源</b>——由 watch 驱动的表格，支持筛选、列配置和批量操作</sub></td>
    <td width="50%"><img alt="Pod 详情视图，显示就绪状态、重启次数、CPU、内存，并提供日志、Shell 和端口转发的快捷操作" src="../assets/screenshots/gui-pod-detail.webp"><br><sub><b>资源详情</b>——状态、事件、所有者，一键查看日志、打开 Shell 和端口转发</sub></td>
  </tr>
  <tr>
    <td><img alt="一个故障 Pod 的日志流，其中一条 fatal 级别的 connection refused 日志以红色标出" src="../assets/screenshots/gui-logs.webp"><br><sub><b>日志</b>——多 Pod 日志流、上一个实例的日志、筛选、导出和 AI 摘要</sub></td>
    <td><img alt="感知 Schema 的 Deployment YAML 编辑器，支持校验、diff、dry run 和 apply" src="../assets/screenshots/gui-yaml.webp"><br><sub><b>YAML</b>——感知 Schema 的编辑、服务端 dry run、diff 和服务端 apply</sub></td>
  </tr>
  <tr>
    <td><img alt="服务拓扑图，展示一个故障 Deployment 及其调用的外部数据库" src="../assets/screenshots/gui-topology.webp"><br><sub><b>拓扑</b>——谁在调用谁，故障从哪里开始</sub></td>
    <td><img alt="Helm release 列表，以及两个修订版本之间渲染后的 diff" src="../assets/screenshots/gui-helm.webp"><br><sub><b>Helm</b>——安装、升级、回滚和卸载，并提供渲染后的 diff</sub></td>
  </tr>
</table>

它能做些什么——[用户指南](../USAGE.md)对每一项都有详细介绍：

- **多集群工作台**——自动发现 kubeconfig 上下文，可添加或粘贴更多文件，为每个上下文设置名称、Logo
  和颜色，并通过集群侧栏切换集群。不同文件中同名的上下文会各自独立，不会混淆。
- **实时 Kubernetes 资源**——工作负载、网络、存储、RBAC、准入控制、自动扩缩容和自定义资源，支持实时
  watch、搜索、列选择、按命名空间限定范围以及批量操作。
- **资源详情与 YAML**——清单、事件、关联关系和指标；感知 Schema 的 YAML 编辑，支持校验、dry run diff
  和服务端 apply。
- **日志**——Pod 或工作负载日志流，支持查看上一个实例的日志、时间戳、tail 和 since 时间窗口控制、按来源着色、容器筛选和导出。
- **终端与 Shell**——Pod exec、限定在上下文中的本地终端、用于 distroless Pod 的临时调试容器，以及特权节点
  Shell。
- **端口转发**——在所有已打开的集群中创建、查看、复制和停止端口转发。
- **Helm**——查看 release，并借助 values 编辑器和渲染后 diff 的预览来安装、升级、回滚或卸载它们。
- **运维操作**——扩缩容、重启 rollout、驱逐或删除 Pod、暂停或触发 CronJob、cordon 和 drain
  节点，所有操作都需经过确认。
- **工具箱**——安装和管理 `kubectl`、`krew`、`helm` 及 krew 插件，并诊断某个上下文的 exec-auth 需求。
- **指标**——在 `metrics-server` 可用时，显示节点和 Pod 的 CPU 与内存。
- **AI 助手**——询问眼前这个集群的情况、总结日志流，或解释 Helm diff。
- **命令面板**——键盘优先的导航（Cmd/Ctrl-K），可在视图、上下文和资源之间快速跳转。
- **Web 模式**——将同一个应用作为多用户 Web 服务器运行在 Docker 中，支持 OIDC
  登录和按用户隔离。参见 [docs/WEB.md](../WEB.md)。

## 终端界面（`srectl`）

`srectl` 就是终端里的 srelens：单个二进制文件、k9s 风格的导航，并与桌面应用共用同一个 Rust
核心。无论是通过 SSH、在跳板机上，还是在任何没有图形窗口的地方，它都能得心应手。

<p align="center">
  <img alt="srectl 显示实时 Pod 表格，包含命名空间、就绪状态、状态、重启次数、IP 和节点等列" src="../assets/screenshots/tui-pods.webp" width="100%">
</p>

<table>
  <tr>
    <td width="50%"><img alt="从 Deployment 到 ReplicaSet 再到 Pod 的资源关系树，并包含其容器、ConfigMap 和 Service" src="../assets/screenshots/tui-tree.webp"><br><sub><b>关系树</b>——所有者、子资源、配置和服务尽在一个视图中</sub></td>
    <td width="50%"><img alt="某个 Pod 的实时日志流，带行号，以及跟随、时间戳、自动换行和保存等控件" src="../assets/screenshots/tui-logs.webp"><br><sub><b>日志</b>——跟随、搜索、时间戳、上一个实例和保存</sub></td>
  </tr>
  <tr>
    <td><img alt="Helm release 概览，包含 values diff、修订版本、manifest 和 notes 等标签页" src="../assets/screenshots/tui-helm.webp"><br><sub><b>Helm</b>——修订版本、values diff、manifest 和回滚</sub></td>
    <td><img alt="GPU 检查器，显示每个节点的 GPU 和显存（VRAM）分配，以及申请 GPU 的 Pod" src="../assets/screenshots/tui-gpu.webp"><br><sub><b>GPU 检查器</b>——按节点和按 Pod 查看 GPU 与显存（VRAM）分配</sub></td>
  </tr>
  <tr>
    <td><img alt="Argo CD 应用在 hub 与 spoke 集群中的同步和健康状态" src="../assets/screenshots/tui-argo.webp"><br><sub><b>Argo CD</b>——跨 hub 与 spoke 集群的同步、健康和漂移状态</sub></td>
    <td><img alt="AI 助手在调用 listNodes 工具后，回答哪些节点配有 T4 GPU" src="../assets/screenshots/tui-assistant.webp"><br><sub><b>AI 助手</b>——借助工具向集群提问，内置 SRE 处置手册（playbook）</sub></td>
  </tr>
</table>

- **k9s 风格的导航**——`:` 打开命令提示符（`:pod`、`:deploy`、`:svc`、`:no`、`:ns`、`:helm`、`:pf`、`:ctx`
  …），`/` 筛选，`j`/`k` 移动，`Enter` 进入下一级，`Esc` 返回。
- **所见即可操作**——日志（`l`）、Shell（`s`）、describe（`d`）、YAML（`y`）、编辑（`e`）、端口转发（`f`）、删除（`Ctrl-d`），以及用于重启、扩缩容等操作的快捷操作面板（`x`）。
- **排查**——关系树（`t`）、按 CPU 和内存列出热点的 `:top`、`:topo` 拓扑、集群概览，以及用于故障恢复的节点 SSH。
- **进阶功能**——Argo CD（`:argo`）、GPU 与显存（`:gpuinfo`）以及 BGP 对等（`:bgp`）视图。
- **AI 助手**——`:ai` 支持 Anthropic（Claude）、OpenAI、Google Gemini、任何兼容 OpenAI 的端点（例如
  Ollama），以及 Cursor Agent。
- **无界面辅助命令**——`srectl info` 列出 kubeconfig 中的上下文，`srectl toolbox` 检查 `kubectl`、`helm`
  和 `krew`，`srectl mcp` 通过 stdio 运行 MCP 服务器。
- **签名自更新**——只有当发行版的校验和由内置于二进制文件中的 srelens 发布密钥签名时，`srectl update`
  才会安装该版本。

```bash
srectl                      # 从当前上下文启动
srectl -n payments          # 打开 Pod 视图，并限定到某个命名空间
srectl --context prod -A    # 指定上下文，查看所有命名空间
srectl pods/my-pod          # 直接打开某个资源
srectl --help               # 查看全部参数
```

## 安装

所有发行版都发布在 [GitHub Releases](https://github.com/srelens/srelens/releases/latest) 上。[安装指南](../INSTALL.md)介绍了各平台上的首次启动、更新、校验下载文件和卸载。

### 桌面应用

| 平台 | 安装包 | 说明 |
| --- | --- | --- |
| macOS | 适用于 Apple Silicon 和 Intel 的 `.dmg` | 已使用 Developer ID 签名并经过公证 |
| Linux | `.AppImage`、`.deb`、`.rpm`，以及 AUR 上的 [`srelens-bin`](https://aur.archlinux.org/packages/srelens-bin) | AppImage 支持应用内更新 |
| Windows | `.exe`、`.msi` | 代码签名仍在路线图中，在此之前 Windows 可能会弹出 SmartScreen 提示 |

### 终端界面

macOS（Homebrew）：

```bash
brew install srelens/tap/srectl
```

Linux，一行命令：

```bash
( f="$(mktemp)" && trap 'rm -f "$f"' EXIT &&
  curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh -o "$f" &&
  sh "$f" )
```

也可以从发行版页面下载 `srectl-<version>-<target>.tar.gz`（Windows 上为 `.zip`），提供 x86-64 和 arm64
上的 Linux（glibc 与静态 musl）和 macOS 版本，以及 x86-64 上的 Windows 版本。稳定版中的 macOS
构建均已签名并经过公证。

### Web 应用（Docker）

srelens 也可以作为多用户 Web 服务器运行在容器中。用户通过 OIDC 登录（试用时也可使用本地开发登录），每个用户都会获得一个隔离的环境，该环境仅由其本人上传的
kubeconfig 构建。受 OIDC 保护的集群直接在浏览器中完成登录，无需 `kubelogin` 或 exec 插件；kubeconfig
和令牌在存储时使用必填的 `SRELENS_MASTER_KEY` 加密封存。部署方式和完整的安全模型请参见
[docs/WEB.md](../WEB.md)。

## MCP 服务器

srelens 内置一个 MCP 服务器，它与后端由同一个能力注册表生成，因此支持 MCP
的客户端无需额外的集成层，即可获得同样的集群操作。

在桌面应用中打开 **Settings → MCP**（设置 → MCP），你可以：

- 通过回环（loopback）HTTP 运行服务器，并用一个 bearer token 加以保护，你可以查看、轮换或吊销该令牌——轮换会重启正在运行的服务器，使新令牌立即生效；吊销则会停止服务器；
- 安装用于 stdio 连接的 `srelens` CLI，stdio 连接不需要令牌，因为客户端通过启动该进程，就已经拥有了你的权限；
- 为受支持的 MCP 客户端复制客户端配置。

也可以直接启动——通过桌面应用的二进制文件，或通过终端界面：

```sh
srelens --mcp-stdio
srelens --mcp-http 127.0.0.1:8765
srectl mcp
```

stdio 配置示例：

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

在应用中，破坏性工具会弹出确认提示。无界面运行时没有对话框可以显示，因此调用时需要带上
`"_confirm": true`，*并且*需要在进程级别显式启用：要进行任何修改，需使用 `--mcp-allow-destructive`；要读取
Secret，需使用 `--mcp-allow-sensitive-reads`（在 `srectl mcp` 上分别为 `--allow-destructive` 和
`--allow-sensitive-reads`）。两者相互独立，因此能读取 Secret 绝不意味着有权限 drain 节点。完整的安全模型（包括
Host 请求头检查、审计日志和令牌存储）请参见 [docs/MCP.md](../MCP.md)。

> MCP 访问使用的是你在本地已完成认证的集群上下文。请审查工具调用，并配置合适的
> Kubernetes RBAC 权限，尤其是在关键集群上。

## 从源码构建

### 前置条件

- [Rust](https://rustup.rs) 稳定版
- [Node.js](https://nodejs.org) 26（开发默认版本；也支持 Node 24 LTS）。运行 `nvm install && nvm use` 即可使用 `.nvmrc` 中指定的版本。
- [pnpm](https://pnpm.io) 9+
- [Tauri v2 系统依赖](https://v2.tauri.app/start/prerequisites/)
- 一个可访问的 Kubernetes 集群，用于依赖集群的工作流

### 本地运行

```sh
git clone https://github.com/srelens/srelens
cd srelens
pnpm install
pnpm dev                  # the desktop app
cargo run -p srectl       # the terminal UI
```

### 常用命令

| 命令 | 用途 |
| --- | --- |
| `pnpm dev` | 以开发模式启动桌面应用 |
| `cargo run -p srectl` | 运行终端界面 |
| `pnpm test` | 运行 JavaScript 和 TypeScript 测试 |
| `cargo test` | 运行 Rust workspace 测试 |
| `pnpm build` | 构建生产环境前端 |
| `pnpm tauri build` | 打包桌面应用二进制文件 |

架构、测试规范以及如何新增一项能力，请参见[开发者指南](../DEVELOPMENT.md)。

## 架构

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

仓库结构：

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

## 项目状态

srelens **已经稳定，并仍在积极开发中**，会定期发布 macOS、Linux 和 Windows 版本。macOS 构建均已使用
Developer ID 签名并经过公证，受支持的安装包可以原地更新。新功能持续合入，因此升级时值得看一眼发行说明。

- [最新发行版](https://github.com/srelens/srelens/releases/latest)
- [全部发行版](https://github.com/srelens/srelens/releases)
- [Issue 与路线图](https://github.com/srelens/srelens/issues)

## 社区

- [Reddit 上的 r/srelens](https://www.reddit.com/r/srelens/)——公告、提问和反馈
- [Issue 列表](https://github.com/srelens/srelens/issues)——Bug 报告和功能请求
- [官网](https://srelens.com)

## 参与贡献

欢迎参与贡献。可以从[贡献指南](../../CONTRIBUTING.md)、[开发者指南](../DEVELOPMENT.md)、[MCP
智能体集成指南](../MCP.md)和[未关闭的 Issue](https://github.com/srelens/srelens/issues) 入手。参与之前，请先阅读[行为准则](../../.github/CODE_OF_CONDUCT.md)。

各语言译文位于 [`docs/i18n`](.)。英文 README 是唯一权威版本；欢迎通过 Pull Request 修正译文或新增语言。

## 许可证

srelens 基于 [MIT 许可证](../../LICENSE)开源。

---

<p align="center">
  <sub>本项目与 Mirantis Lens 及 Freelens 项目均无关联。</sub>
</p>
