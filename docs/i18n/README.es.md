<p align="center">
  <img src="../assets/logo-full.svg" alt="srelens" width="380" />
</p>

<h3 align="center">La sala de control de Kubernetes: hecha en Rust, lista para ingenieros y agentes de IA.</h3>

<p align="center">
  <a href="../../README.md">English</a> ·
  <a href="README.zh-CN.md">简体中文</a> ·
  <a href="README.ja.md">日本語</a> ·
  <a href="README.ko.md">한국어</a> ·
  <b>Español</b> ·
  <a href="README.pt-BR.md">Português</a> ·
  <a href="README.de.md">Deutsch</a> ·
  <a href="README.fr.md">Français</a>
</p>

<p align="center">
  Esta es una traducción. El <a href="../../README.md">README en inglés</a> es la fuente de verdad; si no coinciden, confía en la versión en inglés.
</p>

<p align="center">
  srelens es un espacio de trabajo de Kubernetes de código abierto y local-first
  para SREs, ingenieros de plataforma e ingenieros de DevOps. Investiga, analiza y
  actúa de forma segura en tus clusters: desde una <b>aplicación de escritorio</b>,
  desde una <b>interfaz de terminal</b> o a través de un <b>servidor MCP</b> al que
  tu agente de IA puede llamar. Un único núcleo en Rust puro mueve las tres.
</p>

<p align="center">
  <a href="https://srelens.com">Sitio web</a> ·
  <a href="#instalación">Instalación</a> ·
  <a href="#aplicación-de-escritorio">Aplicación de escritorio</a> ·
  <a href="#interfaz-de-terminal-srectl">Interfaz de terminal</a> ·
  <a href="#servidor-mcp">Servidor MCP</a> ·
  <a href="../USAGE.md">Guía de uso</a> ·
  <a href="../DEVELOPMENT.md">Guía para desarrolladores</a> ·
  <a href="../../CONTRIBUTING.md">Contribuir</a>
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
    <img alt="La aplicación de escritorio de srelens con su nuevo diseño: una vista general del cluster con la capacidad de los nodos, las cargas de trabajo que no están listas y los objetos por tipo" src="../assets/screenshots/gui-overview-dark.webp" width="100%">
  </picture>
</p>

---

## Elige tu interfaz

