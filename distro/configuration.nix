{ config, lib, pkgs, ... }: {
  imports = [ ./overlays/default.nix ];

  options = {
    splash = lib.mkOption {
      type = lib.types.path;
      default = ../../../../../distro/splash.png;
    };
    kernel = lib.mkOption {
      type = lib.types.path;
      default = ../../../../../target/osdk/iso_root/boot/aster-nix-osdk-bin;
    };
    initramfs = lib.mkOption {
      type = lib.types.path;
      default = pkgs.makeInitrd {
        compressor = "cat";
        contents = [
          {
            object = "${pkgs.busybox}/bin";
            symlink = "/bin";
          }
          {
            object = "${config.stage-1-init}";
            symlink = "/init";
          }
        ];
      };
    };
    stage-1-init = lib.mkOption {
      type = lib.types.path;
      default = ../../../../../tools/nixos/stage-1-init.sh;
    };
    break-into-stage1-shell = lib.mkOption {
      type = lib.types.str;
      default = "0";
      description = ''
        If set to "1", the system will not proceed to switch to the root filesystem after
        initial boot. Instead, it will drop into an initramfs shell. This is primarily
        intended for debugging purposes.
      '';
    };
    resolv-conf = lib.mkOption {
      type = lib.types.path;
      default = ../../../../../target/nixos/resolv.conf;
    };
  };

  config = {
    boot.loader.grub.enable = true;
    boot.loader.grub.efiSupport = true;
    boot.loader.grub.device = "nodev";
    boot.loader.grub.efiInstallAsRemovable = true;
    boot.loader.grub.splashImage = config.splash;

    boot.initrd.enable = false;
    boot.kernel.enable = false;
    boot.postBootCommands = ''
      echo "Executing postBootCommands..."
      cp -L ${config.resolv-conf} /etc/resolv.conf
      PATH=$PATH:/nix/var/nix/profiles/system/sw/bin:~/.nix-profile/bin
      ${pkgs.bash}/bin/sh
    '';
    system.systemBuilderCommands = ''
      echo "PATH=/bin:/nix/var/nix/profiles/system/sw/bin ostd.log_level=${
        builtins.getEnv "LOG_LEVEL"
      } -- sh /init root=/dev/vda2 init=${
        builtins.getEnv "NIXOS_STAGE_2_INIT"
      } rd.break=${config.break-into-stage1-shell} ${
        builtins.getEnv "NIXOS_STAGE_2_ARGS"
      }" > $out/kernel-params
      rm -rf $out/init
      ln -s /bin/busybox $out/init
      ln -s ${config.kernel} $out/kernel
      ln -s ${config.initramfs}/initrd $out/initrd
    '';

    nix.settings = {
      filter-syscalls = false;
      require-sigs = false;
      sandbox = false;
    };

    systemd.enableCgroupAccounting = false;

    environment.defaultPackages = [ pkgs.hello-asterinas ];

    system.nixos.distroName = "Asterinas";

    system.stateVersion = "25.05";
  };
}
