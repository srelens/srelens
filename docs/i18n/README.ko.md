<p align="center">
  <img src="../assets/logo-full.svg" alt="srelens" width="380" />
</p>

<h3 align="center">엔지니어와 AI 에이전트를 위한, Rust로 만든 Kubernetes 컨트롤 룸</h3>

<p align="center">
  <a href="../../README.md">English</a> ·
  <a href="README.zh-CN.md">简体中文</a> ·
  <a href="README.ja.md">日本語</a> ·
  <b>한국어</b> ·
  <a href="README.es.md">Español</a> ·
  <a href="README.pt-BR.md">Português</a> ·
  <a href="README.de.md">Deutsch</a> ·
  <a href="README.fr.md">Français</a>
</p>

<p align="center">
  이 문서는 번역본입니다. <a href="../../README.md">영어 README</a>가 원본이므로, 두 문서의 내용이 다르면 영어 README를 따르십시오.
</p>

<p align="center">
  srelens는 SRE, 플랫폼 엔지니어, DevOps 엔지니어를 위한 오픈 소스 로컬 우선(local-first)
  Kubernetes 워크스페이스입니다. <b>데스크톱 앱</b>, <b>터미널 UI</b>, 또는 AI 에이전트가
  호출할 수 있는 <b>MCP 서버</b>를 통해 여러 클러스터를 조사하고 분석하며 안전하게 조치할 수
  있습니다. 세 가지 모두 하나의 순수 Rust 코어로 동작합니다.
</p>

<p align="center">
  <a href="https://srelens.com">웹사이트</a> ·
  <a href="#설치">설치</a> ·
  <a href="#데스크톱-앱">데스크톱 앱</a> ·
  <a href="#터미널-ui-srectl">터미널 UI</a> ·
  <a href="#mcp-서버">MCP 서버</a> ·
  <a href="../USAGE.md">사용자 가이드</a> ·
  <a href="../DEVELOPMENT.md">개발자 가이드</a> ·
  <a href="../../CONTRIBUTING.md">기여하기</a>
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
    <img alt="새 디자인이 적용된 srelens 데스크톱 앱: 노드 용량, 준비되지 않은 워크로드, 종류(kind)별 오브젝트를 보여 주는 클러스터 개요" src="../assets/screenshots/gui-overview-dark.webp" width="100%">
  </picture>
</p>

---

## 인터페이스 고르기