|  | Aplicación de escritorio (GUI) | Interfaz de terminal (`srectl`) |
| --- | --- | --- |
| **Qué es** | Una ventana nativa hecha con Tauri v2 y React 19 | Un único binario autocontenido hecho con ratatui |
| **Ideal para** | El trabajo diario con clusters, la edición de YAML, la topología y muchos clusters a la vez | Sesiones SSH, jump hosts, trabajo solo con teclado y la memoria muscular de k9s |
| **Funciona en** | macOS, Linux y Windows, o en un navegador con el [modo web](../WEB.md) | Linux (glibc y musl estático) y macOS en x86-64 y arm64; Windows en x86-64 |
| **Instalación** | [Descarga una versión](#instalación) | `brew install srelens/tap/srectl` o [en una sola línea](#instalación) |
| **IA** | Asistente integrado, además del servidor MCP en **Settings → MCP** (Ajustes → MCP) | Asistente `:ai`, además de `srectl mcp` por stdio |

Las dos hablan directamente con tus clusters usando las credenciales de tu
kubeconfig local. Nada pasa por ningún servicio en la nube de srelens.

## ¿Por qué srelens?

Diagnosticar problemas en Kubernetes suele obligarte a saltar entre terminales,
dashboards, editores de YAML, logs y contextos de cluster. srelens reúne todo ese
ciclo de investigación en un único espacio de trabajo local-first.

- **Un solo espacio de trabajo, de la investigación a la acción** — explora
  recursos, lee eventos y YAML, sigue logs, abre shells, haz port forwarding y
  ejecuta acciones sobre el cluster sin cambiar de herramienta.
- **Pensado para ingenieros y agentes de IA** — las capacidades del backend que
  usas en la app son las mismas que expone el servidor MCP integrado.
- **Acceso local-first a los clusters** — srelens se conecta directamente a tus API
  servers con las credenciales de tu propio kubeconfig.
- **Operaciones seguras** — las acciones destructivas se identifican y exigen
  confirmación, en la app, en la terminal y por MCP.
- **Ligero y rápido** — un núcleo en Rust sobre el WebView del sistema operativo.
  srelens `v0.5.0` salió con un instalador para macOS de **17.1 MiB** y un `.deb`
  para Linux de **19.8 MiB**, frente a los 190.2 MiB y 146.8 MiB de Freelens
  `v1.10.3`: unas **11× y 7× más pequeños**. Las
  [mediciones de rendimiento de referencia](../PERFORMANCE.md) explican cómo
  reproducirlo, incluidos los casos en los que la diferencia es menor.
- **Código abierto** — licencia MIT, con código, versiones, issues y hoja de ruta
  públicos.

## Aplicación de escritorio

La aplicación de escritorio es un espacio de trabajo de Kubernetes completo en una
ventana nativa. Las capturas de abajo muestran el **nuevo diseño**, que se está
desplegando pantalla a pantalla: actívalo en
**Settings → Appearance → Design → New design** (Ajustes → Apariencia → Diseño →
Nuevo diseño) y vuelve al anterior por el mismo camino cuando quieras. Incluye seis
temas: Light, Paper, Dark, Midnight, Glass y High contrast.

<table>
  <tr>
    <td width="50%"><img alt="Lista de Pods filtrada a un namespace, con un pod en crash loop marcado en rojo" src="../assets/screenshots/gui-pods.webp"><br><sub><b>Recursos en vivo</b> — tablas alimentadas por watches, con filtros, columnas y acciones masivas</sub></td>
    <td width="50%"><img alt="Vista de detalle de un Pod con readiness, reinicios, CPU, memoria y acciones rápidas para logs, shell y port forward" src="../assets/screenshots/gui-pod-detail.webp"><br><sub><b>Detalles del recurso</b> — estado, eventos, owners, y logs, shell y forward con un clic</sub></td>
  </tr>
  <tr>
    <td><img alt="Stream de logs de un pod que falla, con una línea fatal de connection refused marcada en rojo" src="../assets/screenshots/gui-logs.webp"><br><sub><b>Logs</b> — streams de varios pods, instancia anterior, filtros, exportación y resúmenes con IA</sub></td>
    <td><img alt="Editor de YAML con soporte de esquema para un Deployment, con validación, diff, dry run y apply" src="../assets/screenshots/gui-yaml.webp"><br><sub><b>YAML</b> — edición con soporte de esquema, dry run en el servidor, diff y server-side apply</sub></td>
  </tr>
  <tr>
    <td><img alt="Grafo de topología de servicios que muestra un deployment que falla y la base de datos externa a la que llama" src="../assets/screenshots/gui-topology.webp"><br><sub><b>Topología</b> — quién llama a quién y dónde empieza el fallo</sub></td>
    <td><img alt="Releases de Helm con un diff renderizado entre dos revisiones" src="../assets/screenshots/gui-helm.webp"><br><sub><b>Helm</b> — instala, actualiza, haz rollback y desinstala con un diff renderizado</sub></td>
  </tr>
</table>

Qué puedes hacer con ella (la [guía de uso](../USAGE.md) cubre cada punto a fondo):

- **Espacio de trabajo multicluster** — descubre los contextos de tu kubeconfig,
  añade o pega más archivos, dale a cada contexto un nombre, un logo y un color, y
  cambia de cluster desde la barra lateral de clusters. Los contextos que comparten
  nombre en distintos archivos se mantienen separados.
- **Recursos de Kubernetes en vivo** — cargas de trabajo, red, almacenamiento,
  RBAC, admisión, autoescalado y recursos personalizados, con watches en vivo,
  búsqueda, selector de columnas, filtrado por namespace y acciones masivas.
- **Detalles de recursos y YAML** — manifiestos, eventos, relaciones y métricas;
  YAML con soporte de esquema, validación, diffs de dry run y server-side apply.
- **Logs** — streams de pods o de workloads con los logs de la instancia anterior,
  marcas de tiempo, controles de tail y de ventana temporal (since), colores por
  origen, filtros por contenedor y exportación.
- **Terminales y shells** — exec en pods, una terminal local ligada al contexto,
  contenedores de depuración efímeros para pods distroless y shells privilegiadas
  en nodos.
- **Port forwarding** — crea, inspecciona, copia y detén forwards en todos los
  clusters abiertos.
- **Helm** — inspecciona releases e instálalas, actualízalas, haz rollback o
  desinstálalas con un editor de values y una vista previa del diff renderizado.
- **Acciones operativas** — escala, reinicia rollouts, desaloja o elimina pods,
  suspende o lanza CronJobs, y haz cordon y drain de nodos, todo con confirmación
  previa.
- **Toolbox** — instala y gestiona `kubectl`, `krew`, `helm` y plugins de krew, y
  diagnostica los requisitos de exec-auth de un contexto.
- **Métricas** — CPU y memoria de nodos y pods cuando `metrics-server` está
  disponible.
- **Asistente de IA** — pregunta por el cluster que tienes delante, resume un
  stream de logs o pide que te explique un diff de Helm.
- **Paleta de comandos** — navegación pensada para el teclado (Cmd/Ctrl-K)
  entre vistas, contextos y recursos.
- **Modo web** — ejecuta la misma app como servidor web multiusuario en Docker,
  con inicio de sesión OIDC y aislamiento por usuario. Consulta
  [docs/WEB.md](../WEB.md).

## Interfaz de terminal (`srectl`)

`srectl` es srelens en tu terminal: un solo binario, navegación al estilo k9s y el
mismo núcleo en Rust que la aplicación de escritorio. Se mueve como pez en el agua
por SSH, en un jump host o en cualquier sitio donde no tengas una ventana.

<p align="center">
  <img alt="srectl mostrando una tabla de Pods en vivo con columnas de namespace, readiness, estado, reinicios, IP y nodo" src="../assets/screenshots/tui-pods.webp" width="100%">
</p>

<table>
  <tr>
    <td width="50%"><img alt="Árbol de relaciones de recursos de Deployment a ReplicaSet y a Pod, con su contenedor, ConfigMap y Service" src="../assets/screenshots/tui-tree.webp"><br><sub><b>Árbol de relaciones</b> — owners, hijos, configuración y servicios en una sola vista</sub></td>
    <td width="50%"><img alt="Stream de logs en vivo de un pod con números de línea y controles de follow, marcas de tiempo, ajuste de línea y guardado" src="../assets/screenshots/tui-logs.webp"><br><sub><b>Logs</b> — follow, búsqueda, marcas de tiempo, instancia anterior y guardado</sub></td>
  </tr>
  <tr>
    <td><img alt="Vista general de una release de Helm con pestañas para el diff de values, revisiones, manifiesto y notas" src="../assets/screenshots/tui-helm.webp"><br><sub><b>Helm</b> — revisiones, diff de values, manifiesto y rollback</sub></td>
    <td><img alt="Inspector de GPU que muestra la asignación de GPU y VRAM por nodo y los pods que solicitan GPUs" src="../assets/screenshots/tui-gpu.webp"><br><sub><b>Inspector de GPU</b> — asignación de GPU y VRAM por nodo y por pod</sub></td>
  </tr>
  <tr>
    <td><img alt="Aplicaciones de Argo CD con su estado de sync y de salud en clusters hub y spoke" src="../assets/screenshots/tui-argo.webp"><br><sub><b>Argo CD</b> — sync, salud y drift en clusters hub y spoke</sub></td>
    <td><img alt="Asistente de IA respondiendo qué nodos tienen una GPU T4 tras llamar a la herramienta listNodes" src="../assets/screenshots/tui-assistant.webp"><br><sub><b>Asistente de IA</b> — consulta al cluster mediante herramientas, con playbooks de SRE</sub></td>
  </tr>
</table>

- **Navegación al estilo k9s** — `:` abre el prompt de comandos (`:pod`, `:deploy`,
  `:svc`, `:no`, `:ns`, `:helm`, `:pf`, `:ctx` …), `/` filtra, `j`/`k` te mueven,
  `Enter` entra al detalle y `Esc` vuelve atrás.
- **Actúa sobre lo que ves** — logs (`l`), shell (`s`), describe (`d`), YAML (`y`),
  editar (`e`), port forward (`f`), eliminar (`Ctrl-d`) y una paleta de acciones
  rápidas (`x`) para reiniciar, escalar y más.
- **Investiga** — árbol de relaciones (`t`), los puntos calientes de CPU y memoria
  con `:top`, la topología con `:topo`, una vista general del cluster y SSH a los
  nodos para recuperarlos.
- **Más allá de lo básico** — vistas de Argo CD (`:argo`), GPU y VRAM (`:gpuinfo`)
  y peering BGP (`:bgp`).
- **Asistente de IA** — `:ai` con Anthropic (Claude), OpenAI, Google Gemini,
  cualquier endpoint compatible con OpenAI como Ollama, o Cursor Agent.
- **Utilidades headless** — `srectl info` lista los contextos de tu kubeconfig,
  `srectl toolbox` comprueba si tienes `kubectl`, `helm` y `krew`, y `srectl mcp`
  ejecuta el servidor MCP por stdio.
- **Autoactualización firmada** — `srectl update` solo instala una versión si sus
  checksums están firmados con una clave de release de srelens incluida en el
  binario.

```bash
srectl                      # arranca en el contexto actual
srectl -n payments          # arranca en la vista de Pods, acotada a un namespace
srectl --context prod -A    # elige un contexto, todos los namespaces
srectl pods/my-pod          # abre directamente un recurso
srectl --help               # todas las opciones
```

## Instalación

Todas las versiones están en [GitHub Releases](https://github.com/srelens/srelens/releases/latest).
La [guía de instalación](../INSTALL.md) cubre el primer arranque, las
actualizaciones, la verificación de las descargas y la desinstalación en cada
plataforma.

### Aplicación de escritorio

| Plataforma | Paquetes | Notas |
| --- | --- | --- |
| macOS | `.dmg` para Apple Silicon e Intel | Firmado con Developer ID y notarizado |
| Linux | `.AppImage`, `.deb`, `.rpm` y [`srelens-bin`](https://aur.archlinux.org/packages/srelens-bin) en AUR | El AppImage admite el actualizador integrado en la app |
| Windows | `.exe`, `.msi` | Windows puede mostrar un aviso de SmartScreen mientras la firma de código siga en la hoja de ruta |

### Interfaz de terminal

macOS (Homebrew):

```bash
brew install srelens/tap/srectl
```

Linux, en una sola línea:

```bash
( f="$(mktemp)" && trap 'rm -f "$f"' EXIT &&
  curl -fsSL https://raw.githubusercontent.com/srelens/srelens/main/packaging/install/install.sh -o "$f" &&
  sh "$f" )
```

O descarga `srectl-<version>-<target>.tar.gz` (o `.zip` en Windows) de la release,
para Linux (glibc y musl estático) y macOS en x86-64 y arm64, y para Windows en
x86-64. Los builds de macOS de una versión estable están firmados y notarizados.

### Aplicación web (Docker)

srelens también funciona como servidor web multiusuario dentro de un contenedor.
Los usuarios inician sesión con OIDC (o con un login local de desarrollo para
pruebas) y cada uno obtiene un entorno aislado construido únicamente a partir de
los kubeconfigs que ha subido. En los clusters protegidos con OIDC se inicia
sesión desde el navegador, sin necesidad de `kubelogin` ni de un exec plugin, y
los kubeconfigs y tokens se guardan cifrados en reposo con una
`SRELENS_MASTER_KEY` obligatoria. Consulta [docs/WEB.md](../WEB.md) para el
despliegue y el modelo de seguridad completo.

## Servidor MCP

srelens incluye un servidor MCP generado a partir del mismo registro de
capacidades que el backend, así que los clientes compatibles con MCP obtienen las
mismas operaciones sobre el cluster sin una capa de integración aparte.

En la aplicación de escritorio, abre **Settings → MCP** (Ajustes → MCP) para:

- ejecutar el servidor por HTTP en loopback, protegido con un bearer token que
  puedes mostrar, rotar o revocar: al rotarlo se reinicia el servidor en marcha
  para que el nuevo token se aplique al instante, y al revocarlo se detiene;
- instalar la CLI `srelens` para conexiones stdio, que no necesitan token porque
  el cliente, al lanzar el proceso, ya tiene tus privilegios;
- copiar la configuración de cliente para los clientes MCP compatibles.

O arráncalo directamente, desde el binario de escritorio o desde la interfaz de
terminal:

```sh
srelens --mcp-stdio
srelens --mcp-http 127.0.0.1:8765
srectl mcp
```

Ejemplo de configuración stdio:

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

Las herramientas destructivas piden confirmación en la app. Las ejecuciones
headless no tienen ningún diálogo que mostrar, así que necesitan
`"_confirm": true` en la llamada *y* una activación explícita a nivel de proceso:
`--mcp-allow-destructive` para modificar cualquier cosa, o
`--mcp-allow-sensitive-reads` para leer Secrets (`--allow-destructive` y
`--allow-sensitive-reads` en `srectl mcp`). Las dos son independientes, así que
poder leer un Secret nunca implica permiso para hacer drain de un nodo. Consulta
[docs/MCP.md](../MCP.md) para el modelo de seguridad completo, incluida la
comprobación de la cabecera Host, el log de auditoría y el almacenamiento de
tokens.

> El acceso por MCP usa tus contextos de cluster autenticados localmente. Revisa
> las llamadas a herramientas y aplica permisos de RBAC de Kubernetes adecuados,
> sobre todo con clusters críticos.

## Compilar desde el código fuente

### Requisitos previos

- [Rust](https://rustup.rs) estable
- [Node.js](https://nodejs.org) 26 (la versión por defecto para desarrollo; Node 24 LTS también es compatible). Ejecuta `nvm install && nvm use` para usar la versión de `.nvmrc`.
- [pnpm](https://pnpm.io) 9+
- [Dependencias del sistema de Tauri v2](https://v2.tauri.app/start/prerequisites/)
- Un cluster de Kubernetes accesible para los flujos de trabajo que dependen de un cluster

### Ejecutar en local

```sh
git clone https://github.com/srelens/srelens
cd srelens
pnpm install
pnpm dev                  # the desktop app
cargo run -p srectl       # the terminal UI
```

### Comandos útiles

| Comando | Para qué sirve |
| --- | --- |
| `pnpm dev` | Lanza la aplicación de escritorio en modo desarrollo |
| `cargo run -p srectl` | Ejecuta la interfaz de terminal |
| `pnpm test` | Ejecuta los tests de JavaScript y TypeScript |
| `cargo test` | Ejecuta los tests del workspace de Rust |
| `pnpm build` | Compila el frontend de producción |
| `pnpm tauri build` | Genera los binarios empaquetados de escritorio |

Consulta la [guía para desarrolladores](../DEVELOPMENT.md) para conocer la
arquitectura, los estándares de testing y cómo añadir una capacidad.

## Arquitectura

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

Estructura del repositorio:

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

## Estado del proyecto

srelens es **estable y está en desarrollo activo**, con versiones periódicas para
macOS, Linux y Windows. Los builds de macOS están firmados con Developer ID y
notarizados, y los paquetes compatibles se actualizan in situ. Las nuevas
capacidades llegan de forma continua, así que vale la pena echar un vistazo a las
notas de la versión cuando actualices.

- [Última versión](https://github.com/srelens/srelens/releases/latest)
- [Todas las versiones](https://github.com/srelens/srelens/releases)
- [Issues y hoja de ruta](https://github.com/srelens/srelens/issues)

## Comunidad

- [r/srelens en Reddit](https://www.reddit.com/r/srelens/) — anuncios, preguntas
  y feedback
- [Issues](https://github.com/srelens/srelens/issues) — bugs y peticiones de
  funcionalidades
- [Sitio web](https://srelens.com)

## Contribuir

Las contribuciones son bienvenidas. Empieza por la
[guía de contribución](../../CONTRIBUTING.md), la
[guía para desarrolladores](../DEVELOPMENT.md), la
[guía de integración de agentes MCP](../MCP.md) y los
[issues abiertos](https://github.com/srelens/srelens/issues). Antes de participar,
lee el [Código de conducta](../../.github/CODE_OF_CONDUCT.md).

Las traducciones están en [`docs/i18n`](.). El README en inglés es la fuente de
verdad; las correcciones a una traducción, o un idioma nuevo, son bienvenidas como
pull request.

## Licencia

srelens es de código abierto bajo la [licencia MIT](../../LICENSE).

---

<p align="center">
  <sub>Sin afiliación con Mirantis Lens ni con el proyecto Freelens.</sub>
</p>
