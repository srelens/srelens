<p align="center">
  <img src="../assets/logo-full.svg" alt="srelens" width="380" />
</p>

<h3 align="center">Rust で作られた、エンジニアと AI エージェントのための Kubernetes コントロールルーム。</h3>

<p align="center">
  <a href="../../README.md">English</a> ·
  <a href="README.zh-CN.md">简体中文</a> ·
  <b>日本語</b> ·
  <a href="README.ko.md">한국어</a> ·
  <a href="README.es.md">Español</a> ·
  <a href="README.pt-BR.md">Português</a> ·
  <a href="README.de.md">Deutsch</a> ·
  <a href="README.fr.md">Français</a>
</p>

<p align="center">
  <sub>これは翻訳版です。<a href="../../README.md">英語版 README</a> を正とし、内容に相違がある場合は英語版を優先してください。</sub>
</p>

<p align="center">
  srelens は、SRE、プラットフォームエンジニア、DevOps エンジニアのための、オープンソースでローカルファーストな Kubernetes ワークスペースです。<b>デスクトップアプリ</b>から、<b>ターミナル UI</b> から、あるいは AI エージェントが呼び出せる <b>MCP サーバー</b>を通じて、複数のクラスターにまたがる調査・分析と安全な操作を行えます。この 3 つはすべて、ピュア Rust で書かれた単一のコアで動いています。
</p>

<p align="center">
  <a href="https://srelens.com">Web サイト</a> ·
  <a href="#インストール">インストール</a> ·
  <a href="#デスクトップアプリ">デスクトップアプリ</a> ·
  <a href="#ターミナル-uisrectl">ターミナル UI</a> ·
  <a href="#mcp-サーバー">MCP サーバー</a> ·
  <a href="../USAGE.md">ユーザーガイド</a> ·
  <a href="../DEVELOPMENT.md">開発者ガイド</a> ·
  <a href="../../CONTRIBUTING.md">コントリビューション</a>
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
    <img alt="新デザインの srelens デスクトップアプリ：ノードの容量、Ready になっていないワークロード、種類別のオブジェクトを表示するクラスター概要" src="../assets/screenshots/gui-overview-dark.webp" width="100%">
  </picture>
</p>

---

## インターフェースを選ぶ

