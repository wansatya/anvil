---
name: esp32-safety
description: Electrical safety for ESP32 hardware — mains relays, level shifting, LiPo/battery rules, ESD and fusing. Consult before any mains or battery design.
---

# ESP32 Electrical Safety

> These are engineering guidelines, not certification advice. Mains work
> must follow your local electrical code; when in doubt, use pre-certified
> modules (Shelly, Sonoff) instead of DIY mains boards.

## Mains (230/120 VAC)

- Prefer pre-certified relay/contactor modules over bare relays on custom
  PCB for first builds.
- DIY rules: ≥3 mm creepage line↔neutral/earth slots, fuse the live side
  (slow-blow, rated below trace capacity), snubber or zero-cross SSR for
  inductive loads (pumps, transformers), enclose everything — no exposed
  mains on a bench devkit.
- Never route mains and 3.3V logic in the same connector/cable bundle.

## Logic levels & power

- ESP32 GPIO is **3.3V, not 5V-tolerant**. 5V sensor TX → divider
  (e.g. 1k8/3k3) or TXB0108/BSS138 shifter; 5V I2C bus → shifter, never
  "it worked on my Arduino".
- Brownout = #1 cause of "random" reboots: 3.3V regulator ≥600 mA for WiFi,
  470 µF bulk + 100 nF ceramic at the module, short thick power traces.
- USB back-power: devkits powered from both USB and external 5V can fight
  — use a diode or single source during flashing.

## Batteries (LiPo/Li-ion)

- Charge only with a proper charger IC (TP4056/MCP73831) with NTC; never
  trickle-charge, never charge unattended on a bench.
- Protect every pack: DW01+8205A (over-charge/discharge/short) at minimum.
- Size ADC dividers for 4.2 V max (skill: esp32-gpio-adc-pwm); a divider
  that reads 3.3 V nominal will over-voltage the pin at full charge.
- Punctured/swollen pack = sand bucket, outside, now. No exceptions.

## ESD & handling

Ground yourself before handling bare modules in dry environments; keep one
hand in pocket near mains; fuse bench supplies (current-limit them to the
expected draw + margin).
