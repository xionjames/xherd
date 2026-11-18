#!/bin/sh
# Prints the Homebrew formula for a release (used by .github/workflows/release.yml).
# Usage: formula.sh VERSION SHA256_ARM64 SHA256_X86_64
set -eu
version=$1 arm=$2 intel=$3
base="https://github.com/xionjames/xherd/releases/download/v${version}/xherd-${version}"
cat <<RB
class Xherd < Formula
  desc "Run, watch and steer many coding agents, each in its own git worktree"
  homepage "https://github.com/xionjames/xherd"
  version "${version}"
  license "GPL-3.0-or-later"

  depends_on "git"
  depends_on "tmux"

  on_macos do
    on_arm do
      url "${base}-aarch64-apple-darwin.tar.gz"
      sha256 "${arm}"
    end
    on_intel do
      url "${base}-x86_64-apple-darwin.tar.gz"
      sha256 "${intel}"
    end
  end

  def install
    bin.install "xherd"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/xherd --version")
  end
end
RB
