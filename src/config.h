// Hardware + behaviour tuning
#pragma once
#include <Arduino.h>

// ---- Pins -----------------------------------------------------------------
constexpr uint8_t I2C0_SDA = 12, I2C0_SCL = 13;  // OLED + right wheel
constexpr uint8_t I2C1_SDA = 14, I2C1_SCL = 15;  // left wheel
constexpr uint32_t I2C_HZ = 1000000;             // drop to 400000 if the OLED/wheel get flaky

// Button GPIOs in physical order: top row left->right, then bottom row
constexpr uint8_t BTN_PINS[10] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10};

// ---- Special buttons (index into BTN_PINS) --------------------------------
constexpr uint8_t FN_BTN = 0;    // top-left: hold = layer select, tap = its normal action
constexpr uint8_t BOOT_BTN = 9;  // bottom-right: Fn + this for 3 s = USB bootloader

// ---- Wheels ---------------------------------------------------------------
// Flip if a wheel turns the "wrong" way (clockwise should be + / up / next)
constexpr bool INVERT_LEFT = false;
constexpr bool INVERT_RIGHT = false;
constexpr uint16_t LAYER_STEPS_PER_REV = 10;  // Fn + left wheel layer scrolling
constexpr uint32_t HOLD_MODS_RELEASE_MS = 700; // e.g. Alt in Alt+Tab is let go this long after you stop turning

// ---- Timing ---------------------------------------------------------------
constexpr uint32_t DEBOUNCE_MS = 5;
constexpr uint32_t FN_TAP_MAX_MS = 600;       // hold Fn longer than this w/o doing anything = no tap action
constexpr uint32_t BOOTLOADER_HOLD_MS = 3000;
constexpr uint32_t DIM_AFTER_MS = 30UL * 1000;       // OLED dims
constexpr uint32_t SLEEP_AFTER_MS = 5UL * 60 * 1000; // OLED turns off (burn-in protection)
constexpr uint32_t LAYER_SAVE_DELAY_MS = 3000;       // remember layer across power cycles
