{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;

      protocolEnv = pkgs: {
        WAYLAND_XML = pkgs.fetchurl {
          url = "https://gitlab.freedesktop.org/wayland/wayland/-/raw/main/protocol/wayland.xml";
          hash = "sha256-zIYJh+VPjYXJQOl/oScMabbkrTH7z1p/ABB84RV/Xgc=";
        };
        EXT_TRANSIENT_SEAT_V1_XML = pkgs.fetchurl {
          url = "https://gitlab.freedesktop.org/wayland/wayland-protocols/-/raw/main/staging/ext-transient-seat/ext-transient-seat-v1.xml";
          hash = "sha256-19g2BHIYAwVYu9TnBuapt+kTOMSytjchW+al0m2ljYE=";
        };
        VIRTUAL_KEYBOARD_XML = pkgs.fetchurl {
          url = "https://gitlab.freedesktop.org/wlroots/wlroots/-/raw/5adf325333602d5b1e7ccdeb122633bbc8040ace/protocol/virtual-keyboard-unstable-v1.xml";
          hash = "sha256-eteHAAPs1ZLK5H3BnSd6YJt/GP17e+ASYjzzIlpylPU=";
        };
      };

      rustToolchain =
        pkgs:
        let
          rust-overlay-pkgs = pkgs.extend rust-overlay.overlays.default;
        in
        rust-overlay-pkgs.rust-bin.nightly.latest.minimal.override {
          extensions = [ "rust-src" ];
        };

      devToolchain =
        pkgs:
        (rustToolchain pkgs).override {
          extensions = [
            "rust-src"
            "rustfmt"
            "clippy"
            "rust-analyzer-preview"
          ];
        };

      wl-send-key =
        {
          pkgs,
          toolchain,
        }:
        let
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
          manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
          appDeps = rustPlatform.importCargoLock {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "wayland-oxide-0.2.0" = "sha256-JKRLSHUYJ3pU21rsrcBGtdmZcue2xBTvvfbJCLrBE1M=";
            };
          };
          stdDeps = rustPlatform.importCargoLock {
            lockFile = "${toolchain}/lib/rustlib/src/rust/library/Cargo.lock";
          };
        in
        rustPlatform.buildRustPackage {
          pname = manifest.name;
          version = manifest.version;
          src = pkgs.lib.cleanSource ./.;

          doCheck = false;

          cargoDeps = pkgs.runCommand "cargo-vendor-dir" { } ''
            mkdir -p "$out"
            cp -rs ${appDeps}/. "$out"
            # Keep the application's lockfile and source configuration.
            for crate in ${stdDeps}/*/; do
              ln -sfn "$crate" "$out/$(basename "$crate")"
            done
          '';
          RUSTFLAGS = [
            "-Zshare-generics=y"
            "-Zunstable-options"
            "-Cpanic=immediate-abort"
          ];
          env = protocolEnv pkgs;
          buildType = "prod";
          cargoBuildFlags = [
            "-Zbuild-std=core,std,alloc,panic_abort"
            "-Zbuild-std-features="
          ];
        };
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          toolchain = devToolchain pkgs;
        in
        {
          default = pkgs.mkShell {
            packages = [ toolchain ];

            env = protocolEnv pkgs;
          };
        }
      );
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          toolchain = rustToolchain pkgs;
        in
        {
          default = wl-send-key { inherit pkgs toolchain; };
        }
      );
    };
}
