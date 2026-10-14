#!/usr/bin/env bash
# Write Casks/opensheet.rb of the Homebrew tap 0xtlt/homebrew-tap for a published release:
# its version and the checksums of the two macOS disk images, read from the release's
# SHA256SUMS.
#
#   VERSION=0.2.0 tools/release/update-homebrew-tap.sh
#
# On GitHub Actions it pushes with GH_TOKEN, a token that can write to the tap (the secret
# HOMEBREW_TAP_TOKEN), and does nothing without one. Run by hand, it pushes with your own
# git credentials.
set -euo pipefail

TAP_REPO="${TAP_REPO:-0xtlt/homebrew-tap}"
SOURCE_REPO="${SOURCE_REPO:-0xtlt/opensheet}"
VERSION="${VERSION:?VERSION is required (e.g. 0.2.0)}"
VERSION="${VERSION#v}"

if [[ -n "${CI:-}" && -z "${GH_TOKEN:-}" ]]; then
  echo "::warning::HOMEBREW_TAP_TOKEN is not set; the Homebrew tap was not updated"
  exit 0
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

gh release download "v${VERSION}" --repo "${SOURCE_REPO}" --pattern SHA256SUMS --dir "${workdir}"
checksum() {
  local sum
  sum="$(awk -v file="$1" '$2 == file { print $1; exit }' "${workdir}/SHA256SUMS")"
  [[ -n "${sum}" ]] || { echo "no checksum for $1 in SHA256SUMS" >&2; exit 1; }
  printf '%s' "${sum}"
}
ARM="$(checksum opensheet-aarch64-apple-darwin.dmg)"
INTEL="$(checksum opensheet-x86_64-apple-darwin.dmg)"

tap="${workdir}/homebrew-tap"
if [[ -n "${GH_TOKEN:-}" && -n "${CI:-}" ]]; then
  git clone --depth 1 "https://x-access-token:${GH_TOKEN}@github.com/${TAP_REPO}.git" "${tap}"
  git -C "${tap}" config user.name "github-actions[bot]"
  git -C "${tap}" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
  git -C "${tap}" config --local credential.helper ""
else
  gh repo clone "${TAP_REPO}" "${tap}" -- --depth 1
fi

mkdir -p "${tap}/Casks"
cat > "${tap}/Casks/opensheet.rb" <<CASK
cask "opensheet" do
  arch arm: "aarch64", intel: "x86_64"

  version "${VERSION}"
  sha256 arm:   "${ARM}",
         intel: "${INTEL}"

  url "https://github.com/${SOURCE_REPO}/releases/download/v#{version}/opensheet-#{arch}-apple-darwin.dmg"
  name "OpenSheet"
  desc "Spreadsheet with the interface and behaviour of Excel"
  homepage "https://github.com/${SOURCE_REPO}"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :monterey"

  app "OpenSheet.app"
  binary "#{appdir}/OpenSheet.app/Contents/MacOS/opensheet"

  zap trash: [
    "~/Library/Application Support/OpenSheet",
    "~/Library/Preferences/org.opensheet.OpenSheet.plist",
    "~/Library/Saved Application State/org.opensheet.OpenSheet.savedState",
  ]
end
CASK

# The README lists the cask once.
readme="${tap}/README.md"
if [[ -f "${readme}" ]] && ! grep -q '| `opensheet`' "${readme}"; then
  perl -0pi -e 's/(brew install mactmux\n)/$1brew install opensheet\n/' "${readme}"
  perl -0pi -e 's/(\| `mactmux` \|[^\n]*\n)/$1| `opensheet` | Open-source spreadsheet with the interface of Excel |\n/' "${readme}"
fi

cd "${tap}"
git add Casks/opensheet.rb README.md
if git diff --cached --quiet; then
  echo "the tap already has OpenSheet ${VERSION}"
  exit 0
fi
git commit -q -m "opensheet ${VERSION}"
git push -q origin "HEAD:$(git rev-parse --abbrev-ref HEAD)"
echo "${TAP_REPO}: Casks/opensheet.rb is now ${VERSION}"
