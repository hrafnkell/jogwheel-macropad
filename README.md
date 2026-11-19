# Jogwheel Macropad

A USB macropad built on a Raspberry Pi Pico (RP2040). It has two magnetic jog wheels, a 128×64 OLED and 10 buttons, and switchable layers for video editing (Kdenlive, DaVinci Resolve), everyday desktop use, gaming and more.

```
   (L)   [  OLED  ]   (R)        L / R = AS5600 magnetic jog wheels
   [1]  [2]  [3]  [4]  [5]
   [6]  [7]  [8]  [9]  [10]
```

The Pico shows up as a normal USB keyboard and mouse, so no drivers or host software are needed. The USB serial port stays active as well, so you can flash new firmware without pressing BOOTSEL.

## Features

- **7 layers**, each with its own button and wheel mapping, all defined in one file ([`src/layers.h`](src/layers.h))
- **Speed-sensitive jog wheels**: turning slowly is precise (one frame per step), turning faster repeats steps, and spinning switches to a coarser action (for example 1 second per step)
- **Wheel modes**: key/media/scroll steps, held modifiers (Alt+Tab switcher), mouse X/Y movement
- **OLED UI** showing the current layer, the wheel functions (they light up while turning, with `x3` / `>>` for acceleration) and a live button map
- **Layer select** with Fn + wheel or Fn + button; the last used layer is remembered after power-off
- **Screensaver**: the OLED dims after 30 s and turns off after 5 min to prevent burn-in
- **Bootloader from the buttons**, for when the enclosure hides BOOTSEL
- A separate **hardware test sketch** for checking wiring

## Hardware

| Part | Notes |
|---|---|
| Raspberry Pi Pico (RP2040) | |
| 2× AS5600 magnetic encoder | One per I2C bus. Both have the fixed address `0x36`, so they can't share a bus |
| 2× diametric magnet | Mounted on the knob shafts above the AS5600s |
| SSD1306 128×64 OLED (I2C, `0x3C`) | Shares I2C0 with the right wheel |
| 10× momentary push buttons | Each connects its GPIO to GND when pressed; uses the internal pull-ups |

### Wiring

| Function | Pico pin |
|---|---|
| I2C0 SDA (OLED + right wheel) | GP12 |
| I2C0 SCL (OLED + right wheel) | GP13 |
| I2C1 SDA (left wheel) | GP14 |
| I2C1 SCL (left wheel) | GP15 |
| Buttons 1–5 (top row, left → right) | GP1 – GP5 |
| Buttons 6–10 (bottom row, left → right) | GP6 – GP10 |
| Button other leg | GND |
| AS5600 / OLED VCC | 3V3 |

Pins and button order can be changed in [`src/config.h`](src/config.h).

## Software setup

