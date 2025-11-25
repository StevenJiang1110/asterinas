{ config, lib, pkgs, ... }: {
  nixpkgs.overlays = [ (import ./hello-asterinas/default.nix) ];
}
