---
name: esp32-ota-debug
description: ESP32 OTA updates, partition tables, JTAG debugging, core dumps, watchdog and flash-size fixes. Use when shipping or debugging firmware.
---

# ESP32 OTA & Debug

## OTA that survives bad flashes

Use the factory + ota_0/ota_1 scheme (`partitions_two_ota.csv`). The golden
rule: **never mark the new image valid until it proves itself.**

```c
const esp_partition_t *running = esp_ota_get_running_partition();
esp_ota_img_states_t state;
esp_ota_get_state_partition(running, &state);
if (state == ESP_OTA_IMG_PENDING_VERIFY) {
    if (self_test_ok()) {          // sensors read, MQTT connects, heap sane
        esp_ota_mark_app_valid_cancel_rollback();
    } else {
        esp_ota_mark_app_invalid_rollback_and_reboot();
    }
}
```

Serve binaries over HTTPS with version + SHA in a manifest; devices poll,
download to ota_N, verify, reboot.

## Partitions & size

- `idf.py partition-table` shows the map; app > 1 MB needs a custom CSV.
- `idf.py size-components` / `size-files` finds the fat: mbedTLS (~300 KB),
  BT stack (~400 KB), debug strings. Trim via menuconfig (log level,
  disabled BT, `CONFIG_COMPILER_OPTIMIZATION_SIZE`).
- OTA needs 2x app slots + overhead — 4 MB flash minimum for OTA apps.

## Debugging

- **JTAG** (S3/C3 have USB-Serial-JTAG built in): `openocd -f
  board/esp32s3-builtin.cfg`, then `idf.py gdb`. Breakpoints, backtraces,
  FreeRTOS task lists — use it instead of printf archaeology.
- **Core dumps** to flash (`CONFIG_ESP_COREDUMP_ENABLE_TO_FLASH`) +
  `espcoredump.py info_corefile` decode the Guru Meditation offline.
- **Watchdogs**: task WDT fires on starved loops (`vTaskDelay` or
  `esp_task_wdt_reset` in long work); interrupt WDT = ISR or disabled
  interrupts too long. Feed deliberately, don't just disable.
- Decode panics: `idf.py monitor --decode-panic` resolves addresses to
  file:line automatically.
