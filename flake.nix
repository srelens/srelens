{
  description = "srelens — Kubernetes workspace, built from source";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));

      # One version, the workspace's. A literal here would drift from Cargo.toml.
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

      packageManager = (builtins.fromJSON (builtins.readFile ./package.json)).packageManager;
      pnpmVersion = builtins.elemAt (builtins.match "pnpm@([^+]+)(\\+.*)?" packageManager) 0;
      pnpmMajor = nixpkgs.lib.versions.major pnpmVersion;
      pnpmFor =
        pkgs:
        (builtins.getAttr "pnpm_${pnpmMajor}" pkgs).override {
          version = pnpmVersion;
          hash = "sha256-0eqyQzFyZhzDahjshfzpP3cdsZYnFzKcwB7JwoJMok8=";
        };

      # Cargo.lock vendors a git crate (fix-path-env) with no checksum. importCargoLock
      # refuses that unless we name the hash. packaging/nix/flake.test.mjs fails when a
      # new git source appears here without an entry.
      cargoLock = {
        lockFile = ./Cargo.lock;
        outputHashes = {
          "fix-path-env-0.0.0" = "sha256-UygkxJZoiJlsgp8PLf1zaSVsJZx1GGdQyTXqaFv3oGk=";
        };
      };

      metaFor = pkgs: mainProgram: description: {
        inherit description;
        homepage = "https://github.com/srelens/srelens";
        license = pkgs.lib.licenses.mit;
        inherit mainProgram;
        platforms = pkgs.lib.platforms.linux ++ pkgs.lib.platforms.darwin;
      };
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          cargoDeps = pkgs.rustPlatform.importCargoLock cargoLock;

          # Use the exact packageManager version so Nix reads the same lockfile
          # configuration and overrides as the other install paths.
          pnpm = pnpmFor pkgs;
          nodejs = pkgs.nodejs_26;

          # Release builds of the desktop binary and srelens-server embed this
          # directory. tauri-build and rust-embed both refuse to compile without it.
          frontend = pkgs.stdenv.mkDerivation {
            pname = "srelens-frontend";
            inherit version;
            src = pkgs.lib.cleanSource ./.;

            nativeBuildInputs = [
              nodejs
              pnpm
              pkgs.pnpmConfigHook
            ];

            pnpmDeps = pkgs.fetchPnpmDeps {
              pname = "srelens-frontend";
              inherit version pnpm;
              src = pkgs.lib.cleanSource ./.;
              fetcherVersion = 4;
              hash = "sha256-LEOYY4GaXq0MMqHto7qb7XtQjbuYi9B48H51lAuXj54=";
            };

            buildPhase = ''
              runHook preBuild
              pnpm --filter @srelens/desktop build
              runHook postBuild
            '';

            installPhase = ''
              runHook preInstall
              mkdir -p "$out"
              cp -R apps/desktop/dist/. "$out"/
              runHook postInstall
            '';
          };

          common = {
            inherit version cargoDeps;
            src = pkgs.lib.cleanSource ./.;

            # The workspace suite is `cargo test --workspace` in CI. A package
            # install should not depend on a cluster, and a large part of that
            # suite does.
            doCheck = false;

            nativeBuildInputs = [ pkgs.pkg-config ];

            buildInputs = [
              pkgs.sqlite
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
          };

          srectl = pkgs.rustPlatform.buildRustPackage (
            common
            // {
              pname = "srectl";

              cargoBuildFlags = [
                "-p"
                "srectl"
                "--bin"
                "srectl"
              ];

              # Homebrew leaves this name behind as a wrapper. A package manager
              # installs both; the binary itself will not, because it refuses to
              # rewrite a file under /nix/store.
              postInstall = ''
                cat > "$out/bin/srelens-tui" <<EOF
                #!${pkgs.runtimeShell}
                [ -t 2 ] && echo 'srelens-tui is now srectl' >&2
                exec "$out/bin/srectl" "\$@"
                EOF
                chmod 0755 "$out/bin/srelens-tui"
              '';

              meta = metaFor pkgs "srectl" "Kubernetes control room in your terminal";
            }
          );

          # The GUI libraries the desktop binary links on Linux. WebKit comes from
          # the host SDK on Darwin, the same way the .dmg does.
          linuxGui = with pkgs; [
            webkitgtk_4_1
            gtk3
            libsoup_3
            librsvg
            libayatana-appindicator
            openssl
            xdotool
          ];

          withFrontend = ''
            rm -rf apps/desktop/dist
            mkdir -p apps/desktop/dist
            cp -R ${frontend}/. apps/desktop/dist/
          '';

          srelens-server = pkgs.rustPlatform.buildRustPackage (
            common
            // {
              pname = "srelens-server";

              cargoBuildFlags = [
                "-p"
                "srelens-server"
                "--bin"
                "srelens-server"
              ];

              preBuild = withFrontend;

              meta = metaFor pkgs "srelens-server" "Headless srelens web server";
            }
          );

          # A nix path cannot contain `@`, so the @2x icon is reached from this directory.
          icons = ./apps/desktop/src-tauri/icons;

          desktopItem = pkgs.makeDesktopItem {
            name = "srelens";
            desktopName = "srelens";
            genericName = "Kubernetes Workspace";
            comment = "Kubernetes desktop workspace";
            exec = "srelens %u";
            icon = "srelens";
            terminal = false;
            categories = [ "Development" ];
            keywords = [
              "kubernetes"
              "k8s"
              "kubectl"
              "cluster"
              "container"
              "devops"
              "sre"
              "helm"
            ];
            startupWMClass = "srelens";
            mimeTypes = [ "x-scheme-handler/srelens" ];
          };

          srelens = pkgs.rustPlatform.buildRustPackage (
            common
            // {
              pname = "srelens";

              nativeBuildInputs =
                common.nativeBuildInputs
                ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.wrapGAppsHook3 ];

              buildInputs = common.buildInputs ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux linuxGui;

              # Embeds the frontend instead of opening the Vite dev server.
              cargoBuildFeatures = [ "custom-protocol" ];

              cargoBuildFlags = [
                "-p"
                "srelens-desktop"
                "-p"
                "srelens-plugin-host"
                "--bin"
                "srelens"
                "--bin"
                "srelens-sandbox-launch"
              ];

              preBuild = withFrontend;

              postInstall = ''
                install -Dm644 ${icons}/32x32.png \
                  "$out/share/icons/hicolor/32x32/apps/srelens.png"
                install -Dm644 ${icons}/64x64.png \
                  "$out/share/icons/hicolor/64x64/apps/srelens.png"
                install -Dm644 ${icons}/128x128.png \
                  "$out/share/icons/hicolor/128x128/apps/srelens.png"
                install -Dm644 ${icons}/128x128@2x.png \
                  "$out/share/icons/hicolor/256x256/apps/srelens.png"
                mkdir -p "$out/share/applications"
                cp -R ${desktopItem}/share/applications/. "$out/share/applications/"
              '';

              meta = metaFor pkgs "srelens" "Kubernetes desktop workspace";
            }
          );
        in
        {
          inherit
            srectl
            srelens
            srelens-server
            ;
          # The terminal UI is the default: one binary, no WebKit, and `nix run`
          # has somewhere to go without a display. The desktop app is `#srelens`.
          default = srectl;
        }
      );

      apps = forAllSystems (
        pkgs:
        let
          packages = self.packages.${pkgs.stdenv.hostPlatform.system};
          app = program: {
            type = "app";
            inherit program;
          };
        in
        {
          srectl = app "${packages.srectl}/bin/srectl";
          srelens = app "${packages.srelens}/bin/srelens";
          srelens-server = app "${packages.srelens-server}/bin/srelens-server";
          default = app "${packages.srectl}/bin/srectl";
        }
      );

      devShells = forAllSystems (
        pkgs:
        let
          packages = self.packages.${pkgs.stdenv.hostPlatform.system};
        in
        {
          # inputsFrom the desktop package so a Linux shell has WebKit and GTK
          # on the pkg-config path. `cargo build -p srectl` does not need them.
          default = pkgs.mkShell {
            inputsFrom = [ packages.srelens ];
            packages = [
              pkgs.rustc
              pkgs.cargo
              pkgs.clippy
              pkgs.nodejs_26
              (pnpmFor pkgs)
              pkgs.pkg-config
            ];
          };
        }
      );

      overlays.default = final: _prev: {
        inherit (self.packages.${final.stdenv.hostPlatform.system})
          srectl
          srelens
          srelens-server
          ;
      };
    };
}
