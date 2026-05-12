{
  description = "Yet Another Car DSP development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

      # Define the package once so it can be shared between 'packages' and 'overlays'
      carDspPkg = pkgs.rustPlatform.buildRustPackage {
        pname = "yet-another-car-dsp";
        version = "0.1.0";
        src = ./.;
        cargoHash = "sha256-lsDvQAwW6ANkfA9xsiRM04zowmnaRDz/oqYons5ZZic=";
        # Use empty for nix build to get current value
        # cargoHash = "";
        nativeBuildInputs = with pkgs; [ pkg-config ];
        buildInputs = with pkgs; [
          gtk4
          libadwaita
          glib
          jack2
          pango
          cairo
          gdk-pixbuf
        ];

        postInstall = ''
          mkdir -p $out/share/applications
          cp com.asiantuntija.yacd.desktop $out/share/applications/
        '';
      };
    in
    {
      packages = {
        ${system}.default = carDspPkg;
      };

      # This allows the package to be added to 'pkgs' globally
      overlays = {
        default = final: super: {
          "yet-another-car-dsp" = carDspPkg;
        };
      };

      # Export the module we created in module.nix
      nixosModules.default = import ./module.nix;

      devShells = {
        ${system}.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            pkg-config
            rustc
            cargo
          ];

          buildInputs = with pkgs; [
            # GTK4 and its dependencies
            gtk4
            glib
            pango
            cairo
            gdk-pixbuf
            libadwaita
          ];

          shellHook = ''
            export XDG_DATA_DIRS="${pkgs.gtk4}/share:${pkgs.libadwaita}/share:$XDG_DATA_DIRS"
            echo "Car DSP Dev Shell Loaded (Unstable)"
            echo "Rust version: $(rustc --version)"
          '';
        };
      };
    };
}
