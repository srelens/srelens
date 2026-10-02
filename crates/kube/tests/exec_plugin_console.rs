//! Kubeconfig exec plugins (`aws eks get-token` and the like) are console
//! programs. A release build of the desktop app has no console of its own, so
//! on Windows each one it started opened a console window (#775). The desktop
//! sets [`HIDE_EXEC_PLUGIN_WINDOWS_ENV`] to `1` at startup; this pins that
//! kube-rs then starts them with `CREATE_NO_WINDOW`, so the plugin gets a
//! console with no window. It runs in the Windows CI job, so a kube upgrade
//! that renames or drops the variable fails there.
//!
//! A test binary of its own, because it changes this process's environment.
#![cfg(windows)]

use std::path::{Path, PathBuf};

use srelens_kube::client_cache::ClientCache;
use srelens_kube::connect::HIDE_EXEC_PLUGIN_WINDOWS_ENV;

/// What one run of the probe plugin found about its console.
struct Console {
    /// `GetConsoleWindow`: 0 when the console has no window.
    window: i64,
    /// `GetConsoleProcessList`: every process attached to that console.
    attached: Vec<u32>,
}

/// Write a kubeconfig whose user authenticates through a PowerShell exec
/// plugin. Each run of the plugin leaves a file in `runs/`: its console
/// window handle on the first line, the processes on its console on the
/// second.
///
/// Both are needed. A plugin started without the flag shares this test's
/// console, and whether that console has a window depends on how the test
/// was launched: from a terminal it has one, from a tool runner it may not.
/// Sharing it is the giveaway either way. A parent with no console at all,
/// the release app's case, hands the plugin a new console that has a window.
fn probe_kubeconfig(dir: &Path) -> PathBuf {
    let runs = dir.join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    let plugin = dir.join("probe.ps1");
    std::fs::write(
        &plugin,
        format!(
            "Add-Type -Namespace Probe -Name Console -MemberDefinition '\
             [DllImport(\"kernel32.dll\")] public static extern System.IntPtr GetConsoleWindow();\
             [DllImport(\"kernel32.dll\")] public static extern uint GetConsoleProcessList(uint[] ids, uint count);'\r\n\
             $ids = New-Object 'uint32[]' 64\r\n\
             $count = [Probe.Console]::GetConsoleProcessList($ids, 64)\r\n\
             $report = @([Probe.Console]::GetConsoleWindow().ToInt64(), ($ids[0..($count - 1)] -join ','))\r\n\
             Set-Content -Path (Join-Path '{}' ([guid]::NewGuid().ToString())) -Value $report\r\n\
             Write-Output '{{\"apiVersion\":\"client.authentication.k8s.io/v1beta1\",\"kind\":\"ExecCredential\",\"status\":{{\"token\":\"t\",\"expirationTimestamp\":\"2099-01-01T00:00:00Z\"}}}}'\r\n",
            // Doubled: the path sits in a single-quoted PowerShell string.
            runs.display().to_string().replace('\'', "''")
        ),
    )
    .unwrap();
    let kubeconfig = dir.join("config");
    std::fs::write(
        &kubeconfig,
        format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: eks\n\
             clusters:\n- name: cl\n  cluster: {{server: \"https://127.0.0.1:1\"}}\n\
             users:\n- name: u\n  user:\n    exec:\n      apiVersion: client.authentication.k8s.io/v1beta1\n      \
             command: powershell\n      \
             args: [\"-NoProfile\", \"-NonInteractive\", \"-ExecutionPolicy\", \"Bypass\", \"-File\", \"{}\"]\n      \
             interactiveMode: Never\n\
             contexts:\n- name: eks\n  context: {{cluster: cl, user: u}}\n",
            plugin.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    kubeconfig
}

#[test]
fn exec_plugins_start_without_a_console_window() {
    // First, while this is the only thread doing anything.
    std::env::set_var(HIDE_EXEC_PLUGIN_WINDOWS_ENV, "1");

    // An apostrophe, as in `C:\Users\O'Brien`, would end a single-quoted
    // PowerShell string in the probe if it were not escaped.
    let dir = tempfile::Builder::new()
        .prefix("srelens-o'775-")
        .tempdir()
        .unwrap();
    let cache = ClientCache::new(probe_kubeconfig(dir.path()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(cache.get("eks"))
        .expect("the exec plugin hands kube-rs a token");

    let runs: Vec<Console> = std::fs::read_dir(dir.path().join("runs"))
        .unwrap()
        .map(|entry| {
            let report = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let mut lines = report.lines();
            Console {
                window: lines.next().unwrap().trim().parse().unwrap(),
                attached: lines
                    .next()
                    .unwrap()
                    .trim()
                    .split(',')
                    .map(|id| id.parse().unwrap())
                    .collect(),
            }
        })
        .collect();
    assert!(!runs.is_empty(), "the exec plugin never ran");
    let this_test = std::process::id();
    for run in runs {
        assert!(
            !run.attached.contains(&this_test),
            "an exec plugin shared this process's console: {:?}",
            run.attached
        );
        assert_eq!(run.window, 0, "an exec plugin was given a console window");
    }
}
