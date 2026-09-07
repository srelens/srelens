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

  # Homebrew runs on Linux too, and the release builds both architectures for
  # it. The glibc archives are the right ones here: Homebrew on Linux targets
  # glibc systems, and the musl builds exist for hosts (Alpine, older glibc)
  # that Homebrew does not support anyway.
  on_linux do
    on_arm do
      url "https://github.com/srelens/srelens/releases/download/srelens-v0.0.0/srelens-tui-0.0.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/srelens/srelens/releases/download/srelens-v0.0.0/srelens-tui-0.0.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  # srelens deliberately does not bundle a toolchain — it drives the kubectl
  # and helm already on the machine, including kubeconfig exec-auth plugins —
  # so these are genuinely optional rather than dependencies.
  def caveats
    <<~EOS
      srelens-tui uses the kubectl and helm already on your PATH, if any.
      Neither is required to browse a cluster; `srelens-tui toolbox` reports
      what it found.

      Homebrew owns this copy, so `srelens-tui update` will decline to replace
      it and point you back here. Use `brew upgrade srelens-tui` instead.
    EOS
  end

  def install
    # The archive holds the binary and LICENSE at its root.
    bin.install "srelens-tui"
  end

  test do
    # Asserts the binary runs AND that the formula's version matches what was
    # actually packaged — a mismatch means the render step and the release
    # disagree, which is worth failing on.
    assert_match version.to_s, shell_output("#{bin}/srelens-tui --version")

    # A command that needs no cluster, to prove the binary is not merely
    # loadable. With no kubeconfig it reports zero contexts rather than failing.
    assert_match "SRElens Kubernetes TUI", shell_output("#{bin}/srelens-tui info")
  end
end
