{
  description = "open-conv — a convolution reverb past the limits of convolution (modulatable, level-gated IRs)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "aarch64-darwin" "x86_64-darwin" "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAll (pkgs: {
        # One shell carries everything: python lab, rust engine/cli, wrac plugin build.
        default = pkgs.mkShell {
          packages = [
            (pkgs.python3.withPackages (ps: with ps; [
              numpy
              scipy
              matplotlib
              soundfile
            ]))
            pkgs.ffmpeg
            pkgs.sox
            pkgs.cargo
            pkgs.rustc
            pkgs.libiconv
            # wrac stack: clap-wrapper (VST3/AU) build
            pkgs.nodejs_22
            pkgs.cmake
          ];
        };
      });
    };
}
