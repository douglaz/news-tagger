{
  description = "news-tagger - CLI tool for classifying posts using LLM-powered narrative tagging";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            # Rust toolchain
            rustToolchain

            # Build dependencies
            pkg-config
            openssl
            sqlite

            # Development tools
            cargo-watch
            cargo-edit
            cargo-outdated
            cargo-audit

            # Optional: for Ralph/Codex workflow
            nodejs_22
          ];

          shellHook = ''
            echo "news-tagger development shell"
            echo "Rust: $(rustc --version)"
            echo ""
            echo "Commands:"
            echo "  cargo test           - Run all tests"
            echo "  cargo fmt --all      - Format code"
            echo "  cargo clippy --all-targets --all-features -D warnings"
            echo "  cargo run -- --help  - Show CLI help"
          '';

          RUST_BACKTRACE = "1";
          RUST_LOG = "info";
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "news-tagger";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ openssl sqlite ];
        };
      }
    );
}
