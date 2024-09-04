let
  pkgs = import <nixpkgs> {};
in
pkgs.mkShell {
  packages = [
    pkgs.cargo
    pkgs.rustc

    pkgs.rust-analyzer
    pkgs.rustfmt

    pkgs.python311
    pkgs.python311Packages.pip

    pkgs.nodejs_22

    pkgs.gcc
    pkgs.cmake

    pkgs.pkg-config
    pkgs.openssl
  ];

  env = {
    RUST_BACKTRACE = "full";
  };
}

