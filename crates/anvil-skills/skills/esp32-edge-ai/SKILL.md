---
name: esp32-edge-ai
description: On-device ML on ESP32 — sensor DSP pipeline, ESP-DL and TinyML deployment, quantization, and the cloud-training loop. Use for physical AI features.
---

# ESP32 Edge AI (Physical AI)

## The loop: sense → DSP → infer → act → telemetry

Physical AI lives or dies on the **signal pipeline**, not the model:

1. Sample deterministically (timer ISR or I2S/DMA, fixed rate, no jitter).
2. Preprocess on-device: DC removal, low-pass/biquad, RMS/FFT features.
   A good FFT feature beats a bigger model every time.
3. Infer at the duty cycle you can afford (see budget below).
4. Act locally (relay, LED, BLE alert) — cloud is for logging, not control.
5. Stream hard cases back as telemetry to retrain (skill: esp32-wifi-mqtt).

## Deployment options

| Path | Best for | Notes |
| ---- | -------- | ----- |
| **ESP-DL** (Espressif) | S3 vision/speech, INT8 | uses S3 vector insns; fastest on S3 |
| **TFLite Micro** | portable anomaly/keyword models | arena in PSRAM; measure per-op latency |
| **Classical DSP** | thresholds, FFT bands, peak detect | often beats ML; start here, add ML only with data |

Quantize to INT8 (post-training quant, representative dataset from the
**actual sensor on the actual mount**). Validate on-device accuracy —
desktop accuracy lies when clocks/ADC noise differ.

## Budgets that matter

- S3 @ 240 MHz: ~10–50 ms for small keyword/vision models INT8; classic
  ESP32 roughly 3–5x slower — pick S3 for ML (skill: esp32-start).
- PSRAM is slow-ish: keep hot tensors in internal RAM, arena in PSRAM.
- Power: inference at 1 Hz can dominate a battery budget — duty-cycle it
  (wake → sample → infer → sleep, skill: esp32-power-sleep).

## Starter projects

1. **Vibration anomaly**: accelerometer → RMS + FFT bands → threshold, then
   graduate to autoencoder; telemetry uploads only anomalies.
2. **Keyword spot**: INMP441 I2S mic → MFCC → tiny conv model → relay/LED.
3. **People counting**: S3 + OV2640 → ESP-DL pedestrian detection at
   2–5 fps, MQTT counts only (never stream video over WiFi on battery).
