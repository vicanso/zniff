cask "zniff" do
  arch arm: "aarch64", intel: "x86_64"

  version "0.4.0"
  sha256 arm:   "REPLACE_WITH_SHA256_FROM_RELEASE_SHA256SUMS_AARCH64_DMG",
         intel: "REPLACE_WITH_SHA256_FROM_RELEASE_SHA256SUMS_X86_64_DMG"

  url "https://github.com/vicanso/zniff/releases/download/v#{version}/zniff-#{arch}.dmg",
      verified: "github.com/vicanso/zniff/"
  name "zniff"
  desc "Single-process HTTP(S) debugging proxy in pure Rust with native GUI, Web console, and MCP"
  homepage "https://github.com/vicanso/zniff"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :ventura"

  app "zniff.app"
  binary "#{appdir}/zniff.app/Contents/MacOS/zniff"

  zap trash: [
    "~/Library/Application Support/zniff",
    "~/Library/Caches/zniff",
  ]
end
