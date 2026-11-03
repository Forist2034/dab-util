{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-26.05";

    microvm = {
      url = "github:microvm-nix/microvm.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs =
    {
      nixpkgs,
      microvm,
      self,
      ...
    }:
    {
      nixosConfigurations = {
        test-vm-x86-64-linux = nixpkgs.lib.nixosSystem {
          system = "x86_64-linux";
          modules = [
            microvm.nixosModules.microvm
            {
              microvm = {
                vcpu = 2;
                mem = 1024;
                shares = [
                  {
                    tag = "nix-store";
                    source = "/nix/store";
                    mountPoint = "/nix/.ro-store";
                    readOnly = true;
                  }
                  {
                    tag = "source";
                    source = "source";
                    mountPoint = "/home/test/source";
                    readOnly = true;
                  }
                  {
                    tag = "input";
                    source = "input";
                    mountPoint = "/home/test/input";
                    readOnly = true;
                  }
                ];
              };
            }
            ({ pkgs, lib, ... }: {
              users.users.test = {
                isNormalUser = true;
                password = "";
                extraGroups = [ "wheel" ];
              };
              security.sudo.wheelNeedsPassword = false;
              services.getty.autologinUser = "test";

              environment.systemPackages = with pkgs; [
                nushell
                lzip
                eza
                fd
              ];

              system.stateVersion = lib.trivial.release;
            })
          ];
        };
      };
      packages = {
        x86_64-linux = {
          test-vm = self.nixosConfigurations.test-vm-x86-64-linux.config.microvm.declaredRunner;
        };
      };
    };
}