The firmware uses [PlatformIO](https://platformio.org/) with the [Earle Philhower Arduino core](https://github.com/earlephilhower/arduino-pico). The core is fetched automatically through the `maxgerhardt/platform-raspberrypi` platform.

```bash
pip install platformio      # or install the PlatformIO IDE extension for VS Code
git clone https://github.com/hrafnkell/jogwheel-macropad.git
cd jogwheel-macropad
pio run                     # build
```

The first build downloads the toolchain and takes a few minutes.

### Flashing

**First time:** hold BOOTSEL while plugging in the Pico, then run:

```bash
pio run -t upload
```

Alternatively, copy `.pio/build/pico/firmware.uf2` onto the `RPI-RP2` drive.

**After that:** just run `pio run -t upload`. PlatformIO resets the Pico into its bootloader over USB serial.

> **Note:** `platformio.ini` sets `upload_port` / `monitor_port` to one specific Pico's serial path
> (`/dev/serial/by-id/usb-Raspberry_Pi_Pico_<serial>-if00`). Change it to match yours:
> `ls /dev/serial/by-id/`. This is needed because the HID firmware changes the USB product ID,
> so PlatformIO can no longer find the board on its own.

**If USB upload stops working** (for example after a bad firmware), use the button bootloader:

- Hold **button 1 + button 10** for 3 seconds while running, or
- Hold **button 1 + button 10** while plugging in the USB cable.

The `RPI-RP2` drive will appear. Run `pio run -t upload`, or copy the `.uf2` onto it.

### Hardware test sketch

[`hwtest/main.cpp`](hwtest/main.cpp) scans both I2C buses and shows the wheel angles, magnet status (OK / WEAK / STRONG / NO MAG) and the button states on the OLED and over serial.

```bash
pio run -e hwtest -t upload   # flash the test sketch
pio device monitor            # watch the log (115200 baud)
pio run -t upload             # back to the macropad firmware
```

## Using it

### Layer switching

Button 1 (top-left) doubles as **Fn**:

| Action | Result |
|---|---|
| Tap button 1 | Its normal action for the current layer (fires on release) |
| Hold button 1 + turn left wheel | Cycle through layers |
| Hold button 1 + press button 2…8 | Jump directly to layer 1…7 |
| Hold button 1 + button 10 for 3 s | Reboot into the USB bootloader |

While Fn is held, the OLED shows the layer picker.

### Layers

| # | Layer | Left wheel | Right wheel | Buttons 1–5 | Buttons 6–10 |
|---|---|---|---|---|---|
| 1 | General | Scroll | Volume | Copy · Paste · Cut · Undo · Redo | Prev · Play/Pause · Next · Mute · Lock screen |
| 2 | Kdenlive | Jog (frames → 1 s) | Timeline zoom (Ctrl+scroll) | Play · J · K · L · Razor (Shift+R) | In · Out · Select clip (+) · Delete · Undo |
| 3 | Resolve | Jog (frames → 1 s) | Timeline zoom (Alt+scroll) | Play · J · K · L · Split (Ctrl+B) | In · Out · Select clip (Shift+V) · Delete · Undo |
| 4 | Desktop | Window switcher (Alt+Tab) | Browser tabs (Ctrl+Tab) | Back · Forward · Reload · New tab · Close tab | Reopen tab · Find · Address bar · Fullscreen · Screenshot |
| 5 | Gaming | Mouse look (X) | Weapon switch (scroll) | Esc · 1 · 2 · 3 · 4 | Tab · Q · E · R · F |
| 6 | Draw | Mouse X | Mouse Y | Esc · Left click · Middle click · Right click · Undo | Draw lock (toggle left button) · Ctrl · Shift · Alt · Delete |
| 7 | Numbers | Up / Down | Left / Right | 1 · 2 · 3 · 4 · 5 | 6 · 7 · 8 · 9 · 0 |

**Video editing tips**

- The zoom wheels send Ctrl/Alt + mouse wheel, so the mouse pointer must be over the timeline.
- *Select clip* selects the clip under the playhead, so *Delete* removes it.
- **Draw** is an etch-a-sketch mode. Open a paint program, press *Draw* to lock the left mouse button, then turn both wheels.

## Customising

Everything you'd normally tweak is in two files.

### [`src/layers.h`](src/layers.h): the key map

```cpp
{"Kdenlive", "Kden",                                   // name (≤8 chars), short name (≤4)
 wheelSteps("Jog", 48, key(KEY_LEFT_ARROW), key(KEY_RIGHT_ARROW))   // left wheel
     .accel(4)                                         //   up to 4 steps per detent when turning fast
     .fast(shift(KEY_LEFT_ARROW), shift(KEY_RIGHT_ARROW)),  // when spinning
 wheelSteps("Zoom", 16, scroll(-1, M_CTRL), scroll(1, M_CTRL)),     // right wheel
 {
   {"Play", key(' ')}, {"J", key('j')}, ...            // 10 buttons, label ≤4 chars
 }},
```

**Button actions**

| Helper | Example |
|---|---|
| `key(k)` / `key(k, mods)` | `key('j')`, `key(KEY_F5)`, `key(KEY_TAB, M_ALT)` |
| `ctrl(k)`, `shift(k)`, `alt(k)`, `gui(k)`, `ctrlShift(k)` | `ctrl('z')`, `gui('l')` |
| `mod(mods)` | `mod(M_CTRL)`: hold Ctrl only |
| `media(code)` | `media(KEY_PLAY_PAUSE)`, `media(KEY_VOLUME_INCREMENT)` |
| `mouseBtn(b)` | `mouseBtn(MOUSE_LEFT)` |
| `mouseLatch(b)` | toggles a mouse button (drag lock) |
| `scroll(n, mods)` | `scroll(1)` up, `scroll(-1, M_CTRL)` Ctrl+scroll down |

Use lowercase letters: `key('c')`, not `key('C')`, because the uppercase form adds Shift. Key names such as `KEY_LEFT_ARROW` come from the core's [`HID_Keyboard.h`](https://github.com/earlephilhower/arduino-pico/blob/master/libraries/HID_Keyboard/src/HID_Keyboard.h).

**Wheels**

| Helper | Meaning |
|---|---|
| `wheelSteps(label, stepsPerRev, ccw, cw, holdMods)` | Fire an action every 1/N of a turn. `holdMods` is held while turning (e.g. `M_ALT` for Alt+Tab) |
| `.accel(n)` | Repeat each step up to n times when turning fast |
| `.fast(ccw, cw)` | Use these actions instead when spinning really fast |
| `wheelMouseX(label, pxPerRev)` / `wheelMouseY(...)` | Move the mouse; negative = reversed |
| `wheelNone()` | Do nothing |

To add a layer, append another block to `LAYERS[]`. Fn + button jumps to layers 1–8 (buttons 2–9); you can reach more by cycling with the wheel.

> **Keyboard layouts:** keys are sent as US key *positions*. Letters, digits, arrows and F-keys work
> on most layouts, but symbols (`+ - = [ ]`…) land elsewhere on non-US layouts. Prefer keypad keys
> (`KEY_KP_PLUS`) or scroll-based shortcuts for those.

### [`src/config.h`](src/config.h): hardware and behaviour

- Pins, I2C speed (`I2C_HZ`, default 1 MHz; drop to 400 kHz if the OLED or wheels glitch)
- `INVERT_LEFT` / `INVERT_RIGHT`: flip a wheel's direction (clockwise should be + / down / next)
- Acceleration thresholds: `ACCEL_START_RPS`, `ACCEL_FULL_RPS`, `FAST_RPS`
- Fn / bootloader button, debounce, screensaver timings

## Project layout

```
platformio.ini     build environments: pico (firmware), hwtest (test sketch)
src/
  main.cpp         firmware: HID output, wheels, buttons, layers, OLED UI
  actions.h        action / wheel / layer types and helpers
  layers.h         ← the key map
  config.h         ← pins and tuning
hwtest/
  main.cpp         peripheral test sketch
```

## Troubleshooting

| Problem | Fix |
|---|---|
| `pio run -t upload` can't find the board | Update `upload_port` in `platformio.ini` (`ls /dev/serial/by-id/`), or use the button bootloader |
| Wheel turns the wrong way | Toggle `INVERT_LEFT` / `INVERT_RIGHT` in `config.h` |
| Wheels on the wrong side | Swap the buses in `encs[]` in `main.cpp`, or swap the wiring |
| Wheel jittery / jumps | Flash `hwtest` and check the magnet status. Aim for `OK` (not `WEAK`/`STRONG`) by adjusting the magnet distance (~1–3 mm) |
| OLED shows `(no enc)` | That AS5600 isn't answering. Check its wiring with `hwtest` |
| Shortcut does nothing | Check the app's shortcut settings. Defaults vary between versions and keyboard layouts |
