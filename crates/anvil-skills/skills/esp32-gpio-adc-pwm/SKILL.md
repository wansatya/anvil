---
name: esp32-gpio-adc-pwm
description: ESP32 GPIO, ADC, LEDC/PWM and RMT with snippets and hardware pitfalls (strapping pins, ADC attenuation, DAC removal). Use for sensor/actuator I/O.
---

# ESP32 GPIO / ADC / PWM

## GPIO rules that bite

- **Strapping pins** (boot mode): GPIO0, GPIO3(RX), GPIO45/46 (S3), GPIO8/9
  (C3). Never tie them to a sensor that pulls up/down at reset, or the chip
  won't boot. Check the datasheet "Strapping Pins" table for your variant.
- 3.3V logic only — 5V sensors need dividers or level shifters
  (skill: esp32-safety).
- ESP32-S3/C3/C6 have **no DAC** (classic ESP32 has 2x 8-bit DAC). Need
  analog out? Use LEDC + RC filter or an external DAC (MCP4725).

```c
gpio_config_t io = {
    .pin_bit_mask = 1ULL << GPIO_NUM_4,
    .mode = GPIO_MODE_INPUT,
    .pull_up_en = GPIO_PULLUP_ENABLE,
    .intr_type = GPIO_INTR_NEGEDGE,
};
gpio_config(&io);
```

## ADC (oneshot + calibration — attenuation matters!)

Default 0 dB attenuation reads only ~0–0.8 V. For full 0–3.3 V range use
11 dB attenuation **and** line-fitting calibration.

```c
adc_oneshot_unit_handle_t adc;
adc_oneshot_unit_init_cfg_t u = {.unit_id = ADC_UNIT_1};
adc_oneshot_new_unit(&u, &adc);
adc_oneshot_chan_cfg_t c = {.atten = ADC_ATTEN_DB_11, .bitwidth = ADC_BITWIDTH_12};
adc_oneshot_config_channel(adc, ADC_CHANNEL_3, &c);

adc_cali_handle_t cali;
adc_cali_line_fitting_config_t cal = {
    .unit_id = ADC_UNIT_1, .atten = ADC_ATTEN_DB_11, .bitwidth = ADC_BITWIDTH_12 };
adc_cali_create_scheme_line_fitting(&cal, &cali);

int raw, mv;
adc_oneshot_read(adc, ADC_CHANNEL_3, &raw);
adc_cali_raw_to_voltage(cali, raw, &mv);
```

ADC2 is unusable while WiFi runs (shared hardware) — route analog inputs to
ADC1 on WiFi projects.

## LEDC (PWM) + RMT

```c
ledc_timer_config_t t = {.speed_mode = LEDC_LOW_SPEED_MODE, .duty_resolution = LEDC_TIMER_10_BIT,
    .timer_num = LEDC_TIMER_0, .freq_hz = 5000, .clk_cfg = LEDC_AUTO_CLK};
ledc_timer_config(&t);
ledc_channel_config_t ch = {.gpio_num = 5, .speed_mode = LEDC_LOW_SPEED_MODE,
    .channel = LEDC_CHANNEL_0, .timer_sel = LEDC_TIMER_0, .duty = 512};
ledc_channel_config(&ch);
```

For servos/WS2812/IR, prefer **RMT** over bit-banging — timing-accurate in
hardware, zero CPU jitter. WS2812: use the `led_strip` component, not manual RMT.