|  | 데스크톱 앱 (GUI) | 터미널 UI (`srectl`) |
| --- | --- | --- |
| **무엇인가요** | Tauri v2와 React 19 기반의 네이티브 창 | ratatui로 만든 자체 완결형 단일 바이너리 |
| **이럴 때 좋습니다** | 일상적인 클러스터 작업, YAML 편집, 토폴로지, 여러 클러스터를 나란히 띄워 놓고 하는 작업 | SSH 세션, 점프 호스트, 키보드만으로 하는 작업, k9s에 익숙한 손 |
| **실행 환경** | macOS, Linux, Windows — 또는 [웹 모드](../WEB.md)로 브라우저에서 | x86-64 및 arm64용 Linux(glibc 및 정적 musl)와 macOS, x86-64용 Windows |
| **설치** | [릴리스 다운로드](#설치) | `brew install srelens/tap/srectl` 또는 [한 줄 명령](#설치) |
| **AI** | 내장 어시스턴트, 그리고 **Settings → MCP**(설정 → MCP)의 MCP 서버 | `:ai` 어시스턴트, 그리고 stdio로 동작하는 `srectl mcp` |

두 인터페이스 모두 로컬 kubeconfig에 있는 자격 증명으로 클러스터와 직접 통신합니다.
어떤 요청도 srelens 클라우드 서비스를 거치지 않습니다.

## 왜 srelens인가요?

Kubernetes 트러블슈팅을 하다 보면 보통 터미널, 대시보드, YAML 편집기, 로그, 클러스터
컨텍스트 사이를 계속 오가게 됩니다. srelens는 이 조사 흐름 전체를 하나의 로컬 우선
워크스페이스로 모았습니다.

- **조사부터 조치까지 하나의 워크스페이스에서** — 도구를 바꾸지 않고도 리소스를 탐색하고,
  이벤트와 YAML을 읽고, 로그를 따라가고, 셸을 열고, 포트를 포워딩하고, 클러스터 작업을
  수행할 수 있습니다.
- **엔지니어와 AI 에이전트를 위한 설계** — 앱에서 사용하는 백엔드 기능이 내장 MCP 서버를
  통해 그대로 노출됩니다.
- **로컬 우선 클러스터 접근** — srelens는 사용자의 kubeconfig 자격 증명으로 API 서버에
  직접 연결합니다.
- **안전한 작업** — 파괴적인 작업은 앱, 터미널, MCP 어디에서든 식별되며, 확인을 거쳐야만
  실행됩니다.
- **작고 빠름** — 운영 체제의 WebView 위에서 동작하는 Rust 코어입니다. srelens `v0.5.0`의
  macOS 설치 파일은 **17.1 MiB**, Linux `.deb`는 **19.8 MiB**로, Freelens `v1.10.3`의
  190.2 MiB와 146.8 MiB보다 각각 약 **11배, 7배 작았습니다**. [성능 기준치](../PERFORMANCE.md)
  문서에서 이 결과를 재현하는 방법과, 격차가 더 좁은 부분까지 확인할 수 있습니다.
- **오픈 소스** — MIT 라이선스이며 코드, 릴리스, 이슈, 로드맵이 모두 공개되어 있습니다.

## 데스크톱 앱

데스크톱 앱은 네이티브 창에서 동작하는 완전한 Kubernetes 워크스페이스입니다. 아래
스크린샷은 화면 단위로 순차 적용 중인 **새 디자인**입니다.
**Settings → Appearance → Design → New design**(설정 → 모양 → 디자인 → 새 디자인)에서
켤 수 있으며, 언제든 같은 경로로 되돌릴 수 있습니다. 테마는 Light, Paper, Dark, Midnight,
Glass, High contrast 여섯 가지입니다.

<table>
  <tr>
    <td width="50%"><img alt="한 네임스페이스로 필터링한 Pod 목록과 빨간색으로 표시된 크래시 루프 Pod" src="../assets/screenshots/gui-pods.webp"><br><sub><b>실시간 리소스</b> — 필터, 열 설정, 일괄 작업을 지원하는 watch 기반 테이블</sub></td>
    <td width="50%"><img alt="준비 상태, 재시작 횟수, CPU, 메모리와 로그·셸·포트 포워딩 바로 가기 작업이 있는 Pod 상세 화면" src="../assets/screenshots/gui-pod-detail.webp"><br><sub><b>리소스 상세</b> — 상태, 이벤트, 소유자, 그리고 클릭 한 번으로 여는 로그·셸·포워딩</sub></td>
  </tr>
  <tr>
    <td><img alt="실패 중인 Pod의 로그 스트림과 빨간색으로 표시된 치명적 connection-refused 줄" src="../assets/screenshots/gui-logs.webp"><br><sub><b>로그</b> — 여러 Pod 스트림, 이전 인스턴스, 필터, 내보내기, AI 요약</sub></td>
    <td><img alt="검증, diff, dry run, apply를 지원하는 Deployment용 스키마 인식 YAML 편집기" src="../assets/screenshots/gui-yaml.webp"><br><sub><b>YAML</b> — 스키마 인식 편집, 서버 dry run, diff, server-side apply</sub></td>
  </tr>
  <tr>
    <td><img alt="실패 중인 Deployment와 그것이 호출하는 외부 데이터베이스를 보여 주는 서비스 토폴로지 그래프" src="../assets/screenshots/gui-topology.webp"><br><sub><b>토폴로지</b> — 누가 누구를 호출하는지, 장애가 어디서 시작되는지</sub></td>
    <td><img alt="두 리비전 사이의 렌더링된 diff를 보여 주는 Helm 릴리스" src="../assets/screenshots/gui-helm.webp"><br><sub><b>Helm</b> — 렌더링된 diff를 보며 설치, 업그레이드, 롤백, 제거</sub></td>
  </tr>
</table>

데스크톱 앱에서 할 수 있는 일은 다음과 같습니다. 각 기능은 [사용자 가이드](../USAGE.md)에서
자세히 다룹니다.

- **멀티 클러스터 워크스페이스** — kubeconfig 컨텍스트를 찾아 불러오고, 파일을 더 추가하거나
  붙여 넣고, 컨텍스트마다 이름·로고·색상을 지정하고, 클러스터 레일에서 클러스터를 전환할 수
  있습니다. 여러 파일에 같은 이름의 컨텍스트가 있어도 서로 구분됩니다.
- **실시간 Kubernetes 리소스** — 워크로드, 네트워킹, 스토리지, RBAC, admission, 오토스케일링,
  커스텀 리소스를 실시간 watch, 검색, 열 선택기, 네임스페이스 범위 지정, 일괄 작업과 함께
  다룰 수 있습니다.
- **리소스 상세와 YAML** — 매니페스트, 이벤트, 관계, 메트릭을 보여 주고, 검증·dry-run diff·
  server-side apply를 지원하는 스키마 인식 YAML 편집을 제공합니다.
- **로그** — Pod 또는 워크로드 단위 스트림으로, 이전 인스턴스 로그, 타임스탬프, tail 및
  since 범위 제어, 소스별 색상, 컨테이너 필터, 내보내기를 지원합니다.
- **터미널과 셸** — Pod exec, 컨텍스트 범위의 로컬 터미널, distroless Pod를 위한 임시
  (ephemeral) 디버그 컨테이너, 특권(privileged) 노드 셸을 제공합니다.
- **포트 포워딩** — 열려 있는 모든 클러스터에 걸쳐 포워딩을 생성, 확인, 복사, 중지할 수
  있습니다.
- **Helm** — 릴리스를 확인하고, values 편집기와 렌더링된 diff 미리보기를 보며 설치,
  업그레이드, 롤백, 제거할 수 있습니다.
- **운영 작업** — 스케일 조정, 롤아웃 재시작, Pod 축출(evict) 또는 삭제, CronJob 일시 중지
  또는 수동 실행, 노드 cordon 및 drain을 지원하며, 모두 확인 절차를 거쳐야 실행됩니다.
- **툴박스** — `kubectl`, `krew`, `helm`과 krew 플러그인을 설치·관리하고, 컨텍스트의 exec
  인증 요구 사항을 진단합니다.
- **메트릭** — `metrics-server`를 사용할 수 있으면 노드와 Pod의 CPU 및 메모리를 보여 줍니다.
- **AI 어시스턴트** — 지금 보고 있는 클러스터에 대해 질문하고, 로그 스트림을 요약하거나 Helm
  diff에 대한 설명을 받을 수 있습니다.
- **커맨드 팔레트** — 뷰, 컨텍스트, 리소스를 넘나드는 키보드 중심 탐색(Cmd/Ctrl-K)을
  제공합니다.
- **웹 모드** — 같은 앱을 Docker에서 멀티 유저 웹 서버로 실행할 수 있으며, OIDC 로그인과
  사용자별 격리를 지원합니다. [docs/WEB.md](../WEB.md)를 참고하십시오.

## 터미널 UI (`srectl`)

`srectl`은 터미널에서 쓰는 srelens입니다. 단일 바이너리에 k9s 스타일 탐색을 갖추었고,
데스크톱 앱과 같은 Rust 코어를 사용합니다. SSH 접속 중이든, 점프 호스트 위든, 창을 띄울 수
없는 곳이라면 어디서든 제 몫을 합니다.

<p align="center">
  <img alt="네임스페이스, 준비 상태, 상태, 재시작 횟수, IP, 노드 열이 있는 실시간 Pod 테이블을 보여 주는 srectl" src="../assets/screenshots/tui-pods.webp" width="100%">
</p>

<table>
  <tr>
    <td width="50%"><img alt="Deployment에서 ReplicaSet, Pod로 이어지는 리소스 관계 트리와 그 컨테이너, ConfigMap, Service" src="../assets/screenshots/tui-tree.webp"><br><sub><b>관계 트리</b> — 소유자, 하위 리소스, 설정, 서비스를 한 화면에서</sub></td>
    <td width="50%"><img alt="줄 번호와 follow, 타임스탬프, 줄 바꿈, 저장 컨트롤이 있는 Pod의 실시간 로그 스트림" src="../assets/screenshots/tui-logs.webp"><br><sub><b>로그</b> — follow, 검색, 타임스탬프, 이전 인스턴스, 저장</sub></td>
  </tr>
  <tr>
    <td><img alt="values diff, 리비전, 매니페스트, 노트 탭이 있는 Helm 릴리스 개요" src="../assets/screenshots/tui-helm.webp"><br><sub><b>Helm</b> — 리비전, values diff, 매니페스트, 롤백</sub></td>
    <td><img alt="노드별 GPU 및 VRAM 할당량과 GPU를 요청한 Pod를 보여 주는 GPU 인스펙터" src="../assets/screenshots/tui-gpu.webp"><br><sub><b>GPU 인스펙터</b> — 노드별, Pod별 GPU 및 VRAM 할당</sub></td>
  </tr>
  <tr>
    <td><img alt="허브 및 스포크 클러스터 전반의 동기화 상태와 헬스 상태를 보여 주는 Argo CD 애플리케이션" src="../assets/screenshots/tui-argo.webp"><br><sub><b>Argo CD</b> — 허브·스포크 클러스터 전반의 동기화, 헬스, 드리프트</sub></td>
    <td><img alt="listNodes 도구를 호출한 뒤 T4 GPU가 있는 노드를 알려 주는 AI 어시스턴트" src="../assets/screenshots/tui-assistant.webp"><br><sub><b>AI 어시스턴트</b> — 도구를 통해 클러스터에 직접 질의하고, SRE 플레이북을 활용</sub></td>
  </tr>
</table>

- **k9s 스타일 탐색** — `:`을 누르면 명령 프롬프트가 열립니다(`:pod`, `:deploy`, `:svc`,
  `:no`, `:ns`, `:helm`, `:pf`, `:ctx` …). `/`로 필터링하고, `j`/`k`로 이동하며, `Enter`로
  하위 항목에 들어가고 `Esc`로 돌아갑니다.
- **보이는 대상에 바로 작업** — 로그(`l`), 셸(`s`), describe(`d`), YAML(`y`), 편집(`e`),
  포트 포워딩(`f`), 삭제(`Ctrl-d`), 그리고 재시작·스케일 조정 등을 위한 빠른 작업
  팔레트(`x`)를 제공합니다.
- **조사** — 관계 트리(`t`), CPU·메모리 기준 핫스팟을 보여 주는 `:top`, `:topo` 토폴로지,
  클러스터 개요, 복구용 노드 SSH를 제공합니다.
- **기본 기능 그 이상** — Argo CD(`:argo`), GPU 및 VRAM(`:gpuinfo`), BGP 피어링(`:bgp`)
  뷰를 제공합니다.
- **AI 어시스턴트** — `:ai`에서 Anthropic(Claude), OpenAI, Google Gemini, Ollama 같은
  OpenAI 호환 엔드포인트, 또는 Cursor Agent를 사용할 수 있습니다.
- **헤드리스 보조 명령** — `srectl info`는 kubeconfig의 컨텍스트를 나열하고,
  `srectl toolbox`는 `kubectl`, `helm`, `krew`가 있는지 확인하며, `srectl mcp`는 stdio로
  MCP 서버를 실행합니다.
- **서명된 자체 업데이트** — `srectl update`는 체크섬이 바이너리에 내장된 srelens 릴리스
  키로 서명된 경우에만 릴리스를 설치합니다.

```bash
srectl                      # 현재 컨텍스트로 시작
srectl -n payments          # 네임스페이스를 지정해 Pods 뷰로 시작
srectl --context prod -A    # 컨텍스트 지정, 모든 네임스페이스
srectl pods/my-pod          # 리소스를 바로 열기
srectl --help               # 전체 플래그 보기
```

## 설치

모든 릴리스는 [GitHub Releases](https://github.com/srelens/srelens/releases/latest)에 있습니다.
[설치 가이드](../INSTALL.md)에서 플랫폼별 첫 실행, 업데이트, 다운로드 검증, 제거 방법을
다룹니다.

### 데스크톱 앱

| 플랫폼 | 패키지 | 참고 |
| --- | --- | --- |
| macOS | Apple Silicon 및 Intel용 `.dmg` | Developer ID 서명 및 공증(notarization) 완료 |
| Linux | `.AppImage`, `.deb`, `.rpm`, 그리고 AUR의 [`srelens-bin`](https://aur.archlinux.org/packages/srelens-bin) | AppImage는 앱 내 업데이터를 지원합니다 |
| Windows | `.exe`, `.msi` | 코드 서명이 아직 로드맵 단계이므로 Windows에서 SmartScreen 경고가 표시될 수 있습니다 |

### 터미널 UI

macOS(Homebrew):

```bash
brew install srelens/tap/srectl
```

Linux에서는 한 줄로 설치합니다:

```bash
( f="$(mktemp)" && trap 'rm -f "$f"' EXIT &&
  curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh -o "$f" &&
  sh "$f" )
```

또는 릴리스에서 `srectl-<version>-<target>.tar.gz`(Windows는 `.zip`)를 내려받을 수
있습니다. x86-64 및 arm64용 Linux(glibc 및 정적 musl)와 macOS, 그리고 x86-64용 Windows
빌드가 제공됩니다. stable 릴리스의 macOS 빌드는 서명 및 공증되어 있습니다.

### 웹 앱 (Docker)

srelens는 컨테이너에서 멀티 유저 웹 서버로도 실행됩니다. 사용자는 OIDC(또는 체험용 로컬
개발 로그인)로 로그인하며, 각 사용자는 본인이 업로드한 kubeconfig만으로 구성된 격리된 환경을
갖게 됩니다. OIDC로 보호되는 클러스터는 `kubelogin`이나 exec 플러그인 없이 브라우저에서
로그인하며, kubeconfig와 토큰은 필수 설정인 `SRELENS_MASTER_KEY`로 저장 시 봉인(sealed at
rest)됩니다. 배포 방법과 전체 보안 모델은 [docs/WEB.md](../WEB.md)를 참고하십시오.

## MCP 서버

srelens에는 백엔드와 같은 capability 레지스트리에서 생성되는 MCP 서버가 포함되어 있습니다.
따라서 MCP를 지원하는 클라이언트는 별도의 통합 계층 없이 동일한 클러스터 작업을 사용할 수
있습니다.

데스크톱 앱에서 **Settings → MCP**(설정 → MCP)를 열면 다음을 할 수 있습니다.

- 공개(reveal), 교체(rotate), 폐기(revoke)할 수 있는 bearer 토큰으로 보호되는 루프백 HTTP로
  서버를 실행합니다. 토큰을 교체하면 실행 중인 서버가 재시작되어 새 토큰이 즉시 적용되고,
  폐기하면 서버가 중지됩니다.
- stdio 연결용 `srelens` CLI를 설치합니다. 클라이언트가 프로세스를 직접 띄우는 시점에 이미
  사용자의 권한을 갖고 있으므로, stdio 연결에는 토큰이 필요하지 않습니다.
- 지원되는 MCP 클라이언트용 설정을 복사합니다.

또는 데스크톱 바이너리나 터미널 UI에서 직접 실행할 수 있습니다:

```sh
srelens --mcp-stdio
srelens --mcp-http 127.0.0.1:8765
srectl mcp
```

stdio 설정 예시:

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

파괴적인 도구는 앱에서 확인을 요청합니다. 헤드리스 실행에서는 확인 대화 상자를 띄울 수
없으므로, 호출에 `"_confirm": true`를 지정하는 것과 *함께* 프로세스 수준의 명시적 허용
(opt-in)이 필요합니다. 무엇이든 변경하려면 `--mcp-allow-destructive`, Secret을 읽으려면
`--mcp-allow-sensitive-reads`를 사용합니다(`srectl mcp`에서는 `--allow-destructive`와
`--allow-sensitive-reads`). 두 옵션은 서로 독립적이므로, Secret을 읽을 수 있다고 해서 노드를
drain할 권한까지 생기지는 않습니다. Host 헤더 검사, 감사 로그, 토큰 저장 방식을 포함한 전체
보안 모델은 [docs/MCP.md](../MCP.md)를 참고하십시오.

> MCP 접근에는 로컬에서 인증된 클러스터 컨텍스트가 그대로 사용됩니다. 특히 중요한
> 클러스터에서는 도구 호출을 검토하고 적절한 Kubernetes RBAC 권한을 사용하십시오.

## 소스에서 빌드하기

### 사전 요구 사항

- [Rust](https://rustup.rs) stable
- [Node.js](https://nodejs.org) 26(개발 기본값이며, Node 24 LTS도 지원합니다). `.nvmrc`에 지정된 버전을 사용하려면 `nvm install && nvm use`를 실행하십시오.
- [pnpm](https://pnpm.io) 9+
- [Tauri v2 시스템 의존성](https://v2.tauri.app/start/prerequisites/)
- 클러스터가 필요한 워크플로를 위한, 접근 가능한 Kubernetes 클러스터

### 로컬에서 실행하기

```sh
git clone https://github.com/srelens/srelens
cd srelens
pnpm install
pnpm dev                  # the desktop app
cargo run -p srectl       # the terminal UI
```

### 자주 쓰는 명령어

| 명령어 | 용도 |
| --- | --- |
| `pnpm dev` | 데스크톱 애플리케이션을 개발 모드로 실행 |
| `cargo run -p srectl` | 터미널 UI 실행 |
| `pnpm test` | JavaScript 및 TypeScript 테스트 실행 |
| `cargo test` | Rust 워크스페이스 테스트 실행 |
| `pnpm build` | 프로덕션 프론트엔드 빌드 |
| `pnpm tauri build` | 패키징된 데스크톱 바이너리 생성 |

아키텍처, 테스트 기준, capability 추가 방법은 [개발자 가이드](../DEVELOPMENT.md)를
참고하십시오.

## 아키텍처

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

저장소 구조:

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

## 프로젝트 현황

srelens는 **안정 버전이며 활발히 개발 중**이고, macOS, Linux, Windows용 릴리스를 정기적으로
배포합니다. macOS 빌드는 Developer ID로 서명 및 공증되어 있으며, 지원되는 패키지는 재설치 없이
그 자리에서 업데이트됩니다. 새 기능이 계속 추가되므로 업그레이드할 때 릴리스 노트를 한번
훑어보시기 바랍니다.

- [최신 릴리스](https://github.com/srelens/srelens/releases/latest)
- [전체 릴리스](https://github.com/srelens/srelens/releases)
- [이슈 및 로드맵](https://github.com/srelens/srelens/issues)

## 커뮤니티

- [Reddit의 r/srelens](https://www.reddit.com/r/srelens/) — 공지, 질문, 피드백
- [이슈](https://github.com/srelens/srelens/issues) — 버그 제보 및 기능 요청
- [웹사이트](https://srelens.com)

## 기여하기

기여를 환영합니다. [기여 가이드](../../CONTRIBUTING.md), [개발자 가이드](../DEVELOPMENT.md),
[MCP 에이전트 통합 가이드](../MCP.md), [열린 이슈](https://github.com/srelens/srelens/issues)부터
살펴보십시오. 참여하기 전에 [행동 강령](../../.github/CODE_OF_CONDUCT.md)을 읽어 주십시오.

번역본은 [`docs/i18n`](.)에 있습니다. 영어 README가 원본이며, 번역 수정이나 새 언어 추가는
pull request로 보내 주시면 환영합니다.

## 라이선스

srelens는 [MIT 라이선스](../../LICENSE)에 따라 공개된 오픈 소스입니다.

---

<p align="center">
  <sub>Mirantis Lens 또는 Freelens 프로젝트와는 관련이 없습니다.</sub>
</p>
