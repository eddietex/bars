#!/bin/sh
# Grants the logged-in user access to keyboards + /dev/uinput via systemd's
# "uaccess" (no group changes, no reboot). Run as root: pkexec/sudo.
set -eu
here=$(dirname "$(readlink -f "$0")")
install -m 0644 "$here/../packaging/linux/70-bars.rules" /etc/udev/rules.d/70-bars.rules
udevadm control --reload-rules
udevadm trigger --action=change --subsystem-match=input --subsystem-match=misc
echo "Installed /etc/udev/rules.d/70-bars.rules"
