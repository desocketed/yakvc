{
  description = "relay.yak.dsktd.net: the yakvc-server host";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # A deploy overrides this with the commit it ships
    # (`--override-input yakvc github:desocketed/yakvc/<rev>`), so the lock's
    # pin only matters for building the configuration on its own.
    yakvc = {
      url = "github:desocketed/yakvc";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, disko, yakvc, ... }: {
    nixosConfigurations.yakvc-relay = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        disko.nixosModules.disko
        yakvc.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
