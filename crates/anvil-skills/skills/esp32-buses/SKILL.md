---
name: esp32-buses
description: ESP32 I2C, SPI and UART driver setup, wiring, pull-ups and level shifting. Use when connecting sensors, displays, or modules.
---

# ESP32 Buses

## I2C (sensors, OLEDs, RTCs)

```c
i2c_master_bus_config_t bus = {.i2c_port = I2C_NUM_0,
    .sda_io_num = 8, .scl_io_num = 9,
    .clk_source = I2C_CLK_SRC_DEFAULT,
    .glitch_ignore_cnt = 7,
    .flags.enable_internal_pullup = true};
i2c_master_bus_handle_t h;
i2c_new_master_bus(&bus, &h);

i2c_device_config_t dev = {.dev_addr_length = I2C_ADDR_BIT_LEN_7,
    .device_address = 0x3C, .scl_speed_hz = 400000};
i2c_master_dev_handle_t oled;
i2c_master_bus_add_device(h, &dev, &oled);

uint8_t cmd[] = {0x00, 0xAE};  // example write
i2c_master_transmit(oled, cmd, sizeof cmd, pdMS_TO_TICKS(100));
```

Wiring rules: SDA/SCL need pull-ups (4.7k to 3.3V; internal pull-ups are
weak — add externals past ~10 cm or >1 device). Keep bus < 30 cm at
400 kHz. Scan for devices first (`i2cdetect`-style probe loop) before
debugging drivers.

## SPI (displays, SD cards, fast ADCs)

```c
spi_bus_config_t b = {.mosi_io_num = 11, .miso_io_num = 13, .sclk_io_num = 12,
    .quadwp_io_num = -1, .quadhd_io_num = -1, .max_transfer_sz = 4096};
spi_bus_initialize(SPI2_HOST, &b, SPI_DMA_CH_AUTO);
spi_device_interface_config_t d = {.clock_speed_hz = 20 * 1000 * 1000,
    .mode = 0, .spics_io_num = 10, .queue_size = 4};
spi_device_handle_t dev;
spi_bus_add_device(SPI2_HOST, &d, &dev);
```

Use DMA (`SPI_DMA_CH_AUTO`) for anything over ~1 KB or display framebuffers.
SPI1 (flash bus) is off-limits.

## UART (GPS, debug, modems)

```c
uart_config_t u = {.baud_rate = 9600, .data_bits = UART_DATA_8_BITS,
    .parity = UART_PARITY_DISABLE, .stop_bits = UART_STOP_BITS_1,
    .flow_ctrl = UART_HW_FLOWCTRL_DISABLE, .source_clk = UART_SCLK_DEFAULT};
uart_driver_install(UART_NUM_1, 2048, 0, 0, NULL, 0);
uart_param_config(UART_NUM_1, &u);
uart_set_pin(UART_NUM_1, 17, 18, UART_PIN_NO_CHANGE, UART_PIN_NO_CHANGE);
```

5V modules (many GPS/RS485 boards) TX into ESP32 RX through a divider or
shifter — never direct (skill: esp32-safety). USB Serial/JTAG on S3/C3 gives
a free second console via `esp_usb_serial`.
