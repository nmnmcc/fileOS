# SPDX-License-Identifier: AGPL-3.0-only

{
  pkgs,
  lib,
  config,
  ...
}:

let
  version = (lib.importTOML ./Cargo.toml).workspace.package.version;

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

  # Python for the scripts in scripts/. uv manages the virtualenv and
  # installs the locked dev tools from uv.lock on shell entry.
  languages.python = {
    enable = true;
    # Pinned in .python-version, the single source of truth (also read by uv).
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
  # wrapper, whose hardening flags do not exist for WebAssembly.
  env.CC_wasm32_unknown_unknown = "${pkgs.llvmPackages.clang-unwrapped}/bin/clang";
  env.AR_wasm32_unknown_unknown = "${pkgs.llvmPackages.llvm}/bin/llvm-ar";

  # Astro collects anonymous telemetry by default. We turn it off.
  env.ASTRO_TELEMETRY_DISABLED = "1";

  packages = [
    pkgs.cargo-deny
    pkgs.cargo-machete
    pkgs.cargo-llvm-lines
    pkgs.git
  ];

  # One command formats every file in the repository: `treefmt`.
  # `check` runs `treefmt --ci`, which fails if anything is not formatted.
  treefmt = {
    enable = true;
    config = {
      # Lock files, LICENSE and .gitignore are excluded by default.
      settings.global.excludes = [
        "crates/*/LICENSE-*"
        ".envrc"
        ".python-version"
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

  # devenv formats the whole tree on every shell entry by default. Formatting
  # should never rewrite files behind your back, and it would hide unformatted
  # code from `treefmt --ci` in CI. Run `treefmt` yourself instead.
  tasks."devenv:treefmt:run".before = lib.mkForce [ ];

  # Build artifacts: `devenv build outputs.hs` or `devenv build outputs.docs`.
  outputs = {
    hs = rustPlatform.buildRustPackage {
      pname = "hs";
      inherit version;
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./crates
        ];
      };
      cargoLock.lockFile = ./Cargo.lock;
      cargoBuildFlags = [
        "--package"
        "hs"
      ];
      cargoTestFlags = [ "--workspace" ];
      meta = {
        description = "homeserver";
        license = lib.licenses.agpl3Only;
        mainProgram = "hs";
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
      # Dependencies come straight from docs/package-lock.json; no hash to maintain.
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

  # `devenv test` runs the same checks, but only reports pass or fail.
  enterTest = "check";
}
