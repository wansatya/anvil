---
name: esp-idf-patterns
description: ESP-IDF C patterns — app_main, FreeRTOS tasks, logging, esp_err_t, Kconfig, NVS. Use when writing or reviewing ESP-IDF firmware.
---

# ESP-IDF Patterns

## app_main + tasks

`app_main` runs on core 0 with a ~8 KB stack — spawn workers, don't block it.

```c
static const char *TAG = "app";

void sensor_task(void *arg) {
    for (;;) {
        float t = read_temp();
        ESP_LOGI(TAG, "temp=%.2f", t);
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
}

void app_main(void) {
    // Pin timing-critical work; leave WiFi/IPC on core 0.
    xTaskCreatePinnedToCore(sensor_task, "sensor", 4096, NULL, 5, NULL, 1);
}
```

Stack sizes: 2048 absolute minimum, 4096 typical, 8192 for printf-heavy or
TLS code. Watch for `Stack canary watchpoint triggered` = grow the stack.

## Logging & errors

```c
ESP_LOGI(TAG, "..."); ESP_LOGW(TAG, "..."); ESP_LOGE(TAG, "...");
esp_err_t r = do_thing();
if (r != ESP_OK) {
    ESP_LOGE(TAG, "do_thing: %s", esp_err_to_name(r));
    // Decide: retry with backoff, ESP_ERROR_CHECK (dev only), or goto cleanup.
}
```

`ESP_ERROR_CHECK` reboots on failure — fine during bring-up, never in
release paths that touch the network or flash wear.

## Kconfig (never hardcode board constants)

```c
// Kconfig.projbuild:
config SAMPLE_PERIOD_MS
    int "Sampling period"
    default 1000
// usage:
vTaskDelay(pdMS_TO_TICKS(CONFIG_SAMPLE_PERIOD_MS));
```

## NVS (small key-value, not a database)

```c
nvs_handle_t h;
nvs_open("cal", NVS_READWRITE, &h);
int32_t offset = 0;
nvs_get_i32(h, "offset", &offset);   // missing key: keep default, don't crash
nvs_set_i32(h, "offset", offset + 1);
nvs_commit(h);
nvs_close(h);
```

NVS writes wear flash (~100k cycles/namespace page) — cache in RAM and
commit on change or every N minutes, not every sample.
