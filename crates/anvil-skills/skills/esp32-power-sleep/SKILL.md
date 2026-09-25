---
name: esp32-power-sleep
description: ESP32 sleep modes, ULP coprocessor, battery measurement and power budgeting. Use for battery or solar sensor nodes.
---

# ESP32 Power & Sleep

## Sleep mode picker

| Mode | Current | RAM | Wake sources |
| ---- | ------- | --- | ------------ |
| Modem-sleep (auto) | ~20 mA | kept | anytime |
| Light-sleep | ~0.8 mA | kept | GPIO, timer, UART |
| Deep-sleep | ~10 µA (S3 ~7 µA) | lost (RTC mem kept) | timer, touch, ULP, EXT0/1 |

Deep-sleep = reboot on wake. Keep wake counters in `RTC_DATA_ATTR` or NVS.

```c
RTC_DATA_ATTR int boot_count = 0;

void app_main(void) {
    ++boot_count;
    read_and_send();                       // do the whole job, then sleep
    esp_sleep_enable_timer_wakeup(10 * 60 * 1000000ULL);  // 10 min
    esp_deep_sleep_start();
}
```

## ULP (sense while the main cores sleep)

- S3/C3: **ULP-RISC-V** programmed in C via `ulp_riscv_` components.
- Classic: **ULP-FSM** in macro assembly.
- Pattern: ULP samples ADC/GPIO at 1–10 Hz into RTC_SLOW_MEM, wakes the
  main CPU only on threshold crossing. Ideal for door, leak, vibration.

## Battery measurement

```c
// Divider: VBAT --100k--+--100k-- GND, midpoint to ADC1 (11 dB atten)
int mv_at_pin;  // via calibrated ADC (skill: esp32-gpio-adc-pwm)
float vbat = mv_at_pin * 2.0f / 1000.0f;
```

Never exceed 3.3 V on the ADC pin — size the divider for max charge voltage
(4.2 V LiPo → ≥1.3:1 ratio). Add 100 nF at the pin.

## Power budget (do the math before ordering PCBs)

Example 10-min cycle node: 30 s active @ 120 mA + 570 s deep-sleep @ 10 µA
→ average ≈ 0.63 mA → 2000 mAh cell ≈ 130 days. WiFi TX spikes (~400 mA)
brown out weak regulators — bulk 470 µF + ceramic at the module, short fat
traces. Solar: panel must cover worst-week sun, not average.
