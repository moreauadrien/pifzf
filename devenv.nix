{ pkgs, ... }: {
  packages = with pkgs; [ rustc cargo tmux fzf nodejs ];
}
