{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.car-dsp;
in
{
  options.car-dsp = {
    enable = lib.mkEnableOption "Enable Yet Another Car DSP";
  };

  config = lib.mkIf cfg.enable {
    # This assumes the package is available in 'pkgs' via an overlay
    environment.systemPackages = [ pkgs.yet-another-car-dsp ];
  };
}
