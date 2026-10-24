//! What confines a local terminal to the one cluster it was opened for
//! (srelens/srelens#846).
//!
//! A local shell is the reader's own process, so nothing here is a sandbox and
//! none of it claims to be. The aim is narrower and worth having: a command
//! typed in a terminal opened from one cluster's tab does not reach another
//! cluster by accident, and when the reader tries to point it elsewhere they
//! are told which cluster the terminal is bound to instead.
//!
//! Each session gets a private directory holding:
//!
//! - `kubeconfig` — only the session's context, with its cluster and user. This
//!   is what `kubectl config get-contexts` lists, and it was the whole of the
//!   lock before this module existed.
//! - `bin/kubectl`, `bin/helm` — small guards placed ahead of the real tools on
//!   `PATH`. They refuse the flags and `config` verbs that would aim the tool at
//!   another cluster, pin `KUBECONFIG` to the file above whatever the
//!   environment says by then, and hand over to the real binary. In a
//!   read-only terminal, the one a protected cluster gets (#101), they also
//!   refuse every subcommand that is not a read.
//! - `init.sh` and the rc files that source it — for bash and zsh, run after
//!   the reader's own rc files: put `bin` back at the front of `PATH` (rc files
//!   commonly prepend to it), make `KUBECONFIG` read-only, and tag the prompt
//!   with the cluster and namespace, and `read-only` when the terminal is.
//!
//! What is left outside, by design: a tool that is neither `kubectl` nor `helm`
//! and is told about another kubeconfig by its own flag; a shell that is
//! neither bash nor zsh, which gets the kubeconfig and the guards but no
//! read-only variable and no prompt tag; and a reader who removes the guards
//! from `PATH` on purpose. `docs/USAGE.md` says the same to the reader.

use std::io::Write;
use std::path::{Path, PathBuf};

/// A session's private directory and what the shell needs to be started in it.
pub(crate) struct Scope {
    /// Removed, with everything in it, when the session ends.
    pub dir: PathBuf,
    pub kubeconfig: PathBuf,
    /// The guards' directory, to go first on `PATH`. `None` where there are no
    /// guards (a platform without `/bin/sh`).
    pub bin: Option<PathBuf>,
}

