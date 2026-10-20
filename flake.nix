{
  description = "fsa - small real-time microphone processor (pipewire on linux, blackhole on macos)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAll = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # libraries egui/winit dlopen at runtime on linux
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
      packages = forAll (
        pkgs:
        let
          inherit (pkgs.stdenv.hostPlatform) isLinux isDarwin;
        in
        {
          default = pkgs.rustPlatform.buildRustPackage (finalAttrs: {
            pname = "fsa";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            src = lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [
              pkgs.pkg-config
            ]
            ++ lib.optionals isLinux [
              pkgs.rustPlatform.bindgenHook
              pkgs.makeWrapper
              pkgs.copyDesktopItems
            ];
            # app launcher entry; StartupWMClass matches the wayland app id
            desktopItems = lib.optionals isLinux [
              (pkgs.makeDesktopItem {
                name = "fsa";
                desktopName = "fsa";
                genericName = "Microphone Processor";
                comment = "Clean up your mic and send it to a virtual \"Mic (processed)\" input";
                exec = "fsa";
                icon = "audio-input-microphone";
                startupWMClass = "fsa";
                categories = [
                  "AudioVideo"
                  "Audio"
                ];
                keywords = [
                  "mic"
                  "microphone"
                  "eq"
                  "noise"
                ];
              })
            ];
            buildInputs = lib.optionals isLinux (
              with pkgs;
              [
                pipewire
                libx11 # plugin editor host windows
                lilv # lv2 hosting
                # lilv-0.pc lists these in Requires.private, which pkg-config still resolves
                serd
                sord
                sratom
                zix
              ]
            );
            postFixup =
              lib.optionalString isLinux ''
                wrapProgram $out/bin/fsa \
                  --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath (runtimeLibs pkgs)} \
                  --suffix LV2_PATH : ${pkgs.lv2}/lib/lv2
              ''
              + lib.optionalString isDarwin ''
                app=$out/Applications/fsa.app/Contents
                mkdir -p $app/MacOS
                mv $out/bin/fsa $app/MacOS/fsa
                ln -s $app/MacOS/fsa $out/bin/fsa
                # the plist also gives macos a reason to show when asking for the mic
                cp ${pkgs.replaceVars ./assets/macos/Info.plist { inherit (finalAttrs) version; }} $app/Info.plist
              '';
            meta = {
              description = "real-time microphone processor with a virtual mic output";
              mainProgram = "fsa";
              platforms = lib.platforms.linux ++ lib.platforms.darwin;
            };
          });
        }
        // lib.optionalAttrs isDarwin {
          # the virtual device fsa plays into on macos
          blackhole = pkgs.blackhole.override { channel = "2ch"; };
        }
      );

      devShells = forAll (pkgs: {
        default = pkgs.mkShell (
          {
            inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
            packages = with pkgs; [
              rustc
              cargo
              clippy
              rustfmt
              rust-analyzer
            ];
          }
          // lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            LD_LIBRARY_PATH = lib.makeLibraryPath (runtimeLibs pkgs);
            LV2_PATH = "${pkgs.lv2}/lib/lv2";
          }
        );
      });

      # nix-darwin: `imports = [ fsa.darwinModules.default ]; programs.fsa.enable = true;`
      darwinModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.fsa;
          driver = "BlackHole${cfg.blackhole.channel}.driver";
          src = "${cfg.blackhole.package}/Library/Audio/Plug-Ins/HAL/${driver}";
        in
        {
          options.programs.fsa = {
            enable = lib.mkEnableOption "fsa, a real-time microphone processor";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
              defaultText = lib.literalExpression "fsa.packages.\${system}.default";
              description = "the fsa package";
            };
            blackhole = {
              enable = lib.mkOption {
                type = lib.types.bool;
                default = true;
                description = ''
                  install the BlackHole virtual audio driver that fsa plays into.
                  coreaudiod only loads drivers from /Library/Audio/Plug-Ins/HAL, so
                  it is copied there on activation (and coreaudiod restarted when it
                  changes). disabling this does not remove an installed driver.
                '';
              };
              channel = lib.mkOption {
                type = lib.types.enum [
                  "2ch"
                  "16ch"
                  "64ch"
                  "128ch"
                  "256ch"
                ];
                default = "2ch";
                description = "BlackHole variant; discord lists it as \"BlackHole <channel>\"";
              };
              package = lib.mkOption {
                type = lib.types.package;
                default = pkgs.blackhole.override { inherit (cfg.blackhole) channel; };
                defaultText = lib.literalExpression ''pkgs.blackhole.override { channel = "2ch"; }'';
                description = "the BlackHole package";
              };
            };
          };

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ cfg.package ];
            system.activationScripts.postActivation.text = lib.mkIf cfg.blackhole.enable (
              lib.mkAfter ''
                # fsa: install the BlackHole driver into the system HAL folder
                dst="/Library/Audio/Plug-Ins/HAL/${driver}"
                # kept outside the bundle so the driver's code signature stays intact
                marker="/var/db/fsa-${driver}.nix-source"
                fsaFound=
                for fsaEntry in "${cfg.blackhole.package}/Library/Audio/Plug-Ins/HAL"/*.driver; do
                  if [ "''${fsaEntry##*/}" = "${driver}" ]; then fsaFound=1; fi
                done
                if [ -z "$fsaFound" ]; then
                  echo "fsa: ${driver} not found in ${cfg.blackhole.package}" >&2
                  exit 1
                fi
                if [ ! -d "$dst" ] || [ "$(cat "$marker" 2>/dev/null)" != "${src}" ]; then
                  echo "fsa: installing ${driver}"
                  mkdir -p /Library/Audio/Plug-Ins/HAL
                  rm -rf "$dst"
                  cp -R "${src}" "$dst"
                  chmod -R u+w "$dst"
                  chown -R root:wheel "$dst"
                  echo "${src}" > "$marker"
                  killall coreaudiod 2>/dev/null || true
                fi
              ''
            );
          };
        };
    };
}
