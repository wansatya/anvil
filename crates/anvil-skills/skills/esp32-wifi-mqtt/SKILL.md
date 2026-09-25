---
name: esp32-wifi-mqtt
description: ESP32 WiFi station with robust reconnect, MQTT over TLS, offline buffering and provisioning. Use for any cloud-connected node.
---

# ESP32 WiFi + MQTT

## WiFi station that heals itself

```c
static void wifi_event(void *a, esp_event_base_t b, int32_t id, void *d) {
    if (id == WIFI_EVENT_STA_DISCONNECTED) {
        esp_wifi_connect();  // immediate retry; add backoff counter for production
    } else if (id == IP_EVENT_STA_GOT_IP) {
        xEventGroupSetBits(net_ev, GOT_IP);
    }
}

wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
esp_wifi_init(&cfg);
esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, wifi_event, NULL);
esp_event_handler_register(IP_EVENT, IP_EVENT_STA_GOT_IP, wifi_event, NULL);
wifi_config_t wc = {.sta = {.ssid = WIFI_SSID, .password = WIFI_PASS,
    .threshold.authmode = WIFI_AUTH_WPA2_PSK}};
esp_wifi_set_mode(WIFI_MODE_STA);
esp_wifi_set_config(WIFI_IF_STA, &wc);
esp_wifi_start();
esp_wifi_connect();
```

Production upgrades: exponential backoff, reboot after N consecutive
failures, `esp_wifi_set_ps(WIFI_PS_MAX_MODEM)` for battery nodes.

## MQTT over TLS (never plaintext credentials)

```c
esp_mqtt_client_config_t m = {
    .broker.address.uri = "mqtts://broker.example:8883",
    .broker.verification.certificate = (const char *)ca_pem_start,
    .credentials.client_id = "sensor-01",
    .credentials.username = "node",
    .credentials.authentication.password = MQTT_PASS,
    .session.keepalive = 30,
    .network.reconnect_timeout_ms = 5000,
};
esp_mqtt_client_handle_t c = esp_mqtt_client_init(&m);
esp_mqtt_client_start(c);
esp_mqtt_client_publish(c, "nodes/sensor-01/temp", payload, 0, 1, 0);
```

Embed the CA with `EMBED_TXTFILES`. QoS 1 for commands/alerts, QoS 0 for
high-rate telemetry. Last-Will (`lwt_*`) on `nodes/<id>/status` =
instant offline detection server-side.

## Offline buffering + provisioning

- Buffer unsent samples in a FreeRTOS queue → spill to NVS/SPIFFS with a
  ring file; replay oldest-first on reconnect, drop oldest on overflow.
- Provisioning: BLE (`blufi`) or SoftAP captive portal for STA credentials;
  never hardcode home WiFi in firmware you share.
- Time: `sntp` once online; sensor timestamps without NTP drift and break
  every cloud chart.