/// How to start `shell` so that it reads its own rc files and then ours.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ShellStart {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Scope {
    /// Create the directory for one session.
    ///
    /// `create_dir` rather than `create_dir_all`, and mode 0700 at creation: a
    /// path that already exists is refused rather than followed, and there is
    /// no window in which another user can read the credentials inside.
    ///
    /// `read_only` makes the guards refuse every `kubectl` and `helm`
    /// subcommand that is not a read, and tags the prompt `read-only` (#101).
    /// Where there are no guards it changes nothing.
    pub fn create(
        id: u64,
        context: &str,
        namespace: Option<&str>,
        kubeconfig_paths: &[PathBuf],
        read_only: bool,
    ) -> Result<Scope, String> {
        let yaml = srelens_kube::connect::single_context_kubeconfig_yaml_in(
            kubeconfig_paths,
            context,
            namespace,
        )?;
        let dir = std::env::temp_dir().join(format!("srelens-term-{}-{}", std::process::id(), id));
        make_private_dir(&dir).map_err(|e| e.to_string())?;
        let scope = Scope {
            kubeconfig: dir.join("kubeconfig"),
            bin: cfg!(unix).then(|| dir.join("bin")),
            dir,
        };
        let written = scope.write(context, &yaml, read_only);
        if written.is_err() {
            scope.remove();
        }
        written.map(|()| scope)
    }

    fn write(&self, context: &str, yaml: &str, read_only: bool) -> Result<(), String> {
        write_private(&self.kubeconfig, yaml, false)?;
        let Some(bin) = &self.bin else { return Ok(()) };
        make_private_dir(bin).map_err(|e| e.to_string())?;
        for tool in [Tool::Kubectl, Tool::Helm] {
            let script = guard_script(tool, context, bin, &self.kubeconfig, read_only);
            write_private(&bin.join(tool.name()), &script, true)?;
        }
        write_private(
            &self.dir.join("init.sh"),
            &init_script(context, bin, &self.kubeconfig, read_only),
            false,
        )?;
        write_private(&self.dir.join("bashrc"), &bashrc(&self.dir), false)?;
        let zsh = self.dir.join("zsh");
        make_private_dir(&zsh).map_err(|e| e.to_string())?;
        write_private(&zsh.join(".zshenv"), &zshenv(&zsh), false)?;
        write_private(&zsh.join(".zshrc"), &zshrc(&self.dir), false)
    }

    /// The arguments and environment that make `shell` read our init file after
    /// its own. Empty for a shell this module has no init for.
    pub fn shell_start(&self, shell: &str) -> ShellStart {
        if self.bin.is_none() {
            return ShellStart::default();
        }
        let name = Path::new(shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        match name {
            "bash" => ShellStart {
                args: vec![
                    "--rcfile".into(),
                    self.dir.join("bashrc").to_string_lossy().into_owned(),
                ],
                env: Vec::new(),
            },
            "zsh" => {
                let mut env = vec![(
                    "ZDOTDIR".to_string(),
                    self.dir.join("zsh").to_string_lossy().into_owned(),
                )];
                // The reader's own, so our `.zshenv` can read their files from
                // where they keep them.
                if let Ok(theirs) = std::env::var("ZDOTDIR") {
                    env.push(("SRELENS_USER_ZDOTDIR".to_string(), theirs));
                }
                ShellStart { args: Vec::new(), env }
            }
            _ => ShellStart::default(),
        }
    }

    /// `PATH` with the guards first.
    pub fn path_with_guards(&self, path: Option<std::ffi::OsString>) -> Option<std::ffi::OsString> {
        let bin = self.bin.as_ref()?;
        let mut entries = vec![bin.clone()];
        if let Some(path) = &path {
            entries.extend(std::env::split_paths(path).filter(|p| p != bin));
        }
        std::env::join_paths(entries).ok()
    }

    pub fn remove(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The line a terminal opens with, so the reader is told once, in words, what
/// the terminal is bound to. Written by the host rather than printed by an rc
/// file: it is then the same in every shell, and prompt themes that object to
/// output during shell start-up have nothing to object to.
pub(crate) fn banner(context: &str, namespace: Option<&str>) -> String {
    let namespace = match namespace {
        Some(ns) if !ns.is_empty() => format!(", namespace {}", printable(ns)),
        _ => String::new(),
    };
    format!(
        "\x1b[2msrelens: this terminal is bound to {}{namespace}. kubectl and helm here cannot be pointed at another cluster.\x1b[0m\r\n",
        printable(context)
    )
}

/// `text` with control characters dropped, for writing to a terminal: a context
/// name is the reader's kubeconfig's to choose and may hold an escape sequence.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

#[derive(Clone, Copy)]
enum Tool {
    Kubectl,
    Helm,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Kubectl => "kubectl",
            Tool::Helm => "helm",
        }
    }

    /// The flags that aim the tool at a cluster, as a `case` pattern. Each
    /// long flag is listed bare and with `=`, the two ways its value is
    /// attached. kubectl's `-s` is matched as a prefix: a short flag takes its
    /// value attached with nothing between (`-shttps://…`), and kubectl has no
    /// other short flag that begins with `s`.
    fn refused_flags(self) -> &'static str {
        match self {
            Tool::Kubectl => {
                "--kubeconfig|--kubeconfig=*|--context|--context=*|--cluster|--cluster=*|--server|--server=*|-s*"
            }
            Tool::Helm => {
                "--kubeconfig|--kubeconfig=*|--kube-context|--kube-context=*|--kube-apiserver|--kube-apiserver=*"
            }
        }
    }

    /// `kubectl config` verbs that would switch the context or rewrite where
    /// the one cluster in the file points. `set-context` is left alone: it is
    /// how a namespace is changed, and the file holds one cluster for it to
    /// name.
    fn refused_config_verbs(self) -> Option<&'static str> {
        match self {
            Tool::Kubectl => Some("use-context|use|set-cluster|set-credentials|set"),
            Tool::Helm => None,
        }
    }

    /// The `kubectl config` verbs that are left alone. Named so the verb can be
    /// told from a flag's value standing before it (`config -n default set …`):
    /// the verb is the first word after `config` that is a verb at all.
    const ALLOWED_CONFIG_VERBS: &'static str = "current-context|delete-cluster|delete-context|delete-user|get-clusters|get-contexts|get-users|rename-context|set-context|unset|view";

    /// kubectl's global flags that take their value as the next word. That
    /// word is a value, never the verb: `config -n set view` is `view` in a
    /// namespace called `set`, and must not be read as `config set`.
    const VALUE_FLAGS: &'static str = "-n|--namespace|--as|--as-group|--as-uid|--user|--token|--username|--password|--request-timeout|--cache-dir|--certificate-authority|--client-certificate|--client-key|--tls-server-name|-v|--v|--vmodule|--profile|--profile-output|--log-flush-frequency";

    /// Environment that aims the tool at a cluster without a flag, cleared
    /// before the real tool runs. helm reads its target from these as readily
    /// as from `--kube-context` and `--kube-apiserver`, and an rc file or the
    /// app's own environment may have set them for another cluster.
    fn cleared_env(self) -> &'static str {
        match self {
            Tool::Kubectl => "",
            Tool::Helm => "unset HELM_KUBECONTEXT HELM_KUBEAPISERVER\n",
        }
    }

    /// The global flags that take no value (#101). To cobra, which both tools
    /// are built on, every other flag word with no `=` — `--name`, or `-x` —
    /// takes the next word as its value, a flag it has never heard of
    /// included: in `kubectl -l get delete pods` the subcommand is `delete`.
    /// The read-only check reads words the same way, so the subcommand it
    /// checks is the one the tool runs.
    ///
    /// kubectl's list includes the klog booleans that 1.25 and earlier
    /// registered (`--logtostderr` and the rest). Newer kubectl rejects them
    /// as unknown before it runs anything, so listing them is safe.
    ///
    /// ponytail: a boolean flag missing from this list hides the subcommand
    /// from the check, because the guard reads the flag as taking the next
    /// word as its value; with no subcommand found nothing is refused, and a
    /// write can run (`kubectl --logtostderr apply -f x.yaml` did, before
    /// those were listed). List each boolean global flag when kubectl or
    /// helm adds one.
    fn bool_flags(self) -> &'static str {
        match self {
            Tool::Kubectl => {
                "--add-dir-header|--alsologtostderr|--disable-compression|--insecure-skip-tls-verify|--logtostderr|--match-server-version|--one-output|--skip-headers|--skip-log-headers|--warnings-as-errors"
            }
            Tool::Helm => "--debug|--kube-insecure-skip-tls-verify",
        }
    }

    /// The read-only terminal's check on each word that is neither a flag nor
    /// a flag's value (#101). The first such word is the subcommand, and it
    /// must be a read: anything nobody listed, a plugin's name included, is
    /// refused. kubectl's `rollout` and `auth` pass only for their reads,
    /// because `rollout restart` and `auth reconcile` write. cobra's hidden
    /// `__complete` and `__completeNoDesc` pass too: the scripts that
    /// `completion` prints call them on every Tab, and they run the
    /// completion functions, never the command being completed.
    fn read_only_check(self) -> &'static str {
        match self {
            Tool::Kubectl => r#"  case "$verb" in
    '')
      case "$arg" in
        get|describe|logs|top|explain|api-resources|api-versions|version|cluster-info|events|diff|wait|completion|__complete|__completeNoDesc|help|config) verb=read ;;
        rollout|auth) verb=$arg ;;
        *) refuse_write "$arg" ;;
      esac
      ;;
    rollout)
      case "$arg" in
        status|history) verb=read ;;
        *) refuse_write "rollout $arg" ;;
      esac
      ;;
    auth)
      case "$arg" in
        can-i|whoami) verb=read ;;
        *) refuse_write "auth $arg" ;;
      esac
      ;;
  esac
