{
  description = "fsa - small real-time microphone processor for pipewire";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # libraries egui/winit dlopen at runtime
      runtimeLibs =
        pkgs: with pkgs; [
          wayland
          libxkbcommon
          libGL
          vulkan-loader
          # x11 fallback, also needed by plugin editors running through xwayland
          libx11
          libxcursor
          libxi
          libxrandr
        ];
    in
    {
      packages = forAll (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "fsa";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };
          nativeBuildInputs = with pkgs; [
            pkg-config
            rustPlatform.bindgenHook
            makeWrapper
          ];
          buildInputs = [ pkgs.pipewire ];
          postFixup = ''
            wrapProgram $out/bin/fsa \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}
          '';
          meta = {
            description = "real-time microphone processor exposing a virtual pipewire source";
            mainProgram = "fsa";
            platforms = pkgs.lib.platforms.linux;
          };
        };
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
          ];
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);
        };
      });
    };
}
