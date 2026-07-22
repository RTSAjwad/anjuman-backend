{
  description = "Classroom Anki Platform";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];

        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rust = pkgs.rust-bin.stable.latest.default;
      in {
        devShells.default = pkgs.mkShell {
          packages = [
            rust

            pkgs.rust-analyzer
            pkgs.sqlx-cli

            pkgs.sqlite

            pkgs.pkg-config
            pkgs.openssl

            pkgs.just
          ];

          shellHook = ''
            export DATABASE_URL=sqlite://platform.db

            if [ ! -f platform.db ]; then
              sqlite3 platform.db "PRAGMA journal_mode=WAL;"
            fi
          '';
        };
      });
}