|  | デスクトップアプリ（GUI） | ターミナル UI（`srectl`） |
| --- | --- | --- |
| **概要** | Tauri v2 と React 19 で作られたネイティブウィンドウ | ratatui で作られた、単体で動作する 1 つのバイナリ |
| **向いている用途** | 日々のクラスター作業、YAML の編集、トポロジー、多数のクラスターを並べての作業 | SSH セッション、踏み台ホスト、キーボードだけの操作、k9s で身についた操作感 |
| **動作環境** | macOS、Linux、Windows。[Web モード](../WEB.md)ならブラウザーでも利用可能 | x86-64 と arm64 の Linux（glibc および静的 musl）と macOS、x86-64 の Windows |
| **インストール** | [リリースをダウンロード](#インストール) | Homebrew または[ワンライナー](#インストール) |
| **AI** | 組み込みのアシスタントに加え、**Settings → MCP**（設定 → MCP）から MCP サーバー | `:ai` アシスタントに加え、stdio 経由の `srectl mcp` |

どちらも、ローカルの kubeconfig にある認証情報を使ってクラスターと直接通信します。srelens のクラウドサービスを経由することは一切ありません。

## srelens を選ぶ理由

Kubernetes のトラブルシューティングでは、ターミナル、ダッシュボード、YAML エディター、ログ、クラスターのコンテキストの間を行き来するのが当たり前になりがちです。srelens は、こうした調査のサイクルを 1 つのローカルファーストなワークスペースにまとめます。

- **調査からアクションまで 1 つのワークスペースで** — ツールを切り替えることなく、リソースの閲覧、イベントや YAML の確認、ログの追跡、シェルの起動、ポートフォワード、クラスターへの操作を行えます。
- **エンジニアと AI エージェントのために設計** — アプリで使うバックエンドの機能は、組み込みの MCP サーバーで公開されている機能とまったく同じものです。
- **ローカルファーストなクラスターアクセス** — srelens は、あなた自身の kubeconfig の認証情報を使って API サーバーに直接接続します。
- **安全な操作** — 破壊的な操作はそれと識別され、アプリでもターミナルでも MCP 経由でも、確認を経なければ実行されません。
- **小さく、速い** — OS の WebView の上で動く Rust コア。srelens `v0.5.0` では、macOS インストーラーが **17.1 MiB**、Linux の `.deb` が **19.8 MiB** で、Freelens `v1.10.3` の 190.2 MiB と 146.8 MiB に比べて、それぞれ約 **11 分の 1、7 分の 1** のサイズでした。[パフォーマンスのベースライン](../PERFORMANCE.md)では、差がより小さくなるケースも含めて、この結果を再現する方法を説明しています。
- **オープンソース** — MIT ライセンスで、コード、リリース、Issue、ロードマップを公開しています。

## デスクトップアプリ

デスクトップアプリは、ネイティブウィンドウで動く本格的な Kubernetes ワークスペースです。以下のスクリーンショットは**新デザイン**のもので、画面ごとに順次展開しています。**Settings → Appearance → Design → New design**（設定 → 外観 → デザイン → 新デザイン）で有効にでき、同じ手順でいつでも元に戻せます。テーマは Light、Paper、Dark、Midnight、Glass、High contrast の 6 種類です。

<table>
  <tr>
    <td width="50%"><img alt="1 つの Namespace に絞り込んだ Pod の一覧。クラッシュループしている Pod が赤で示されている" src="../assets/screenshots/gui-pods.webp"><br><sub><b>ライブリソース</b> — watch で常に最新に保たれるテーブル。フィルター、列の選択、一括操作に対応</sub></td>
    <td width="50%"><img alt="Pod の詳細ビュー。Readiness、再起動回数、CPU、メモリと、ログ・シェル・ポートフォワードへのクイックアクションを表示" src="../assets/screenshots/gui-pod-detail.webp"><br><sub><b>リソースの詳細</b> — ステータス、イベント、オーナーに加え、ワンクリックでログ・シェル・フォワード</sub></td>
  </tr>
  <tr>
    <td><img alt="障害が起きている Pod のログストリーム。致命的な connection refused の行が赤で強調されている" src="../assets/screenshots/gui-logs.webp"><br><sub><b>ログ</b> — 複数 Pod のストリーム、前回のインスタンス、フィルター、エクスポート、AI による要約</sub></td>
    <td><img alt="Deployment 用のスキーマ対応 YAML エディター。検証、差分、ドライラン、適用に対応" src="../assets/screenshots/gui-yaml.webp"><br><sub><b>YAML</b> — スキーマ対応の編集、サーバー側ドライラン、差分、server-side apply</sub></td>
  </tr>
  <tr>
    <td><img alt="障害が起きている Deployment と、それが呼び出している外部データベースを示すサービストポロジーのグラフ" src="../assets/screenshots/gui-topology.webp"><br><sub><b>トポロジー</b> — どこがどこを呼び出しているのか、障害がどこから始まっているのか</sub></td>
    <td><img alt="2 つのリビジョン間のレンダリング済み差分を表示した Helm リリース" src="../assets/screenshots/gui-helm.webp"><br><sub><b>Helm</b> — レンダリング済みの差分を確認しながら、インストール、アップグレード、ロールバック、アンインストール</sub></td>
  </tr>
</table>

主な機能は次のとおりです。それぞれの詳細は[ユーザーガイド](../USAGE.md)で解説しています。

- **マルチクラスターワークスペース** — kubeconfig のコンテキストを検出し、ファイルを追加したり貼り付けたりできます。各コンテキストに名前・ロゴ・色を設定し、クラスターレールからクラスターを切り替えられます。別々のファイルにある同じ名前のコンテキストも、区別して扱われます。
- **ライブな Kubernetes リソース** — ワークロード、ネットワーク、ストレージ、RBAC、アドミッション、オートスケーリング、カスタムリソースを、ライブ watch、検索、列ピッカー、Namespace の絞り込み、一括操作とともに扱えます。
- **リソースの詳細と YAML** — マニフェスト、イベント、関連リソース、メトリクス。検証、ドライランの差分、server-side apply に対応したスキーマ対応の YAML。
- **ログ** — Pod 単位またはワークロード単位のストリーム。前回インスタンスのログ、タイムスタンプ、tail と since による範囲指定、ソースごとの色分け、コンテナーフィルター、エクスポートに対応しています。
- **ターミナルとシェル** — Pod への exec、コンテキストに紐づいたローカルターミナル、distroless な Pod 向けのエフェメラルデバッグコンテナー、特権付きのノードシェル。
- **ポートフォワード** — 開いているすべてのクラスターにまたがって、フォワードの作成、確認、コピー、停止ができます。
- **Helm** — リリースの確認に加え、values エディターとレンダリング済み差分のプレビューを使って、インストール、アップグレード、ロールバック、アンインストールを行えます。
- **運用操作** — スケール、ロールアウトの再起動、Pod の退避（evict）や削除、CronJob の一時停止や手動実行、ノードの cordon と drain。いずれも確認ゲートを経て実行されます。
- **ツールボックス** — `kubectl`、`krew`、`helm`、krew プラグインのインストールと管理に加え、コンテキストの exec 認証の要件を診断できます。
- **メトリクス** — `metrics-server` が利用できる場合は、ノードと Pod の CPU とメモリを表示します。
- **AI アシスタント** — 目の前のクラスターについて質問したり、ログストリームを要約したり、Helm の差分を説明させたりできます。
- **コマンドパレット** — ビュー、コンテキスト、リソースをまたいだキーボード中心のナビゲーション（Cmd/Ctrl-K）。
- **Web モード** — 同じアプリを、Docker 上のマルチユーザー Web サーバーとして実行できます。OIDC によるサインインとユーザーごとの分離に対応しています。[docs/WEB.md](../WEB.md) を参照してください。

## ターミナル UI（`srectl`）

`srectl` は、ターミナルで動く srelens です。単一のバイナリで、k9s 風のナビゲーションを備え、デスクトップアプリと同じ Rust コアを使っています。SSH 越しでも、踏み台ホスト上でも、ウィンドウを開けないあらゆる場所で本領を発揮します。

<p align="center">
  <img alt="Namespace、Readiness、ステータス、再起動回数、IP、ノードの列を持つライブな Pod テーブルを表示した srectl" src="../assets/screenshots/tui-pods.webp" width="100%">
</p>

<table>
  <tr>
    <td width="50%"><img alt="Deployment から ReplicaSet、Pod へと続くリソースの関係ツリー。コンテナー、ConfigMap、Service も表示されている" src="../assets/screenshots/tui-tree.webp"><br><sub><b>関係ツリー</b> — オーナー、子リソース、設定、サービスを 1 つのビューで</sub></td>
    <td width="50%"><img alt="行番号付きの Pod のライブログストリーム。フォロー、タイムスタンプ、折り返し、保存の操作を備える" src="../assets/screenshots/tui-logs.webp"><br><sub><b>ログ</b> — フォロー、検索、タイムスタンプ、前回のインスタンス、保存</sub></td>
  </tr>
  <tr>
    <td><img alt="values の差分、リビジョン、マニフェスト、ノートのタブを備えた Helm リリースの概要" src="../assets/screenshots/tui-helm.webp"><br><sub><b>Helm</b> — リビジョン、values の差分、マニフェスト、ロールバック</sub></td>
    <td><img alt="ノードごとの GPU と VRAM の割り当てと、GPU を要求している Pod を表示する GPU インスペクター" src="../assets/screenshots/tui-gpu.webp"><br><sub><b>GPU インスペクター</b> — ノード単位・Pod 単位の GPU と VRAM の割り当て</sub></td>
  </tr>
  <tr>
    <td><img alt="ハブクラスターとスポーククラスターにまたがる、同期状態とヘルス状態を表示した Argo CD アプリケーション" src="../assets/screenshots/tui-argo.webp"><br><sub><b>Argo CD</b> — ハブとスポークのクラスターにまたがる同期、ヘルス、ドリフト</sub></td>
    <td><img alt="listNodes ツールを呼び出したうえで、T4 GPU を搭載しているノードを回答する AI アシスタント" src="../assets/screenshots/tui-assistant.webp"><br><sub><b>AI アシスタント</b> — SRE のプレイブックを備え、ツールを通じてクラスターに問い合わせます</sub></td>
  </tr>
</table>

- **k9s 風のナビゲーション** — `:` でコマンドプロンプトを開き（`:pod`、`:deploy`、`:svc`、`:no`、`:ns`、`:helm`、`:pf`、`:ctx` …）、`/` で絞り込み、`j`/`k` で移動、`Enter` で掘り下げ、`Esc` で戻ります。
- **見ているものをそのまま操作** — ログ（`l`）、シェル（`s`）、describe（`d`）、YAML（`y`）、編集（`e`）、ポートフォワード（`f`）、削除（`Ctrl-d`）に加え、再起動やスケールなどを行えるクイックアクションパレット（`x`）があります。
- **調査** — 関係ツリー（`t`）、CPU とメモリのホットスポットを示す `:top`、`:topo` によるトポロジー、クラスター概要、そして復旧用のノード SSH。
- **基本機能のその先へ** — Argo CD（`:argo`）、GPU と VRAM（`:gpuinfo`）、BGP ピアリング（`:bgp`）のビュー。
- **AI アシスタント** — `:ai` で、Anthropic（Claude）、OpenAI、Google Gemini、Ollama などの OpenAI 互換エンドポイント、または Cursor Agent を利用できます。
- **ヘッドレスのヘルパー** — `srectl info` は kubeconfig 内のコンテキストを一覧表示し、`srectl toolbox` は `kubectl`、`helm`、`krew` の有無を確認し、`srectl mcp` は stdio で MCP サーバーを起動します。
- **署名付きのセルフアップデート** — `srectl update` は、リリースのチェックサムがバイナリに組み込まれた srelens のリリース鍵で署名されている場合にのみ、そのリリースをインストールします。

```bash
srectl                      # 現在のコンテキストで起動
srectl -n payments          # Pod ビューで起動し、Namespace を絞り込む
srectl --context prod -A    # コンテキストを指定し、すべての Namespace を対象にする
srectl pods/my-pod          # リソースを直接開く
srectl --help               # すべてのフラグを表示
```

## インストール

すべてのリリースは [GitHub Releases](https://github.com/srelens/srelens/releases/latest) で公開しています。[インストールガイド](../INSTALL.md)では、各プラットフォームでの初回起動、アップデート、ダウンロードの検証、アンインストールについて説明しています。

### デスクトップアプリ

| プラットフォーム | パッケージ | 備考 |
| --- | --- | --- |
| macOS | Apple Silicon 用と Intel 用の `.dmg` | Developer ID で署名され、公証（notarization）済み |
| Linux | `.AppImage`、`.deb`、`.rpm`、および AUR の [`srelens-bin`](https://aur.archlinux.org/packages/srelens-bin) | AppImage はアプリ内アップデーターに対応 |
| Windows | `.exe`、`.msi` | コード署名はまだロードマップ上の項目のため、Windows で SmartScreen の確認画面が表示されることがあります |

### ターミナル UI

> **v0.15.0 以前**のリリースでは、ターミナル UI は旧名の `srelens-tui` で公開されています。Linux のワンライナーはこれに対応しており、`srectl` としてインストールします。Homebrew では、新しい名前でのリリースが出るまで `brew install srelens/tap/srelens-tui` を使ってください。この場合、インストールされるコマンドは `srelens-tui` です。これらのリリースのアーカイブ名は `srelens-tui-<version>-<target>.tar.gz` です。

macOS（Homebrew）:

```bash
brew install srelens/tap/srectl
```

Linux（ワンライナー）:

```bash
( f="$(mktemp)" && trap 'rm -f "$f"' EXIT &&
  curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh -o "$f" &&
  sh "$f" )
```

または、リリースから `srectl-<version>-<target>.tar.gz`（Windows では `.zip`）をダウンロードすることもできます。x86-64 と arm64 の Linux（glibc および静的 musl）と macOS、そして x86-64 の Windows 向けに用意しています。安定版リリースの macOS ビルドは、署名と公証が済んでいます。

### Web アプリ（Docker）

srelens は、コンテナー内でマルチユーザーの Web サーバーとしても動作します。ユーザーは OIDC（試用時にはローカルの開発用ログイン）でサインインし、各ユーザーには、自分でアップロードした kubeconfig だけから構築される分離された環境が割り当てられます。OIDC で保護されたクラスターにはブラウザーからサインインでき、`kubelogin` や exec プラグインは必要ありません。kubeconfig とトークンは、必須の `SRELENS_MASTER_KEY` によって保存時に暗号化されます。デプロイ方法とセキュリティモデルの全体像については [docs/WEB.md](../WEB.md) を参照してください。

## MCP サーバー

srelens には、バックエンドと同じ capability レジストリから生成される MCP サーバーが含まれています。そのため MCP に対応したクライアントは、別途インテグレーション層を用意しなくても、同じクラスター操作を利用できます。

デスクトップアプリで **Settings → MCP**（設定 → MCP）を開くと、次のことができます。

- サーバーをループバック HTTP で起動する。サーバーは Bearer トークンで保護され、このトークンは表示、ローテーション、失効ができます。ローテーションすると実行中のサーバーが再起動して新しいトークンが即座に有効になり、失効させるとサーバーは停止します。
- stdio 接続用に `srelens` CLI をインストールする。stdio 接続にはトークンが必要ありません。クライアントはプロセスを起動する時点で、すでにあなたの権限を持っているためです。
- サポートされている MCP クライアント向けのクライアント設定をコピーする。

または、デスクトップアプリのバイナリやターミナル UI から直接起動することもできます。

```sh
srelens --mcp-stdio
srelens --mcp-http 127.0.0.1:8765
srectl mcp
```

stdio の設定例:

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

破壊的なツールは、アプリ内では実行前に確認を求めます。ヘッドレス実行では確認ダイアログを表示できないため、呼び出しに `"_confirm": true` を指定すること、*かつ*、プロセスレベルでのオプトインが必要です。何かを変更するには `--mcp-allow-destructive`、Secret を読み取るには `--mcp-allow-sensitive-reads` を指定します（`srectl mcp` では `--allow-destructive` と `--allow-sensitive-reads`）。この 2 つは互いに独立しているため、Secret を読み取れるからといって、ノードを drain する権限まで与えられることはありません。Host ヘッダーのチェック、監査ログ、トークンの保存方法を含むセキュリティモデルの全体像については、[docs/MCP.md](../MCP.md) を参照してください。

> MCP によるアクセスには、ローカルで認証済みのクラスターコンテキストが使われます。特に重要なクラスターでは、ツールの呼び出し内容を確認し、適切な Kubernetes RBAC 権限を使用してください。

## ソースからビルド

### 前提条件

- [Rust](https://rustup.rs) の stable 版
- [Node.js](https://nodejs.org) 26（開発時のデフォルト。Node 24 LTS もサポートしています）。`nvm install && nvm use` を実行すると、`.nvmrc` に記載されたバージョンを使えます。
- [pnpm](https://pnpm.io) 11 以上（正確なバージョンは `package.json` で固定されています）
- [Tauri v2 のシステム依存関係](https://v2.tauri.app/start/prerequisites/)
- クラスターに依存するワークフローのための、到達可能な Kubernetes クラスター

### ローカルで実行

```sh
git clone https://github.com/srelens/srelens
cd srelens
pnpm install
pnpm dev                  # the desktop app
cargo run -p srectl       # the terminal UI
```

### 便利なコマンド

| コマンド | 用途 |
| --- | --- |
| `pnpm dev` | デスクトップアプリケーションを開発モードで起動 |
| `cargo run -p srectl` | ターミナル UI を実行 |
| `pnpm test` | JavaScript と TypeScript のテストを実行 |
| `cargo test` | Rust ワークスペースのテストを実行 |
| `pnpm build` | 本番用のフロントエンドをビルド |
| `pnpm tauri build` | パッケージ化されたデスクトップバイナリを作成 |

アーキテクチャ、テストの基準、capability の追加方法については、[開発者ガイド](../DEVELOPMENT.md)を参照してください。

## アーキテクチャ

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

リポジトリの構成:

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

## プロジェクトの状況

srelens は**安定版であり、活発に開発が続いています**。macOS、Linux、Windows 向けに定期的にリリースしています。macOS ビルドは Developer ID で署名され公証済みで、対応しているパッケージはその場でアップデートできます。新しい機能が継続的に追加されているので、アップグレードの際にはリリースノートに目を通しておくことをおすすめします。

- [最新リリース](https://github.com/srelens/srelens/releases/latest)
- [すべてのリリース](https://github.com/srelens/srelens/releases)
- [Issue とロードマップ](https://github.com/srelens/srelens/issues)

## コミュニティ

- [Reddit の r/srelens](https://www.reddit.com/r/srelens/) — お知らせ、質問、フィードバック
- [Issues](https://github.com/srelens/srelens/issues) — バグ報告と機能リクエスト
- [Web サイト](https://srelens.com)

## コントリビューション

コントリビューションを歓迎します。まずは[コントリビューションガイド](../../CONTRIBUTING.md)、[開発者ガイド](../DEVELOPMENT.md)、[MCP エージェント連携ガイド](../MCP.md)、そして[未解決の Issue](https://github.com/srelens/srelens/issues) をご覧ください。参加する前に、[行動規範](../../.github/CODE_OF_CONDUCT.md)に目を通してください。

翻訳は [`docs/i18n`](.) にあります。英語版 README が正であり、翻訳の修正や新しい言語の追加は、プルリクエストとして歓迎します。

## ライセンス

srelens は [MIT License](../../LICENSE) のもとで公開されているオープンソースソフトウェアです。

---

<p align="center">
  <sub>Mirantis Lens および Freelens プロジェクトとは提携関係にありません。</sub>
</p>
