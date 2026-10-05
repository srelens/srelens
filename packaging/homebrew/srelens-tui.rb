# srelens-tui — the terminal UI, installed from the prebuilt release archive.
#
# A formula rather than a cask: this is a command-line binary. The desktop app
# is a `.dmg` and wants `brew install --cask` (see #225), which is a separate
# piece of work published a different way.
#
# It installs the archive the release already publishes instead of compiling
# from source. Building srelens-tui pulls in the whole workspace — kube-rs,
# reqwest, ratatui, tokio — for a binary CI has already produced, signed and
# notarized for exactly these four targets. `brew install` should not take
# minutes to do again, worse, what a download does in seconds.
#
# The version and checksums below are placeholders. Every stable release
# renders them from that release's own SHA256SUMS and pushes the result to the
# tap; see packaging/homebrew/README.md. The committed file is the template,
# which is why it names a version that does not exist.
class SrelensTui < Formula
  desc "Kubernetes control room in your terminal, built in Rust with k9s navigation"
  homepage "https://github.com/srelens/srelens"
  version "0.0.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/srelens/srelens/releases/download/srelens-v0.0.0/srelens-tui-0.0.0-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/srelens/srelens/releases/download/srelens-v0.0.0/srelens-tui-0.0.0-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  # Homebrew runs on Linux too, and takes the STATIC musl archives there.
  #
  # Not the glibc ones, which is what this said first and was wrong about.
  # Those are built on ubuntu-22.04 and ubuntu-24.04-arm, so they carry a
  # glibc floor of 2.35 and 2.39 — and Homebrew on Linux deliberately
  # supports far older distributions than that, going as far as building
  # its own glibc when the host's is too old. A dynamically linked binary
  # would then fail before `main` with a GLIBC_2.3x symbol error, which
  # tells the user nothing about what to do.
  #
  # The static builds have no such floor. Their known costs — musl's
  # slower allocator, its narrower resolver — do not matter for a terminal
  # client that spends its time waiting on an API server.
  on_linux do
    on_arm do
      url "https://github.com/srelens/srelens/releases/download/srelens-v0.0.0/srelens-tui-0.0.0-aarch64-unknown-linux-musl.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/srelens/srelens/releases/download/srelens-v0.0.0/srelens-tui-0.0.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  # srelens deliberately does not bundle a toolchain — it drives the kubectl
  # and helm already on the machine, including kubeconfig exec-auth plugins —
  # so these are genuinely optional rather than dependencies.
    def caveats
    <<~EOS
      srectl is the command. srelens-tui still runs it, and prints a one-line
      reminder.

      srectl uses the kubectl and helm already on your PATH, if any.
      Neither is required to browse a cluster; `srectl toolbox` reports
      what it found.

      Homebrew owns this copy, so `srectl update` will decline to replace
      it and point you back here. Use `brew upgrade srelens-tui` instead.
    EOS
  end

  def install
    # The archive still holds the binary as `srelens-tui`. Install it under
    # the new command name, and leave the old name as a wrapper so existing
    # scripts keep working.
    bin.install "srelens-tui" => "srectl"
    (bin/"srelens-tui").write <<~SH
      #!/bin/sh
      [ -t 2 ] && echo "srelens-tui is now srectl" >&2
      exec "#{bin}/srectl" "$@"
    SH
    chmod 0755, bin/"srelens-tui"
  end

  test do
    # Asserts the binary runs AND that the formula's version matches what was
    # actually packaged — a mismatch means the render step and the release
    # disagree, which is worth failing on.
    assert_match version.to_s, shell_output("#{bin}/srectl --version")

    # The old command name runs the same binary.
    assert_match version.to_s, shell_output("#{bin}/srelens-tui --version 2>&1")

    # A command that needs no cluster, to prove the binary is not merely
    # loadable. With no kubeconfig it reports zero contexts rather than failing.
    assert_match "SRElens Kubernetes TUI", shell_output("#{bin}/srectl info")

    # The legacy wrapper also forwards subcommand arguments.
    assert_match "SRElens Kubernetes TUI", shell_output("#{bin}/srelens-tui info")
  end
end
