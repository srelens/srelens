# srelens — the signed and notarized macOS desktop application.
#
# The version and checksums below are placeholders. Every stable release
# downloads both public DMGs, hashes those exact files, renders this template,
# and publishes the result to srelens/homebrew-tap. See README.md.
cask "srelens" do
  arch arm: "aarch64", intel: "x64"

  version "0.0.0"
  sha256 arm:   "0000000000000000000000000000000000000000000000000000000000000000",
         intel: "1111111111111111111111111111111111111111111111111111111111111111"

  url "https://github.com/srelens/srelens/releases/download/srelens-v#{version}/srelens_#{version}_#{arch}.dmg"
  name "srelens"
  desc "Kubernetes control room for engineers and AI agents"
  homepage "https://github.com/srelens/srelens"

  auto_updates true
  depends_on :macos

  app "srelens.app"
end