"#,
            Tool::Helm => r#"  if [ -z "$verb" ]; then
    case "$arg" in
      list|ls|status|get|history|show|inspect|search|template|lint|version|env|help|completion|__complete|__completeNoDesc|repo|dependency|dep) verb=read ;;
      *) refuse_write "$arg" ;;
    esac
  fi
"#,
        }
    }
}

/// `text` as one single-quoted POSIX shell word.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn sh_path(path: &Path) -> String {
    sh_quote(&path.to_string_lossy())
}

/// The guard for one tool. POSIX `sh`, with the session's paths written into
/// it, so it depends on nothing in an environment the reader may have changed.
///
/// Arguments after `--` are not read: they belong to whatever the tool runs
/// (`kubectl exec pod -- curl --server …`), not to the tool.
///
/// `read_only` adds the read-only terminal's check (#101; see
/// [`Tool::read_only_check`]), with flags and their values told apart as
/// [`Tool::bool_flags`] says. Read-only, a `--` that a flag is still waiting
/// on is that flag's value, as it is to cobra: `kubectl -n -- delete pods -A`
/// is a delete.
fn guard_script(tool: Tool, context: &str, bin: &Path, kubeconfig: &Path, read_only: bool) -> String {
    let name = tool.name();
    // How a flag's value is told from the word after it. Read-only, cobra's
    // way; otherwise kubectl's named list, which only `config` needs.
    let value_flags = if read_only {
        format!(
            "    {}|--*=*) continue ;;\n    --?*|-?) value=next; continue ;;\n    -*) continue ;;\n",
            tool.bool_flags()
        )
    } else if tool.refused_config_verbs().is_some() {
        format!("    {}) value=next; continue ;;\n", Tool::VALUE_FLAGS)
    } else {
        String::new()
    };
    let value_skip = if value_flags.is_empty() {
        String::new()
    } else {
        format!(
            r#"  if [ -n "$value" ]; then
    value=
    continue
  fi
  case "$arg" in
{value_flags}  esac
"#
        )
    };
    let config_check = match tool.refused_config_verbs() {
        Some(verbs) => format!(
            r#"  if [ "$config" = verb ]; then
    case "$arg" in
      {verbs}) refuse "config $arg" ;;
      {allowed}) config=done ;;
    esac
  fi
  [ "$arg" = config ] && [ -z "$config" ] && config=verb
"#,
            allowed = Tool::ALLOWED_CONFIG_VERBS,
        ),
        None => String::new(),
    };
    let stop = if read_only {
        r#"  [ "$arg" = -- ] && [ -z "$value" ] && break"#
    } else {
        r#"  [ "$arg" = -- ] && break"#
    };
    // `verb=` too: a `verb` exported by the reader's environment is not a
    // subcommand already found.
    let (read_only_preamble, read_only_check) = if read_only {
        (
            format!(
                r#"refuse_write() {{
  printf 'srelens: "%s" is not available here: %s is protected and this terminal is read-only.\n' "{name} $1" "$bound" >&2
  exit 64
}}
verb="#
            ),
            tool.read_only_check(),
        )
    } else {
        (String::new(), "")
    };
    format!(
        r#"#!/bin/sh
# Written by srelens for one terminal session. Keeps `{name}` in this terminal
# on the cluster the terminal was opened for, then runs the real `{name}`.
bound={bound}
bin={bin}
KUBECONFIG={kubeconfig}
export KUBECONFIG
{cleared_env}
refuse() {{
  printf 'srelens: "%s" is not available here. This terminal is bound to %s; open a terminal from the other cluster to work there.\n' "$1" "$bound" >&2
  exit 64
}}
{read_only_preamble}
config=
value=
for arg in "$@"; do
{stop}
  case "$arg" in
    {flags}) refuse "$arg" ;;
  esac
{value_skip}{config_check}{read_only_check}done

IFS=:
for dir in $PATH; do
  [ "$dir" = "$bin" ] && continue
  if [ -x "$dir/{name}" ] && [ ! -d "$dir/{name}" ]; then
    exec "$dir/{name}" "$@"
  fi
done
printf 'srelens: {name} was not found on PATH\n' >&2
exit 127
"#,
        bound = sh_quote(&printable(context)),
        bin = sh_path(bin),
        kubeconfig = sh_path(kubeconfig),
        flags = tool.refused_flags(),
        cleared_env = tool.cleared_env(),
    )
}

/// The context as it appears in the prompt: anything a prompt would expand
/// (`\`, `$`, a backtick, zsh's `%`) or a terminal would act on is replaced,
/// so the tag is inert in both shells without per-shell escaping.
fn prompt_safe(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._:/@-".contains(c) { c } else { '_' })
        .collect()
}

/// Sourced by bash and zsh after the reader's own rc files. Written to parse in
/// both; each shell runs only its own branch. `read_only` adds `read-only` to
/// the prompt tag (#101).
fn init_script(context: &str, bin: &Path, kubeconfig: &Path, read_only: bool) -> String {
    format!(
        r#"# Written by srelens for one terminal session. See the guards beside this file.
# rc files commonly prepend to PATH; the guards go back in front of them.
PATH={bin}:$PATH
export PATH
KUBECONFIG={kubeconfig}
export KUBECONFIG
readonly KUBECONFIG 2>/dev/null

__srelens_context={context}
__srelens_last=
__srelens_tag() {{
  __srelens_ns=$(sed -n 's/^[[:space:]]*namespace:[[:space:]]*//p' "$KUBECONFIG" 2>/dev/null | head -n 1 | tr -c 'A-Za-z0-9._\n-' '_')
  printf '[%s%s{mode}] ' "$__srelens_context" "${{__srelens_ns:+/$__srelens_ns}}"
}}
if [ -n "${{BASH_VERSION-}}" ]; then
  __srelens_prompt() {{
    local tag
    tag=$(__srelens_tag)
    case "$PS1" in
      "$tag"*) ;;
      *) PS1="$tag${{PS1#"$__srelens_last"}}" ;;
    esac
    __srelens_last=$tag
  }}
  PROMPT_COMMAND="${{PROMPT_COMMAND:+$PROMPT_COMMAND
}}__srelens_prompt"
elif [ -n "${{ZSH_VERSION-}}" ]; then
  __srelens_prompt() {{
    local tag
    tag=$(__srelens_tag)
    case "$PROMPT" in
      "$tag"*) ;;
      *) PROMPT="$tag${{PROMPT#"$__srelens_last"}}" ;;
    esac
    __srelens_last=$tag
  }}
  precmd_functions+=(__srelens_prompt)
