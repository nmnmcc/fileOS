# SPDX-License-Identifier: AGPL-3.0-only

# Native Shell's CLI, which builds the web app in crates/hs-ui, pinned and
# kept in the Nix store instead of ~/.nsh. `nsh` finds its runtimes next to
# itself, so the web component is unpacked into the same directory and it
# never downloads one. The hashes are the release's own .sha256 files.
#
# To update: change `version`, then take each archive's hash from
# `<archive>.tar.gz.sha256` (`nix hash convert --hash-algo sha256 <hex>`).

{ pkgs, lib }:

let
  version = "0.7.0";
  release =
    name: hash:
    pkgs.fetchurl {
      url = "https://github.com/longbridge/nsh/releases/download/v${version}/${name}.tar.gz";
      inherit hash;
    };
  platform =
    {
      aarch64-darwin = release "nsh-macos-arm64" "sha256-Wy5TheCaZe1jsXio3ZzvA6ncqyBNLvyUIwDgxbrqOFM=";
      x86_64-darwin = release "nsh-macos-x86_64" "sha256-0Fvs56maBDxkY9G6KKkj4Yj7U5EQUU/enaZ3XGsmnas=";
      x86_64-linux = release "nsh-linux-x86_64" "sha256-0oG/o9k9hOmDHhqX39tjEEMp/Mh6CkjHjbWuq7AMAHM=";
      aarch64-linux = release "nsh-linux-arm64" "sha256-g5LsAmvTLRVUsi9a6LV0q8t0a1ArxqyTvNyhQ2/bv/w=";
    }
    .${pkgs.stdenv.hostPlatform.system};
  web = release "nsh-web" "sha256-6VHniWamWvTXoLvNCKSl7vTztrStrsO0tQLffE/otOY=";
in
pkgs.stdenvNoCC.mkDerivation {
  pname = "nsh";
  inherit version;
  srcs = [
    platform
    web
  ];
  sourceRoot = ".";
  # Only the CLI runs here; the desktop runtimes need GTK and are unused.
  nativeBuildInputs = lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.autoPatchelfHook ];
  buildInputs = lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.stdenv.cc.cc.lib ];
  autoPatchelfIgnoreMissingDeps = true;
  dontStrip = true;
  installPhase = ''
    runHook preInstall
    cp -r nsh "$out"
    runHook postInstall
  '';
  meta = {
    description = "Native Shell CLI with its web runtime";
    homepage = "https://nsh.dev";
    license = lib.licenses.asl20;
    mainProgram = "nsh";
  };
}
