{
  # Why a flake rather than a downloadable installer: a .deb or an AppImage carries
  # the glibc and the WebKitGTK of the machine it was built on, and on NixOS neither
  # is where the binary expects it. Building from source against this machine's own
  # nixpkgs is what makes the app run, so the release for NixOS is a git tag:
  #
  #   nix profile install github:JanVDB2000/symfo-linker-profiler/v0.5.0
  #
  # and for working on it:
  #
  #   nix develop        # then npm ci && npm run desktop
  description = "Switch Composer dependencies between the vendor version and a local checkout";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # Read from Cargo.toml so this is not a fourth place to bump a version. The
        # other three are package.json, src-tauri/Cargo.toml and tauri.conf.json.
        version = (builtins.fromTOML (builtins.readFile ./src-tauri/Cargo.toml)).package.version;

        # What the webview actually links against. Deliberately not the list the
        # release workflow installs on its Ubuntu runner: that one also carries
        # libxdo, OpenSSL and an appindicator, and none of libxdo-sys, openssl-sys or
        # a tray crate appears in Cargo.lock. rfd resolves against gtk-sys rather than
        # ashpd, so the directory picker needs GTK3 but no xdg-desktop-portal.
        libraries = with pkgs; [
          webkitgtk_4_1
          gtk3
          libsoup_3
          glib
          cairo
          pango
          gdk-pixbuf
          atk
          # TLS inside the webview. Omitting it is the classic mistake: everything
          # builds, and then nothing can reach an https URL at runtime.
          glib-networking
        ];

        # The frontend is a separate derivation because tauri.conf.json's
        # beforeBuildCommand would otherwise run `npm run build` - and an npm install
        # cannot reach the network inside the Nix sandbox. Built here, handed over as
        # a directory below.
        frontend = pkgs.buildNpmPackage {
          pname = "symfolinker-frontend";
          inherit version;
          src = ./.;

          # Reads package-lock.json directly, so there is no hash to regenerate every
          # time a dependency moves. If the pinned nixpkgs turns out to be too old to
          # have importNpmLock, replace these two lines with
          #   npmDepsHash = "sha256-...";
          # obtained from: nix run nixpkgs#prefetch-npm-deps -- package-lock.json
          npmDeps = pkgs.importNpmLock { npmRoot = ./.; };
          inherit (pkgs.importNpmLock) npmConfigHook;

          # vue-tsc then vite, exactly as `npm run build` does.
          buildPhase = ''
            runHook preBuild
            npm run build
            runHook postBuild
          '';

          installPhase = ''
            runHook preInstall
            cp -r dist $out
            runHook postInstall
          '';
        };
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "symfolinker";
          inherit version;
          src = ./.;

          cargoRoot = "src-tauri";
          buildAndTestSubdir = "src-tauri";
          # Committed, lockfile v4, no git sources - so no outputHashes are needed.
          cargoLock.lockFile = ./src-tauri/Cargo.lock;

          # Mandatory: the binary target is declared with
          # required-features = ["desktop"], so a plain build matches no target and
          # silently produces nothing at all.
          buildFeatures = [ "desktop" ];

          nativeBuildInputs = with pkgs; [
            pkg-config
            # GTK3, per the gtk 0.18 generation in Cargo.lock - not wrapGAppsHook4.
            wrapGAppsHook3
            makeWrapper
            copyDesktopItems
          ];
          buildInputs = libraries;

          # Scanner tests create a repository; runtime PATH wrapping happens later.
          nativeCheckInputs = [ pkgs.git ];

          # tauri::generate_context! embeds frontendDist at compile time, so ../dist
          # has to exist beside src-tauri before cargo runs. Neutralising
          # beforeBuildCommand is what stops Tauri building it a second time.
          postPatch = ''
            cp -r ${frontend} dist
            substituteInPlace src-tauri/tauri.conf.json \
              --replace-fail '"beforeBuildCommand": "npm run build"' '"beforeBuildCommand": ""'
          '';

          # Every external tool is found by PATH lookup and there are no absolute
          # paths in the production code, so wrapping the PATH is all that is needed.
          # git and xdg-utils belong to the app: without them the Git panel and the
          # file-manager button go quiet. docker, podman, php and composer are
          # deliberately left out - those are the user's own toolchain, and pinning a
          # version of them here would be this app choosing which PHP you develop on.
          postFixup = ''
            wrapProgram $out/bin/symfolinker \
              --suffix PATH : ${pkgs.lib.makeBinPath (with pkgs; [ git xdg-utils ])}
          '';

          desktopItems = [
            (pkgs.makeDesktopItem {
              name = "symfolinker";
              exec = "symfolinker";
              icon = "symfolinker";
              desktopName = "SymfoLinker";
              comment = "Switch Composer dependencies between vendor and a local checkout";
              categories = [ "Development" ];
            })
          ];

          postInstall = ''
            install -Dm444 src-tauri/icons/128x128.png \
              $out/share/icons/hicolor/128x128/apps/symfolinker.png
            install -Dm444 src-tauri/icons/32x32.png \
              $out/share/icons/hicolor/32x32/apps/symfolinker.png
          '';

          # Tells the running app it sits in a read-only store, so the update check
          # offers the Nix command instead of a self-update it cannot perform.
          SYMFOLINKER_UPDATE_CHANNEL = "nix";

          meta = with pkgs.lib; {
            description = "Switch Composer dependencies between the vendor version and a local checkout";
            homepage = "https://github.com/JanVDB2000/symfo-linker-profiler";
            license = licenses.mit;
            mainProgram = "symfolinker";
            platforms = platforms.linux;
          };
        };

        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/symfolinker";
        };

        # Everything the package needs, plus the toolchain, so `nix develop` followed
        # by `npm ci && npm run desktop` works on a machine with nothing installed.
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            # 22.12 or newer: the translation tests run node --test with
            # --experimental-test-isolation=none.
            nodejs_22
            pkg-config
            git
          ] ++ libraries;

          # Without this the webview starts but cannot complete a TLS handshake, which
          # looks like a broken network rather than a missing module.
          shellHook = ''
            export GIO_EXTRA_MODULES="${pkgs.glib-networking}/lib/gio/modules"
          '';
        };

        formatter = pkgs.nixpkgs-fmt;
      });
}
