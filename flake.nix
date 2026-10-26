{
  description = "Find interactive Pi instances in tmux panes";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      packages.${system} = {
        default = self.packages.${system}.pifzf;
        pifzf = pkgs.rustPlatform.buildRustPackage {
          pname = "pifzf";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./src ./tests ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.makeWrapper ];
          postInstall = ''
            wrapProgram $out/bin/pifzf --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.tmux pkgs.fzf ]}
          '';
          meta = {
            description = "Find interactive Pi instances in tmux panes";
            mainProgram = "pifzf";
            platforms = [ "x86_64-linux" ];
          };
        };
      };
    };
}
