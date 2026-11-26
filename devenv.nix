# SPDX-License-Identifier: AGPL-3.0-only

{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

let
  workspace = (lib.importTOML ./Cargo.toml).workspace.package;
  version = workspace.version;
  msrv =
    (pkgs.extend inputs.rust-overlay.overlays.default)
    .rust-bin.stable.${workspace.rust-version}.minimal;

  # Build with the same toolchain as development: the one in rust-toolchain.toml.
  toolchain = config.languages.rust.toolchainPackage;
  rustPlatform = pkgs.makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };
in
{
  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;
  };

  # Python for the scripts in scripts/. uv looks after the virtualenv. When
  # the shell starts, it installs the locked dev tools from uv.lock.
  languages.python = {
    enable = true;
    # Pinned in .python-version, the one source of truth (uv reads it too).
    package = pkgs."python${lib.replaceStrings [ "." ] [ "" ] (lib.fileContents ./.python-version)}";
    venv.enable = true;
    uv = {
      enable = true;
      sync.enable = true;
    };
  };

  # Node.js for the documentation site (Astro Starlight in docs/).
  languages.javascript = {
    enable = true;
    npm.enable = true;
  };

  # Portable crates are also built for the browser (wasm32-unknown-unknown).
  # C code in dependencies such as `ring` needs a clang without the Nix
  # wrapper, because the wrapper's hardening flags do not exist for WebAssembly.
  env.CC_wasm32_unknown_unknown = "${pkgs.llvmPackages.clang-unwrapped}/bin/clang";
  env.AR_wasm32_unknown_unknown = "${pkgs.llvmPackages.llvm}/bin/llvm-ar";

  # By default, Astro collects anonymous telemetry. Turn it off.
  env.ASTRO_TELEMETRY_DISABLED = "1";
  env.HS_MSRV_CARGO = "${msrv}/bin/cargo";

  packages = [
    pkgs.cargo-deny
    pkgs.cargo-machete
    pkgs.cargo-llvm-lines
    pkgs.git
    pkgs.gh
    pkgs.knope # change files and releases: knope.toml
    pkgs.wasm-tools # makes components of the apps: scripts/apps.py
  ];

  # One command formats every file in the repository: `treefmt`.
  # `check` runs `treefmt --ci`, which fails if anything is not formatted.
  treefmt = {
    enable = true;
    config = {
      # By default, lock files, LICENSE and .gitignore are left out.
      settings.global.excludes = [
        "crates/*/LICENSE-*"
        ".envrc"
        ".python-version"
        # Generated, and checked against the code by a test in hs-api.
        "docs/openapi.json"
      ];
      programs = {
        rustfmt = {
          enable = true;
          package = toolchain; # the rustfmt pinned in rust-toolchain.toml
          edition = "2024";
        };
        nixfmt.enable = true;
        ruff-format = {
          enable = true;
          lineLength = 100; # same width as rustfmt
        };
        ruff-check.enable = true; # sorts imports (see settings below)
        taplo.enable = true;
        prettier.enable = true;
      };
      settings.formatter.ruff-check.options = [
        "--select"
        "I"
      ];
    };
  };

  # By default, devenv formats the whole tree every time the shell starts. That
  # changes files when no one asked, and keeps unformatted code hidden from
  # `treefmt --ci` in CI. So it is off; run `treefmt` yourself.
  tasks."devenv:treefmt:run".before = lib.mkForce [ ];

  # Build artifacts: `devenv build outputs.hs` or `devenv build outputs.docs`.
  outputs = {
    fos = rustPlatform.buildRustPackage {
      pname = "fos";
      inherit version;
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
          ./wit
        ];
      };
      cargoLock.lockFile = ./Cargo.lock;
      cargoBuildFlags = [
        "--package"
        "fos"
      ];
      # The tests need the WebAssembly apps in target/apps/, which are built
      # outside Nix. `check` runs every test.
      doCheck = false;
      meta = {
        description = "fileOS";
        license = lib.licenses.agpl3Only;
        mainProgram = "fos";
      };
    };

    docs = pkgs.buildNpmPackage {
      pname = "homeserver-docs";
      inherit version;
      src = lib.fileset.toSource {
        root = ./docs;
        fileset = lib.fileset.difference ./docs (
          lib.fileset.unions [
            (lib.fileset.maybeMissing ./docs/node_modules)
            (lib.fileset.maybeMissing ./docs/dist)
            (lib.fileset.maybeMissing ./docs/.astro)
          ]
        );
      };
      # Dependencies come straight from docs/package-lock.json, so there is no
      # hash to keep up to date.
      npmDeps = pkgs.importNpmLock {
        package = lib.importJSON ./docs/package.json;
        packageLock = lib.importJSON ./docs/package-lock.json;
      };
      npmConfigHook = pkgs.importNpmLock.npmConfigHook;
      env.ASTRO_TELEMETRY_DISABLED = "1";
      installPhase = ''
        runHook preInstall
        cp -r dist "$out"
        runHook postInstall
      '';
      meta.license = lib.licenses.agpl3Only;
    };
  };

  # Every scripts/<name>.py is available as the command <name> (check, budget, docs, ...).
  scripts =
    lib.mapAttrs'
      (file: _: {
        name = lib.removeSuffix ".py" file;
        value.exec = ''"$DEVENV_ROOT/scripts/${file}" "$@"'';
      })
      (
        lib.filterAttrs (file: type: type == "regular" && lib.hasSuffix ".py" file) (
          builtins.readDir ./scripts
        )
      );

  # `devenv test` runs the same checks, but only says pass or fail.
  enterTest = "check";
}