fi
"#,
        bin = sh_path(bin),
        kubeconfig = sh_path(kubeconfig),
        context = sh_quote(&prompt_safe(context)),
        mode = if read_only { " read-only" } else { "" },
    )
}

/// bash's `--rcfile` replaces `~/.bashrc`, so it is read from here first.
fn bashrc(dir: &Path) -> String {
    format!(
        "[ -f \"$HOME/.bashrc\" ] && . \"$HOME/.bashrc\"\n. {}\n",
        sh_path(&dir.join("init.sh"))
    )
}

/// zsh reads `$ZDOTDIR/.zshenv` then `$ZDOTDIR/.zshrc`. The reader's own are
/// read from where they keep them — a `.zshenv` that sets `ZDOTDIR` is a common
/// way to move the rest — and `ZDOTDIR` is pointed back here only for as long
/// as it takes zsh to find our `.zshrc`.
fn zshenv(zsh: &Path) -> String {
    format!(
        r#"if [ -n "${{SRELENS_USER_ZDOTDIR+x}}" ]; then ZDOTDIR=$SRELENS_USER_ZDOTDIR; else unset ZDOTDIR; fi
[ -f "${{ZDOTDIR:-$HOME}}/.zshenv" ] && . "${{ZDOTDIR:-$HOME}}/.zshenv"
if [ -n "${{ZDOTDIR+x}}" ]; then SRELENS_USER_ZDOTDIR=$ZDOTDIR; else unset SRELENS_USER_ZDOTDIR; fi
ZDOTDIR={}
"#,
        sh_path(zsh)
    )
}

fn zshrc(dir: &Path) -> String {
    format!(
        r#"if [ -n "${{SRELENS_USER_ZDOTDIR+x}}" ]; then ZDOTDIR=$SRELENS_USER_ZDOTDIR; else unset ZDOTDIR; fi
unset SRELENS_USER_ZDOTDIR
[ -f "${{ZDOTDIR:-$HOME}}/.zshrc" ] && . "${{ZDOTDIR:-$HOME}}/.zshrc"
. {}
"#,
        sh_path(&dir.join("init.sh"))
    )
}

pub(crate) fn make_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Created with `O_EXCL` and its final mode, so a path that already exists is
/// never followed or overwritten and the file is never briefly world-readable.
fn write_private(path: &Path, content: &str, executable: bool) -> Result<(), String> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(if executable { 0o700 } else { 0o600 });
    }
    #[cfg(not(unix))]
    let _ = executable;
    let mut file = opts.open(path).map_err(|e| e.to_string())?;
    file.write_all(content.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Output};

    const KUBECONFIG: &str = r#"apiVersion: v1
kind: Config
current-context: dev-cluster
clusters:
- name: dev
  cluster:
    server: https://127.0.0.1:1
- name: other
  cluster:
    server: https://127.0.0.1:2
users:
- name: dev-user
  user: {}
contexts:
- name: dev-cluster
  context:
    cluster: dev
    user: dev-user
- name: other-cluster
  context:
    cluster: other
    user: dev-user
