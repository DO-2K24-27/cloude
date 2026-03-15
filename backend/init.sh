#!/bin/sh
set -eux

# Mount essential pseudo-filesystems (if not already mounted)
mount -t proc proc /proc || true
mount -t sysfs sys /sys || true
mount -t devtmpfs devtmpfs /dev || true
mount -t tmpfs tmpfs /run || true

echo "[initramfs] booting..."

# If cloude-agentd was injected at /usr/bin/cloude-agentd:
if [ -x /usr/bin/cloude-agentd ]; then
  echo "[initramfs] starting cloude-agentd"
  exec /usr/bin/cloude-agentd
else
  echo "[initramfs] ERROR: /usr/bin/cloude-agentd not found or not executable"
  /bin/sh
fi
