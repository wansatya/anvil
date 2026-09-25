---
name: esp32-start
description: ESP32 series overview, chip picker, and toolchain setup (ESP-IDF, Arduino, PlatformIO). Start here for any ESP32 hardware task.
---

# ESP32 Start

## Series picker (2026)

| Chip | Core / Radio | Pick when |
| ---- | ------------ | --------- |
| ESP32 (classic) | 2x LX6, WiFi4+BT | cheap, max library support, dual-core headroom |
| ESP32-S3 | 2x LX7, WiFi4+BT5, USB-OTG, vector insns | camera, ML inference, USB HID |
| ESP32-C3 | 1x RISC-V, WiFi4+BT5 | low cost, simple sensor nodes |
| ESP32-C6 | 1x RISC-V, WiFi6+BT5+802.15.4 | Thread/Zigbee/Matter, future-proof node |
| ESP32-H2 | 1x RISC-V, 802.15.4+BT5 | Zigbee/Thread end-device (no WiFi) |

Rule of thumb: S3 for anything with a camera, display, or on-device ML;
C6 for new low-power wireless sensor designs; classic ESP32 when a tutorial
or library only targets it.

## Toolchains (pick one per project)

1. **ESP-IDF (C, official)** — full control, best power/net features.
   `git clone esp-idf; ./install.sh esp32s3; . ./export.sh`
2. **Arduino core** — fastest for C++ hackers; `board = esp32-s3-devkitc-1`.
   Good libraries, weaker deep-sleep/ULP APIs.
3. **PlatformIO** — `platform = espressif32`, per-env `board_build.*` flags.
   Best for multi-board CI.

## Flash & monitor essentials

```bash
esptool.py --chip esp32s3 --port /dev/ttyACM0 write_flash -z 0x0 firmware.bin
idf.py -p /dev/ttyACM0 flash monitor        # Ctrl+] exits monitor
idf.py menuconfig                            # partition, PSRAM, log level
```

Monitor baud default 115200. "No serial data received" almost always means:
wrong port, USB cable is charge-only, or GPIO0/EN strapping held at boot —
see `esp32-hacks`... (skill: esp32-ota-debug).

## First-boot checklist

1. Blink an LED to prove toolchain + flash + boot mode.
2. Print `esp_get_free_heap_size()` and reset reason
   (`esp_reset_reason()`) over serial.
3. Measure 3.3V rail under WiFi TX load before adding sensors —
   brownouts cause random reboots (skill: esp32-safety).
4. Commit a known-good `sdkconfig.defaults` to version control.