"#;

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_000_000);

    struct Fixture {
        _dir: tempfile::TempDir,
        /// Holds the stand-in "real" `kubectl` and `helm`.
        real: PathBuf,
        scope: Scope,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.scope.remove();
        }
    }

    /// A scope for `context`, and a directory of stand-in tools that print what
    /// they were run with — so a test can tell a refusal from a hand-over.
    fn fixture(context: &str, namespace: Option<&str>) -> Fixture {
        fixture_in_mode(context, namespace, false)
    }

    /// [`fixture`], for a terminal opened read-only or not (#101).
    fn fixture_in_mode(context: &str, namespace: Option<&str>, read_only: bool) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, KUBECONFIG.replace("dev-cluster", context)).unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        for tool in ["kubectl", "helm"] {
            let path = real.join(tool);
            std::fs::write(
                &path,
                format!("#!/bin/sh\nprintf 'real-{tool} KUBECONFIG=%s args=%s\\n' \"$KUBECONFIG\" \"$*\"\n"),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let scope = Scope::create(id, context, namespace, &[config], read_only).unwrap();
        Fixture { _dir: dir, real, scope }
    }

    impl Fixture {
        fn path(&self) -> std::ffi::OsString {
            let system = std::env::var_os("PATH").unwrap_or_default();
            let mut entries = vec![self.real.clone()];
            entries.extend(std::env::split_paths(&system));
            self.scope
                .path_with_guards(Some(std::env::join_paths(entries).unwrap()))
                .unwrap()
        }

        fn run(&self, tool: &str, args: &[&str]) -> Output {
            Command::new(tool)
                .args(args)
                .env("PATH", self.path())
                // What the reader's environment might say by then; the guard
                // must not take its word for it.
                .env("KUBECONFIG", "/somewhere/else")
                .output()
                .unwrap()
        }
    }

    fn stdout(out: &Output) -> String {
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn stderr(out: &Output) -> String {
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    #[test]
    fn the_kubeconfig_holds_only_the_session_context() {
        let f = fixture("dev-cluster", None);
        let yaml = std::fs::read_to_string(&f.scope.kubeconfig).unwrap();
        assert!(yaml.contains("dev-cluster"));
        assert!(!yaml.contains("other-cluster"));
    }

    #[test]
    fn the_directory_and_its_files_are_the_owners_alone() {
        let f = fixture("dev-cluster", None);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&f.scope.dir), 0o700);
        assert_eq!(mode(&f.scope.kubeconfig), 0o600);
        assert_eq!(mode(&f.scope.bin.as_ref().unwrap().join("kubectl")), 0o700);
    }

    #[test]
    fn an_ordinary_command_reaches_the_real_tool_with_the_session_kubeconfig() {
        let f = fixture("dev-cluster", None);
        let out = f.run("kubectl", &["get", "pods", "-n", "kube-system"]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert_eq!(
            stdout(&out).trim(),
            format!(
                "real-kubectl KUBECONFIG={} args=get pods -n kube-system",
                f.scope.kubeconfig.display()
            )
        );
    }

    #[test]
    fn kubectl_refuses_each_way_of_naming_another_cluster() {
        let f = fixture("dev-cluster", None);
        for args in [
            vec!["--context", "other-cluster", "get", "pods"],
            vec!["get", "pods", "--context=other-cluster"],
            vec!["--kubeconfig", "/home/me/.kube/config", "get", "pods"],
            vec!["--kubeconfig=/home/me/.kube/config", "get", "pods"],
            vec!["get", "pods", "--cluster", "other"],
            vec!["get", "pods", "--server=https://elsewhere:6443"],
            vec!["get", "pods", "-s", "https://elsewhere:6443"],
            // A short flag's value attached with nothing between.
            vec!["get", "pods", "-shttps://elsewhere:6443"],
            vec!["get", "pods", "-s=https://elsewhere:6443"],
            // A flag, or a flag and its value, between `config` and the verb.
            vec!["config", "--namespace=default", "set", "clusters.dev.server", "https://elsewhere:6443"],
            vec!["config", "-n", "default", "set-cluster", "dev", "--server=https://elsewhere:6443"],
            vec!["-n", "default", "config", "use-context", "other-cluster"],
            // A flag's value that happens to be a harmless verb does not end
            // the search for the real one.
            vec!["config", "-n", "view", "set-cluster", "dev", "--server=https://elsewhere:6443"],
            vec!["config", "-v=1", "set", "clusters.dev.server", "https://elsewhere:6443"],
            // A refused flag is refused wherever it stands, a value's place included.
            vec!["get", "pods", "-n", "--context=other-cluster"],
            vec!["config", "use-context", "other-cluster"],
            vec!["config", "set-cluster", "dev", "--server=https://elsewhere:6443"],
            vec!["config", "set-credentials", "dev-user", "--token=x"],
            vec!["config", "set", "clusters.dev.server", "https://elsewhere:6443"],
        ] {
            let out = f.run("kubectl", &args);
            assert_eq!(out.status.code(), Some(64), "{args:?} was not refused");
            assert_eq!(stdout(&out), "", "{args:?} reached the real kubectl");
            let said = stderr(&out);
            assert!(said.contains("bound to dev-cluster"), "{args:?}: {said}");
        }
    }

    #[test]
    fn kubectl_still_does_what_stays_on_the_cluster() {
        let f = fixture("dev-cluster", None);
        for args in [
            vec!["config", "get-contexts"],
            vec!["config", "current-context"],
            // How a namespace is changed; the file holds one cluster to name.
            vec!["config", "set-context", "--current", "--namespace=payments"],
            // After `--` the words are the remote command's, not kubectl's.
            vec!["exec", "web-0", "--", "curl", "--server", "x", "-s", "http://svc"],
            // A value that merely looks like a refused verb.
            vec!["get", "configmap", "set"],
            // Once the verb is found, later words are its arguments: a
            // namespace called `set` is a namespace.
            vec!["config", "set-context", "--current", "--namespace", "set"],
            vec!["config", "-n", "default", "view", "--minify"],
            // A flag's value is a value: these are namespaces called `set` and
            // `config`, not the verb or the command.
            vec!["config", "-n", "set", "view", "--minify"],
            vec!["config", "--namespace", "use-context", "get-contexts"],
            vec!["get", "pods", "-n", "config", "set"],
            // Short flags that are not `-s`.
            vec!["get", "pods", "-n", "kube-system", "-o", "wide", "-A"],
        ] {
            let out = f.run("kubectl", &args);
            assert!(out.status.success(), "{args:?}: {}", stderr(&out));
            assert!(stdout(&out).starts_with("real-kubectl "), "{args:?}");
        }
    }

    #[test]
    fn helm_refuses_its_own_cluster_flags_and_passes_the_rest() {
        let f = fixture("dev-cluster", None);
        for args in [
            vec!["list", "--kube-context", "other-cluster"],
            vec!["list", "--kube-context=other-cluster"],
            vec!["list", "--kubeconfig", "/home/me/.kube/config"],
            vec!["list", "--kube-apiserver=https://elsewhere:6443"],
        ] {
            let out = f.run("helm", &args);
            assert_eq!(out.status.code(), Some(64), "{args:?} was not refused");
            assert!(stderr(&out).contains("bound to dev-cluster"));
        }
        // `-s` is helm's `--show-only`, not a server.
        let out = f.run("helm", &["template", "chart", "-s", "templates/a.yaml"]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(stdout(&out).contains(&format!("KUBECONFIG={}", f.scope.kubeconfig.display())));
    }

    #[test]
    fn helm_is_not_aimed_elsewhere_by_its_environment() {
        let f = fixture("dev-cluster", None);
        // A stand-in that reports what it was left with.
        std::fs::write(
            f.real.join("helm"),
            "#!/bin/sh\nprintf 'context=%s server=%s\\n' \"${HELM_KUBECONTEXT-unset}\" \"${HELM_KUBEAPISERVER-unset}\"\n",
        )
        .unwrap();
        let out = Command::new("helm")
            .arg("list")
            .env("PATH", f.path())
            .env("HELM_KUBECONTEXT", "other-cluster")
            .env("HELM_KUBEAPISERVER", "https://elsewhere:6443")
            .output()
            .unwrap();
        assert_eq!(stdout(&out).trim(), "context=unset server=unset", "{}", stderr(&out));
    }

    #[test]
    fn a_missing_tool_is_reported_rather_than_run_in_a_loop() {
        let f = fixture("dev-cluster", None);
        std::fs::remove_file(f.real.join("helm")).unwrap();
        let out = Command::new(f.scope.bin.as_ref().unwrap().join("helm"))
            .arg("list")
            // Only the guards and the stand-ins: no system helm to find.
            .env(
                "PATH",
                std::env::join_paths([f.scope.bin.clone().unwrap(), f.real.clone()]).unwrap(),
            )
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(127));
        assert!(stderr(&out).contains("helm was not found on PATH"));
    }

    #[test]
    fn a_context_name_with_shell_syntax_in_it_is_only_ever_text() {
        let name = "it's $(touch /tmp/srelens-846-pwned) `x`";
        let f = fixture(name, None);
        let out = f.run("kubectl", &["--context", "x"]);
        assert_eq!(out.status.code(), Some(64));
        assert!(stderr(&out).contains(name));
        assert!(!Path::new("/tmp/srelens-846-pwned").exists());
    }

    /// Run `script` in an interactive `shell` started the way the terminal
    /// starts it, with `home` as the reader's home.
    fn interactive(f: &Fixture, shell: &str, home: &Path, script: &str) -> Output {
        let start = f.scope.shell_start(shell);
        Command::new(shell)
            .args(&start.args)
            .args(["-i", "-c", script])
            .envs(start.env.iter().cloned())
            .env("HOME", home)
            .env("PATH", f.path())
            .env("KUBECONFIG", &f.scope.kubeconfig)
            .env_remove("ZDOTDIR")
            .env_remove("PROMPT_COMMAND")
            .output()
            .unwrap()
    }

    #[test]
    fn bash_reads_the_readers_rc_then_puts_the_guards_first_and_locks_the_variable() {
        let f = fixture("dev-cluster", Some("payments"));
        let home = tempfile::tempdir().unwrap();
        // An rc file that does what rc files do: puts something ahead on PATH.
        std::fs::write(
            home.path().join(".bashrc"),
            format!("PATH={}:$PATH\nSRELENS_TEST_RC=read\n", f.real.display()),
        )
        .unwrap();
        let out = interactive(
            &f,
            "bash",
            home.path(),
            "echo rc=$SRELENS_TEST_RC; kubectl --context other get pods; echo refused=$?; \
             export KUBECONFIG=/elsewhere; echo kc=$KUBECONFIG; \
             __srelens_prompt; echo \"ps1=$PS1\"",
        );
        let said = stdout(&out);
        assert!(said.contains("rc=read"), "{said}\n{}", stderr(&out));
        assert!(said.contains("refused=64"), "{said}\n{}", stderr(&out));
        assert!(
            said.contains(&format!("kc={}", f.scope.kubeconfig.display())),
            "{said}\n{}",
            stderr(&out)
        );
        assert!(said.contains("ps1=[dev-cluster/payments] "), "{said}");
    }

    #[test]
    fn the_prompt_tag_follows_a_namespace_change_without_stacking() {
        let f = fixture("dev-cluster", Some("payments"));
        let home = tempfile::tempdir().unwrap();
        let out = interactive(
            &f,
            "bash",
            home.path(),
            "PS1='$ '; __srelens_prompt; __srelens_prompt; echo \"a=$PS1\"; \
             sed -i 's/namespace: payments/namespace: billing/' \"$KUBECONFIG\"; \
             __srelens_prompt; echo \"b=$PS1\"",
        );
        let said = stdout(&out);
        assert!(said.contains("a=[dev-cluster/payments] $ "), "{said}");
        assert!(said.contains("b=[dev-cluster/billing] $ "), "{said}");
    }

    /// zsh, where there is one: the macOS default, and absent from most Linux
    /// CI images. `SRELENS_TEST_ZSH` names a binary that is not on `PATH`.
    fn zsh() -> Option<String> {
        if let Ok(path) = std::env::var("SRELENS_TEST_ZSH") {
            return Some(path);
        }
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join("zsh"))
            .find(|path| path.is_file())
            .map(|path| path.to_string_lossy().into_owned())
    }

    #[test]
    fn zsh_reads_the_readers_files_then_puts_the_guards_first_and_locks_the_variable() {
        let Some(zsh) = zsh() else {
            eprintln!("zsh not found; skipped");
            return;
        };
        let f = fixture("dev-cluster", Some("payments"));
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".zshenv"), "SRELENS_TEST_ENV=read\n").unwrap();
        std::fs::write(
            home.path().join(".zshrc"),
            format!("PATH={}:$PATH\nSRELENS_TEST_RC=read\nPROMPT='%% '\n", f.real.display()),
        )
        .unwrap();
        // Started as the terminal starts it: by name, with our ZDOTDIR.
        let start = f.scope.shell_start("/bin/zsh");
        let out = Command::new(&zsh)
            .args(["-i", "-c"])
            .arg(
                "echo env=$SRELENS_TEST_ENV rc=$SRELENS_TEST_RC; \
                 kubectl --context other get pods; echo refused=$?; \
                 (KUBECONFIG=/elsewhere; echo reassigned) 2>/dev/null; echo kc=$KUBECONFIG; \
                 __srelens_prompt; echo \"prompt=$PROMPT\"; echo zdotdir=${ZDOTDIR-unset}",
            )
            .envs(start.env.iter().cloned())
            .env("HOME", home.path())
            .env("PATH", f.path())
            .env("KUBECONFIG", &f.scope.kubeconfig)
            .output()
            .unwrap();
        let said = stdout(&out);
        let context = format!("{said}\n{}", stderr(&out));
        assert!(said.contains("env=read rc=read"), "{context}");
        assert!(said.contains("refused=64"), "{context}");
        // zsh stops the list at an assignment to a read-only variable.
        assert!(!said.contains("reassigned"), "{context}");
        assert!(said.contains(&format!("kc={}", f.scope.kubeconfig.display())), "{context}");
        assert!(said.contains("prompt=[dev-cluster/payments] %% "), "{context}");
        // Ours for as long as it took to find our `.zshrc`, and no longer: a
        // shell started from this one reads the reader's files, not these.
        assert!(said.contains("zdotdir=unset"), "{context}");
    }

    #[test]
    fn a_shell_with_no_init_of_ours_is_started_plainly() {
        let f = fixture("dev-cluster", None);
        assert_eq!(f.scope.shell_start("/usr/bin/fish"), ShellStart::default());
        assert_eq!(
            f.scope.shell_start("/bin/bash").args,
            vec!["--rcfile".to_string(), f.scope.dir.join("bashrc").display().to_string()]
        );
        assert_eq!(f.scope.shell_start("/bin/zsh").env[0].0, "ZDOTDIR");
    }

    #[test]
    fn removing_the_scope_leaves_nothing_behind() {
        let f = fixture("dev-cluster", None);
        let dir = f.scope.dir.clone();
        assert!(dir.join("bin/kubectl").exists());
        f.scope.remove();
        assert!(!dir.exists());
    }

    #[test]
    fn the_banner_names_the_cluster_and_drops_control_characters() {
        let line = banner("dev\x1b[31m-cluster", Some("payments"));
        assert!(line.contains("bound to dev[31m-cluster, namespace payments."));
        assert!(banner("dev-cluster", None).contains("bound to dev-cluster. kubectl"));
        assert!(banner("dev-cluster", Some("")).contains("bound to dev-cluster. kubectl"));
    }

    /// The read-only refusal, word for word (#101).
    fn read_only_refusal(said: &str) -> String {
        format!(
            "srelens: \"{said}\" is not available here: dev-cluster is protected and this terminal is read-only.\n"
        )
    }

    #[test]
    fn a_read_only_terminal_passes_each_read_through() {
        let f = fixture_in_mode("dev-cluster", None, true);
        for (tool, args) in [
            ("kubectl", vec!["get", "pods"]),
            ("kubectl", vec!["describe", "deploy", "web"]),
            ("kubectl", vec!["logs", "web-0", "-f"]),
            ("kubectl", vec!["top", "pods"]),
            ("kubectl", vec!["explain", "pods.spec"]),
            ("kubectl", vec!["api-resources"]),
            ("kubectl", vec!["api-versions"]),
            ("kubectl", vec!["version"]),
            ("kubectl", vec!["cluster-info"]),
            ("kubectl", vec!["events"]),
            ("kubectl", vec!["diff", "-f", "web.yaml"]),
            ("kubectl", vec!["wait", "--for=condition=Ready", "pod/web-0"]),
            ("kubectl", vec!["completion", "bash"]),
            ("kubectl", vec!["help", "delete"]),
            ("kubectl", vec!["config", "get-contexts"]),
            ("kubectl", vec!["auth", "can-i", "delete", "pods"]),
            ("kubectl", vec!["auth", "whoami"]),
            // No subcommand at all: kubectl prints its help.
            ("kubectl", vec![]),
            // What the scripts `completion` prints call on every Tab: cobra's
            // hidden command, which runs the completion functions and never
            // the command being completed.
            ("kubectl", vec!["__complete", "get", "po", ""]),
            ("kubectl", vec!["__completeNoDesc", "delete", ""]),
            ("helm", vec!["__complete", "uninstall", ""]),
            ("helm", vec!["__completeNoDesc", "uninstall", ""]),
            ("helm", vec!["list", "-A"]),
            ("helm", vec!["ls"]),
            ("helm", vec!["status", "web"]),
            ("helm", vec!["get", "values", "web"]),
            ("helm", vec!["history", "web"]),
            ("helm", vec!["show", "values", "./chart"]),
            ("helm", vec!["inspect", "chart", "./chart"]),
            ("helm", vec!["search", "repo", "nginx"]),
            ("helm", vec!["template", "web", "./chart"]),
            ("helm", vec!["lint", "./chart"]),
            ("helm", vec!["version"]),
            ("helm", vec!["env"]),
            ("helm", vec!["help", "uninstall"]),
            ("helm", vec!["completion", "zsh"]),
            ("helm", vec!["repo", "update"]),
            ("helm", vec!["dependency", "build", "./chart"]),
            ("helm", vec!["dep", "update", "./chart"]),
        ] {
            let out = f.run(tool, &args);
            assert!(out.status.success(), "{tool} {args:?}: {}", stderr(&out));
            assert!(stdout(&out).starts_with(&format!("real-{tool} ")), "{tool} {args:?}");
        }
    }

    #[test]
    fn a_read_only_terminal_refuses_every_other_subcommand() {
        let f = fixture_in_mode("dev-cluster", None, true);
        for (tool, args, said) in [
            ("kubectl", vec!["delete", "pod", "web-0"], "kubectl delete"),
            ("kubectl", vec!["apply", "-f", "web.yaml"], "kubectl apply"),
            ("kubectl", vec!["edit", "deploy/web"], "kubectl edit"),
            ("kubectl", vec!["scale", "deploy/web", "--replicas=0"], "kubectl scale"),
            ("kubectl", vec!["exec", "web-0", "--", "sh"], "kubectl exec"),
            ("kubectl", vec!["cordon", "node-1"], "kubectl cordon"),
            // `auth` reads only through `can-i` and `whoami`; `reconcile` writes RBAC.
            ("kubectl", vec!["auth", "reconcile", "-f", "rbac.yaml"], "kubectl auth reconcile"),
            // A plugin, or anything else nobody listed, is not a read.
            ("kubectl", vec!["neat", "get", "pods"], "kubectl neat"),
            ("helm", vec!["install", "web", "./chart"], "helm install"),
            ("helm", vec!["upgrade", "web", "./chart"], "helm upgrade"),
            ("helm", vec!["rollback", "web", "1"], "helm rollback"),
            ("helm", vec!["uninstall", "web"], "helm uninstall"),
            ("helm", vec!["plugin", "install", "https://example.com/p"], "helm plugin"),
        ] {
            let out = f.run(tool, &args);
            assert_eq!(out.status.code(), Some(64), "{tool} {args:?} was not refused");
            assert_eq!(stdout(&out), "", "{tool} {args:?} reached the real {tool}");
            assert_eq!(stderr(&out), read_only_refusal(said), "{tool} {args:?}");
        }
        // A `verb` the reader's environment happens to export is not a
        // subcommand already found.
        let out = Command::new("kubectl")
            .args(["delete", "pod", "web-0"])
            .env("PATH", f.path())
            .env("verb", "read")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(64), "{}", stdout(&out));
    }

    #[test]
    fn a_read_only_rollout_may_be_watched_but_not_changed() {
        let f = fixture_in_mode("dev-cluster", None, true);
        for args in [vec!["rollout", "status", "deploy/web"], vec!["rollout", "history", "deploy/web"]] {
            let out = f.run("kubectl", &args);
            assert!(out.status.success(), "{args:?}: {}", stderr(&out));
            assert!(stdout(&out).starts_with("real-kubectl "), "{args:?}");
        }
        for (args, said) in [
            (vec!["rollout", "restart", "deploy/web"], "kubectl rollout restart"),
            (vec!["rollout", "undo", "deploy/web"], "kubectl rollout undo"),
            (vec!["rollout", "pause", "deploy/web"], "kubectl rollout pause"),
        ] {
            let out = f.run("kubectl", &args);
            assert_eq!(out.status.code(), Some(64), "{args:?} was not refused");
            assert_eq!(stderr(&out), read_only_refusal(said), "{args:?}");
        }
    }

    #[test]
    fn a_read_only_terminal_finds_the_subcommand_past_flags_and_their_values() {
        let f = fixture_in_mode("dev-cluster", None, true);
        for (tool, args) in [
            ("kubectl", vec!["-n", "kube-system", "get", "pods"]),
            // A namespace that happens to be called `delete` is a namespace.
            ("kubectl", vec!["--namespace", "delete", "get", "pods"]),
            ("kubectl", vec!["--namespace=prod", "-v=6", "get", "pods"]),
            // A flag that takes no value leaves the next word the subcommand.
            ("kubectl", vec!["--insecure-skip-tls-verify", "get", "pods"]),
            ("kubectl", vec!["--alsologtostderr", "get", "pods"]),
            ("kubectl", vec!["rollout", "-n", "prod", "status", "deploy/web"]),
            ("helm", vec!["-n", "prod", "list"]),
            ("helm", vec!["--debug", "status", "web"]),
        ] {
            let out = f.run(tool, &args);
            assert!(out.status.success(), "{tool} {args:?}: {}", stderr(&out));
        }
        for (tool, args, said) in [
            // A read standing as a flag's value is the value: the subcommand
            // is the word after it.
            ("kubectl", vec!["-n", "get", "delete", "pods", "--all"], "kubectl delete"),
            // A flag the guard has never heard of takes the next word, as
            // kubectl itself reads it: `get` is delete's label selector here.
            ("kubectl", vec!["-l", "get", "delete", "pods"], "kubectl delete"),
            ("kubectl", vec!["--selector", "get", "delete", "pods"], "kubectl delete"),
            ("kubectl", vec!["rollout", "-n", "status", "restart", "deploy/web"], "kubectl rollout restart"),
            ("helm", vec!["-n", "list", "uninstall", "web"], "helm uninstall"),
            ("helm", vec!["--kube-as-user", "list", "uninstall", "web"], "helm uninstall"),
            ("kubectl", vec!["--logtostderr", "apply", "-f", "web.yaml"], "kubectl apply"),
        ] {
            let out = f.run(tool, &args);
            assert_eq!(out.status.code(), Some(64), "{tool} {args:?} was not refused");
            assert_eq!(stderr(&out), read_only_refusal(said), "{tool} {args:?}");
        }
        // Every klog boolean kubectl 1.25 and earlier registered takes no
        // value. Read as taking one, it swallows the subcommand after it, the
        // check finds none, and the write runs.
        for flag in [
            "--add-dir-header",
            "--alsologtostderr",
            "--logtostderr",
            "--one-output",
            "--skip-headers",
            "--skip-log-headers",
        ] {
            let out = f.run("kubectl", &[flag, "apply", "-f", "web.yaml"]);
            assert_eq!(out.status.code(), Some(64), "kubectl {flag} apply was not refused");
            assert_eq!(stderr(&out), read_only_refusal("kubectl apply"), "kubectl {flag} apply");
        }
    }

    #[test]
    fn a_read_only_terminal_stops_at_a_double_dash_unless_a_flag_is_waiting_for_it() {
        let f = fixture_in_mode("dev-cluster", None, true);
        // After `--` the words are not kubectl's subcommand, to kubectl as here.
        let out = f.run("kubectl", &["--", "delete", "pods"]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(stdout(&out).starts_with("real-kubectl "));
        // A flag still waiting for its value takes `--` as that value, in
        // kubectl as here, and the subcommand is the word after it.
        let out = f.run("kubectl", &["-n", "--", "delete", "pods", "-A"]);
        assert_eq!(out.status.code(), Some(64), "{}", stdout(&out));
        assert_eq!(stderr(&out), read_only_refusal("kubectl delete"));
    }

    #[test]
    fn a_terminal_that_is_not_read_only_still_hands_writes_over() {
        let f = fixture("dev-cluster", Some("payments"));
        for (tool, args) in [
            ("kubectl", vec!["delete", "pod", "web-0"]),
            ("kubectl", vec!["rollout", "restart", "deploy/web"]),
            ("kubectl", vec!["-l", "app", "delete", "pods"]),
            ("helm", vec!["uninstall", "web"]),
        ] {
            let out = f.run(tool, &args);
            assert!(out.status.success(), "{tool} {args:?}: {}", stderr(&out));
            assert!(stdout(&out).starts_with(&format!("real-{tool} ")), "{tool} {args:?}");
        }
        for tool in [Tool::Kubectl, Tool::Helm] {
            let script = std::fs::read_to_string(f.scope.bin.as_ref().unwrap().join(tool.name())).unwrap();
            assert!(!script.contains("read-only"), "{script}");
        }
        let init = std::fs::read_to_string(f.scope.dir.join("init.sh")).unwrap();
        assert!(!init.contains("read-only"), "{init}");
    }

    #[test]
    fn a_read_only_terminal_says_so_in_its_prompt() {
        let f = fixture_in_mode("dev-cluster", Some("payments"), true);
        let home = tempfile::tempdir().unwrap();
        let out = interactive(&f, "bash", home.path(), "PS1='$ '; __srelens_prompt; echo \"ps1=$PS1\"");
        let said = stdout(&out);
        assert!(said.contains("ps1=[dev-cluster/payments read-only] $ "), "{said}\n{}", stderr(&out));
    }
}
