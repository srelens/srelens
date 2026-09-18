use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "srelens-tui",
    version,
    about = "Kubernetes control room in your terminal — built in Rust with k9s navigation"
)]
pub struct Cli {
    /// Kubernetes namespace to scope the initial view
    #[arg(short, long)]
    pub namespace: Option<String>,

    /// Scope to all namespaces on launch
    #[arg(short = 'A', long)]
    pub all_namespaces: bool,

    /// Kubernetes context to activate
    #[arg(short, long)]
    pub context: Option<String>,

    /// Custom kubeconfig path
    #[arg(short, long)]
    pub kubeconfig: Option<PathBuf>,

    /// Run headless Model Context Protocol (MCP) server over stdio
    #[arg(long = "mcp-stdio")]
    pub mcp_stdio: bool,

    /// Allow cluster-mutating / destructive capabilities with --mcp-stdio
    #[arg(long = "mcp-allow-destructive")]
    pub mcp_allow_destructive: bool,

    /// Allow secret-reading capabilities with --mcp-allow-sensitive-reads
    #[arg(long = "mcp-allow-sensitive-reads")]
    pub mcp_allow_sensitive_reads: bool,

    #[command(subcommand)]
    pub command: Option<CliCommand>,

    /// Deep link URL (srelens://...) or resource target (e.g. pods, nodes, pods/my-pod)
    pub target: Option<String>,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// Print cluster overview & reachability information
    Info,
    /// Run headless Model Context Protocol (MCP) server over stdio
    Mcp {
        /// Allow cluster-mutating / destructive capabilities (requires caller _confirm: true)
        #[arg(long = "allow-destructive")]
        allow_destructive: bool,
        /// Allow secret-reading capabilities (e.g. k8s.getSecret)
        #[arg(long = "allow-sensitive-reads")]
        allow_sensitive_reads: bool,
    },
    /// Check toolbox diagnostics (kubectl, helm, krew)
    Toolbox,
    /// Print version information
    Version,
    /// Update srelens-tui to the latest release
    Update {
        /// Report what an update would do, without changing anything
        #[arg(long)]
        check: bool,
        /// Which releases to consider: stable, or the rolling dev
        /// pre-releases. Defaults to the channel this binary came from.
        #[arg(long, value_parser = ["stable", "dev"])]
        channel: Option<String>,
    },
}
