{
  description = "rbitcoin — pinned Nix builds for byte-identical release binaries";

  # Track the current NixOS **stable/large** channel (Hydra-tested; cache.nixos.org).
  # Advance deliberately with `nix flake update` every ~6 months or near channel EOL —
  # not daily. Exact rev is frozen in flake.lock (never floating import <nixpkgs> {}).
  # See docs/reproducible-builds.md § pin policy.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  # Layered cargo builds: deps derivation + app derivation (faster rebuilds).
  # Crane ≥0.23 targets modern nixpkgs; keep in lockstep with the channel bump.
  inputs.crane.url = "github:ipetkov/crane/v0.24.0";

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    let
      # Release packages + checks. Linux only, deliberately: the Nix path exists
      # to produce fully static musl binaries, and Apple forbids a static
      # libSystem link, so a Darwin `nix build` is store-rpath (not portable).
      # Darwin/Windows ship from GHA instead — docs/reproducible-builds.md
      # § Windows / Darwin snapshots.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      # Dev shells only pin *tool* versions; they build nothing that ships, so
      # the static-link constraint above does not apply to them. Darwin is a
      # required PR gate (ci.yml `macos`) over real Darwin-only store code
      # (pool session, bulk_io, sorted_run), so those contributors get the same
      # pinned rustc / LLVM / ast-grep as Linux.
      devSystems = systems ++ [ "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      forAllDevSystems = nixpkgs.lib.genAttrs devSystems;

      mkRbitcoin =
        pkgs:
        pkgs.callPackage ./nix/rbitcoin.nix {
          craneLib = crane.mkLib pkgs;
        };
    in
    {
      nixosModules = {
        default = self.nixosModules.rbitcoin;
        rbitcoin = import ./nix/modules/rbitcoin.nix {
          defaultPackage = system: self.packages.${system}.rbitcoin-musl;
        };
      };

      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            # Disable impure overlays; pure evaluation for reproducibility.
            config = { };
            overlays = [ ];
          };
          # Optional dynamic glibc package (Nix-store linked; not portable off-store).
          rbitcoin-glibc = mkRbitcoin pkgs;
          # Primary / default: fully static musl — portable operator binary.
          # callPackage under pkgsStatic so rustc's sysroot includes musl std;
          # rbitcoin.nix scopes static link flags to the host target only.
          rbitcoin-musl = mkRbitcoin pkgs.pkgsStatic;
        in
        {
          default = rbitcoin-musl;
          rbitcoin = rbitcoin-musl;
          rbitcoin-node = rbitcoin-musl;
          rbitcoin-cli = rbitcoin-musl;
          rbitcoin-musl = rbitcoin-musl;
          # Kept for store-native Nix environments / optional dual-platform repro.
          rbitcoin-glibc = rbitcoin-glibc;
        }
        // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          # Optional third platform: aarch64-linux cross from x86_64 (heavy toolchain).
          rbitcoin-aarch64 =
            let
              pkgsAarch64 = import nixpkgs {
                system = "x86_64-linux";
                crossSystem = {
                  config = "aarch64-unknown-linux-gnu";
                };
                config = { };
                overlays = [ ];
              };
            in
            mkRbitcoin pkgsAarch64;
        }
      );

      # Dev shell uses the **same pinned** nixpkgs (not floating <nixpkgs>).
      devShells = forAllDevSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config = { };
            overlays = [ ];
          };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rustc
              cargo
              rustfmt
              clippy
              # Match rustc's LLVM major (nixos-26.05 → rustc 1.95 → LLVM 21).
              llvmPackages.bintools
              llvmPackages.llvm
              cargo-llvm-cov
              ast-grep
              pkg-config
            ];
            RUST_BACKTRACE = "1";
            # Dev shell still denies warnings; release package uses its own RUSTFLAGS.
            RUSTFLAGS = "-Dwarnings";
            shellHook = ''
              export LLVM_COV="${pkgs.llvmPackages.llvm}/bin/llvm-cov"
              export LLVM_PROFDATA="${pkgs.llvmPackages.llvm}/bin/llvm-profdata"
              # Host gnu debug (fmt/clippy/test). Coverage → target/cov; musl → nix/crane.
              # Agent VM: export CARGO_TARGET_DIR=/tmp/rbtc-target/dev first (TESTING.md).
              if [ -z "''${CARGO_TARGET_DIR:-}" ]; then
                export CARGO_TARGET_DIR="$PWD/target/dev"
              fi
              echo "rbitcoin devShell: rustc=$(rustc --version) (pinned nixpkgs via flake) CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
            '';
          };
        }
      );

      # `nix flake check` can validate the package builds on the current system.
      checks = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config = { };
            overlays = [ ];
          };
        in
        {
          rbitcoin = self.packages.${system}.rbitcoin-musl;
          nixos-module-eval = import ./nix/tests/nixos-module-eval.nix {
            inherit nixpkgs pkgs;
            expectedPackage = self.packages.${system}.rbitcoin-musl;
            module = self.nixosModules.rbitcoin;
          };
        }
        // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          nixos-module-runtime = import ./nix/tests/nixos-module-runtime.nix {
            inherit pkgs;
            module = self.nixosModules.rbitcoin;
          };
        }
      );
    };
}
