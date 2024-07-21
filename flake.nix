{
  description = "Description for the project";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    hercules-ci-effects.url = "github:hercules-ci/hercules-ci-effects";
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;} {
      imports = [
        inputs.hercules-ci-effects.flakeModule
      ];
      systems = ["x86_64-linux" "aarch64-linux" "aarch64-darwin" "x86_64-darwin"];
      # NOTE: I do not have runners for darwin
      herculesCI.ciSystems = ["x86_64-linux" "aarch64-linux"];

      perSystem = {
        config,
        self',
        inputs',
        pkgs,
        system,
        ...
      }: rec {
        # Per-system attributes can be defined here. The self' and inputs'
        # module parameters provide easy access to attributes of the same
        # system.

        packages.age-threshold = pkgs.rustPlatform.buildRustPackage {
          name = "age-threshold";
          src = ./.;

          cargoHash = "sha256-W3ts0jkxOGkiHu0F3phwNpbpqCbBLCIFzZ3uGkw4u1s=";
        };

        packages.default = packages.age-threshold;

        checks =
          pkgs.lib.mapAttrs'
          (k: v:
            pkgs.lib.nameValuePair "${k}-cargo-fmt" (v.overrideAttrs (prev: {
              name = "${prev.name}-cargo-fmt";

              nativeCheckInputs = (prev.nativeCheckInputs or []) ++ [pkgs.rustfmt];

              dontCargoBuild = true;
              checkPhase = "cargo fmt --check";
              installPhase = "touch $out";
            }))) {
            inherit (packages) age-threshold;
          }
          // pkgs.lib.mapAttrs'
          (k: v:
            pkgs.lib.nameValuePair "${k}-cargo-clippy" (v.overrideAttrs (prev: {
              name = "${prev.name}-cargo-clippy";

              nativeCheckInputs = (prev.nativeCheckInputs or []) ++ [pkgs.clippy];

              dontCargoBuild = true;
              checkPhase = "cargo clippy";
              installPhase = "touch $out";
            }))) {
            inherit (packages) age-threshold;
          };

        formatter = pkgs.alejandra;
      };
      flake = {
        # The usual flake attributes can be defined here, including system-
        # agnostic ones like nixosModule and system-enumerating ones, although
        # those are more easily expressed in perSystem.
      };
    };
}
