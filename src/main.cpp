// ---------------------------------------------------------------------------
// Jogwheel macropad firmware
//   Key map + tuning: layers.json (on the pad's USB drive / web configurator)
//   Defaults:         config/default_layers.json
//   Hardware:         config.h
// ---------------------------------------------------------------------------
#include <Arduino.h>
#include <Wire.h>
#include <EEPROM.h>
#include <Keyboard.h>
#include <Mouse.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>
#include "app.h"
#include "config.h"

extern bool debugLog;
#define BOOTLOG(...) do { if (debugLog && Serial) { Serial.printf(__VA_ARGS__); Serial.flush(); delay(5); } } while (0)

// Poll HID every 1 ms instead of the core's default 10 ms (snappier wheels)
int usb_hid_poll_interval = 1;

constexpr uint8_t BTN_COUNT = sizeof(BTN_PINS);
static_assert(BTN_COUNT == NUM_BUTTONS, "keymap assumes 10 buttons");

// ===========================================================================
// HID output
// ===========================================================================
static const uint8_t MOD_KEYS[4] = {KEY_LEFT_CTRL, KEY_LEFT_SHIFT, KEY_LEFT_ALT, KEY_LEFT_GUI};
static uint8_t modRefs[4] = {};  // ref-counted so two buttons sharing Ctrl don't fight
static uint8_t latchedMouse = 0;

void modsDown(uint8_t m) {
  for (uint8_t b = 0; b < 4; b++)
    if ((m & (1 << b)) && modRefs[b]++ == 0) Keyboard.press(MOD_KEYS[b]);
}

void modsUp(uint8_t m) {
  for (uint8_t b = 0; b < 4; b++)
    if ((m & (1 << b)) && modRefs[b] && --modRefs[b] == 0) Keyboard.release(MOD_KEYS[b]);
}

void actionDown(const Action &a) {
  if (hidMuted()) return;
  switch (a.type) {
    case A_KEY:
      modsDown(a.mods);
      if (a.code) Keyboard.press((uint8_t)a.code);
      break;
    case A_MEDIA: Keyboard.consumerPress(a.code); break;
    case A_MOUSE:
      modsDown(a.mods);
      Mouse.press((uint8_t)a.code);
      break;
    case A_MOUSE_LATCH:
      latchedMouse ^= a.code;
      if (latchedMouse & a.code) Mouse.press((uint8_t)a.code);
      else Mouse.release((uint8_t)a.code);
      break;
    case A_SCROLL:
      modsDown(a.mods);
      Mouse.move(0, 0, (int8_t)(uint8_t)a.code);
      modsUp(a.mods);
      break;
    default: break;
  }
}

void actionUp(const Action &a) {
  if (hidMuted()) return;
  switch (a.type) {
    case A_KEY:
      if (a.code) Keyboard.release((uint8_t)a.code);
      modsUp(a.mods);
      break;
    case A_MEDIA: Keyboard.consumerRelease(); break;
    case A_MOUSE:
      Mouse.release((uint8_t)a.code);
      modsUp(a.mods);
      break;
    default: break;
  }
}

void tap(const Action &a) {
  actionDown(a);
  actionUp(a);
}

// ===========================================================================
// State
// ===========================================================================
uint8_t currentLayer = 0;
bool fnActive = false, fnUsed = false;
uint32_t fnPressedAt = 0;
int32_t layerAcc = 0;
bool dirty = true;
uint32_t lastActivity = 0;
bool layerSavePending = false;
uint32_t layerChangedAt = 0;
bool safeMode = false;
uint32_t bootedAt = 0;
String toastMsg;
uint32_t toastUntil = 0;

void showToast(const String &msg, uint32_t ms) {
  toastMsg = msg;
  toastUntil = millis() + ms;
  lastActivity = millis();
  dirty = true;
}

// ===========================================================================
// Encoders
// ===========================================================================
constexpr uint8_t AS5600_ADDR = 0x36;

struct Encoder {
  TwoWire *bus;
  bool present = false;
  uint16_t last = 0;
  int32_t pos = 0, filt = 0;  // unwrapped raw position, and the deadbanded one
  int32_t acc = 0;        // step accumulator (raw counts)
  float mouseAcc = 0;     // fractional mouse pixels
  uint32_t activityAcc = 0;
  bool holding = false;   // holdMods currently pressed
  uint8_t heldMods = 0;
  uint32_t lastStepAt = 0;
  uint32_t flashUntil = 0;
  uint32_t lastRetry = 0;
  float speed = 0;        // smoothed speed, revolutions per second
  uint32_t lastReadUs = 0;
  uint8_t boost = 0;      // OLED: 0 = normal, N = xN, 255 = fast
  uint8_t drawnBoost = 0;
};

Encoder encs[2] = {{&Wire1}, {&Wire}};  // [0]=left (I2C1), [1]=right (I2C0)

bool readRaw(Encoder &e, uint16_t &raw) {
  e.bus->beginTransmission(AS5600_ADDR);
  e.bus->write(0x0C);  // RAW ANGLE
  if (e.bus->endTransmission(false) != 0) return false;
  if (e.bus->requestFrom(AS5600_ADDR, (uint8_t)2) != 2) return false;
  raw = ((e.bus->read() & 0x0F) << 8);
  raw |= e.bus->read();
  return true;
}

// Movement since last call in raw counts (4096/rev), + = clockwise
int32_t readDelta(uint8_t idx) {
  Encoder &e = encs[idx];
  uint16_t raw;
  if (!e.present) {
    if (millis() - e.lastRetry < 1000) return 0;
    e.lastRetry = millis();
    if (!readRaw(e, raw)) return 0;
    e.present = true;
    e.last = raw;
    return 0;
  }
  if (!readRaw(e, raw)) { e.present = false; return 0; }
  int32_t d = (int32_t)raw - (int32_t)e.last;
  if (d > 2048) d -= 4096;
  else if (d < -2048) d += 4096;
  e.last = raw;
  // Deadband: the sensor jitters +-1 count when still. Track the position with
  // 1 count of hysteresis so jitter never reaches the actions / screensaver.
  e.pos += d;
  int32_t before = e.filt;
  if (e.pos - e.filt > 1) e.filt = e.pos - 1;
  else if (e.filt - e.pos > 1) e.filt = e.pos + 1;
  d = e.filt - before;
  bool inv = idx == 0 ? settings.invertLeft : settings.invertRight;
  return inv ? -d : d;
}

void releaseWheelHold(Encoder &e) {
  if (e.holding) {
    modsUp(e.heldMods);
    e.holding = false;
  }
}

void resetWheels() {
  for (auto &e : encs) {
    releaseWheelHold(e);
    e.acc = 0;
    e.mouseAcc = 0;
  }
  layerAcc = 0;
}

// ===========================================================================
// Buttons state (needed by releaseAllOutputs)
// ===========================================================================
bool btnState[BTN_COUNT] = {}, btnRaw[BTN_COUNT] = {};
uint32_t btnChangedAt[BTN_COUNT] = {};
bool btnFired[BTN_COUNT] = {};
Action btnHeld[BTN_COUNT];  // action captured at press time (layer may change while held)

void releaseAllOutputs() {
  for (auto &e : encs) e.holding = false;
  for (uint8_t i = 0; i < BTN_COUNT; i++) btnFired[i] = false;
  memset(modRefs, 0, sizeof(modRefs));
  latchedMouse = 0;
  Keyboard.releaseAll();
  Keyboard.consumerRelease();
  Mouse.release(MOUSE_ALL);
}

// ===========================================================================
// Layers
// ===========================================================================
void setLayer(uint8_t n) {
  if (n >= layers.size() || n == currentLayer) return;
  resetWheels();
  if (latchedMouse) { Mouse.release(latchedMouse); latchedMouse = 0; }
  currentLayer = n;
  layerSavePending = true;
  layerChangedAt = millis();
  dirty = true;
  sendLayerEvent();
}

void loadSavedLayer() {
  EEPROM.begin(256);
  if (EEPROM.read(0) == 0xA5) currentLayer = EEPROM.read(1);
}

void saveLayerIfNeeded() {
  if (!layerSavePending || millis() - layerChangedAt < LAYER_SAVE_DELAY_MS) return;
  layerSavePending = false;
  if (EEPROM.read(0) == 0xA5 && EEPROM.read(1) == currentLayer) return;
  EEPROM.write(0, 0xA5);
  EEPROM.write(1, currentLayer);
  EEPROM.commit();
}

// ===========================================================================
// Wheels -> actions
// ===========================================================================
// Acceleration multiplier (1..w.accel) for a speed in rev/s, using the wheel's
// overrides or the global settings.
float accelFactor(const Wheel &w, float speed) {
  float start = w.accelStart >= 0 ? w.accelStart : settings.accelStart;
  float full = w.accelFull >= 0 ? w.accelFull : settings.accelFull;
  float curve = w.accelCurve > 0 ? w.accelCurve : settings.accelCurve;
  if (full <= start) full = start + 0.1f;
  float f = constrain((speed - start) / (full - start), 0.0f, 1.0f);
  if (curve != 1.0f) f = powf(f, curve);
  return 1.0f + f * (w.accel - 1.0f);
}

void processWheel(uint8_t idx, int32_t delta) {
  Encoder &e = encs[idx];
  uint32_t now = millis();

  e.activityAcc += abs(delta);
  if (e.activityAcc > 40) { e.activityAcc = 0; lastActivity = now; }

  if (fnActive) {  // layer select mode: left wheel scrolls layers
    if (idx != 0 || layers.empty()) return;
    layerAcc += delta;
    const int32_t cps = 4096 / settings.layerSteps;
    uint8_t n = layers.size();
    while (layerAcc >= cps) { layerAcc -= cps; setLayer((currentLayer + 1) % n); fnUsed = true; }
    while (layerAcc <= -cps) { layerAcc += cps; setLayer((currentLayer + n - 1) % n); fnUsed = true; }
    return;
  }

  const Wheel &w = idx == 0 ? layers[currentLayer].left : layers[currentLayer].right;
  switch (w.mode) {
    case W_STEP: {
      const int32_t cps = max(1, 4096 / w.perRev);
      e.acc += delta;
      float fastSpeed = w.fastSpeed >= 0 ? w.fastSpeed : settings.fastSpeed;
      while (e.acc >= cps || e.acc <= -cps) {
        bool cw = e.acc > 0;
        e.acc += cw ? -cps : cps;
        if (w.holdMods && !e.holding && !hidMuted()) {
          modsDown(w.holdMods);
          e.holding = true;
          e.heldMods = w.holdMods;
        }
        if (w.fastCw.type != A_NONE && e.speed >= fastSpeed) {
          tap(cw ? w.fastCw : w.fastCcw);
          e.boost = 255;
        } else {
          uint8_t repeat = (uint8_t)(accelFactor(w, e.speed) + 0.5f);
          for (uint8_t r = 0; r < repeat; r++) tap(cw ? w.cw : w.ccw);
          e.boost = repeat > 1 ? repeat : 0;
        }
        e.lastStepAt = now;
        e.flashUntil = now + 150;
      }
      break;
    }
    case W_MOUSE_X:
    case W_MOUSE_Y: {
      float mult = accelFactor(w, e.speed);
      e.mouseAcc += delta * (float)w.perRev / 4096.0f * mult;
      int px = (int)e.mouseAcc;
      if (px) {
        e.mouseAcc -= px;
        px = constrain(px, -127, 127);
        if (!hidMuted()) {
          if (w.mode == W_MOUSE_X) Mouse.move(px, 0, 0);
          else Mouse.move(0, px, 0);
        }
        e.boost = mult >= 1.5f ? (uint8_t)(mult + 0.5f) : 0;
        e.flashUntil = now + 150;
      }
      break;
    }
    default: break;
  }
}

// ===========================================================================
// Buttons
// ===========================================================================
void onPress(uint8_t i) {
  sendButtonEvent(i, true);
  if (i == FN_BTN) {
    fnActive = true;
    fnUsed = false;
    fnPressedAt = millis();
    resetWheels();
    return;
  }
  if (fnActive) {
    fnUsed = true;
    if (i == DRIVE_BTN) setDrive(!driveActive());
    else if (i != BOOT_BTN && i >= 1 && i - 1 < (int)layers.size()) setLayer(i - 1);
    return;
  }
  btnHeld[i] = layers[currentLayer].buttons[i].action;
  btnFired[i] = true;
  actionDown(btnHeld[i]);
}

void onRelease(uint8_t i) {
  sendButtonEvent(i, false);
  if (i == FN_BTN) {
    fnActive = false;
    layerAcc = 0;
    if (!fnUsed && millis() - fnPressedAt < FN_TAP_MAX_MS) tap(layers[currentLayer].buttons[FN_BTN].action);
    return;
  }
  if (btnFired[i]) {
    btnFired[i] = false;
    actionUp(btnHeld[i]);
  }
}

void updateButtons() {
  uint32_t now = millis();
  for (uint8_t i = 0; i < BTN_COUNT; i++) {
    bool pressed = digitalRead(BTN_PINS[i]) == LOW;
    if (pressed != btnRaw[i]) { btnRaw[i] = pressed; btnChangedAt[i] = now; }
    if (btnState[i] != btnRaw[i] && now - btnChangedAt[i] >= DEBOUNCE_MS) {
      btnState[i] = btnRaw[i];
      lastActivity = now;
      dirty = true;
      if (btnState[i]) onPress(i);
      else onRelease(i);
    }
  }
}

// ===========================================================================
// Display
// ===========================================================================
Adafruit_SSD1306 display(128, 64, &Wire, -1, I2C_HZ, I2C_HZ);
bool displayOk = false;
enum { SCREEN_ON, SCREEN_DIM, SCREEN_OFF } screenState = SCREEN_ON;
bool drawnFlash[2] = {};
bool drawnToast = false;

void setContrast(uint8_t c) {
  display.ssd1306_command(SSD1306_SETCONTRAST);
  display.ssd1306_command(c);
}

uint8_t fullContrast() { return (uint8_t)map(settings.brightness, 1, 100, 1, 255); }

void printCentered(const char *s, int cx, int y) {
  int w = strlen(s) * 6 - 1;
  display.setCursor(cx - w / 2, y);
  display.print(s);
}

void drawCell(uint8_t i, const char *label, bool lit) {
  const int bw = 24, bh = 20, gap = 2, y0 = 22;
  int x = (i % 5) * (bw + gap), y = y0 + (i / 5) * (bh + gap);
  if (lit) display.fillRoundRect(x, y, bw, bh, 3, SSD1306_WHITE);
  else display.drawRoundRect(x, y, bw, bh, 3, SSD1306_WHITE);
  display.setTextColor(lit ? SSD1306_BLACK : SSD1306_WHITE);
  printCentered(label, x + bw / 2, y + 6);
  display.setTextColor(SSD1306_WHITE);
}

void drawWheelLabel(const char *label, uint8_t boost, bool right, bool lit) {
  // Acceleration tag on the side facing the screen centre
  char tag[6] = "";
  if (boost == 255) strcpy(tag, ">>");
  else if (boost > 1) snprintf(tag, sizeof(tag), "x%u", boost);
  char s[20];
  if (!tag[0]) snprintf(s, sizeof(s), "%s", label);
  else if (right) snprintf(s, sizeof(s), "%s %s", tag, label);
  else snprintf(s, sizeof(s), "%s %s", label, tag);
  int w = strlen(s) * 6 - 1;
  int x = right ? 127 - w : 0;
  if (lit) {
    display.fillRect(x - 1, 11, w + 2, 9, SSD1306_WHITE);
    display.setTextColor(SSD1306_BLACK);
  }
  display.setCursor(x, 12);
  display.print(s);
  display.setTextColor(SSD1306_WHITE);
}

void drawHeader(const char *text) {
  display.fillRect(0, 0, 128, 10, SSD1306_WHITE);
  display.setTextColor(SSD1306_BLACK);
  printCentered(text, 64, 1);
  display.setTextColor(SSD1306_WHITE);
}

void drawScreen() {
  const Layer &L = layers[currentLayer];
  uint32_t now = millis();
  display.clearDisplay();
  display.setTextSize(1);

  char hdr[28];
  if (driveActive()) snprintf(hdr, sizeof(hdr), "USB DRIVE OPEN");
  else if (fnActive) snprintf(hdr, sizeof(hdr), "LAYER: %s", L.name);
  else if (hidMuted()) snprintf(hdr, sizeof(hdr), "CONFIG: %s", L.name);
  else snprintf(hdr, sizeof(hdr), "%u/%u %s", currentLayer + 1, (unsigned)layers.size(), L.name);
  drawHeader(hdr);

  if (driveActive() && !fnActive) {
    printCentered("Edit layers.json,", 64, 16);
    printCentered("then Fn + button 9", 64, 28);
    printCentered("to apply.", 64, 40);
  } else if (fnActive) {
    printCentered("<turn L / pick>", 64, 12);
    for (uint8_t i = 0; i < BTN_COUNT; i++) {
      if (i == FN_BTN) drawCell(i, "Fn", true);
      else if (i == BOOT_BTN) drawCell(i, "Boot", btnState[i]);
      else if (i == DRIVE_BTN) drawCell(i, "Disk", driveActive());
      else if (i - 1 < (int)layers.size()) drawCell(i, layers[i - 1].shortName, i - 1 == currentLayer);
      else drawCell(i, "", false);
    }
  } else {
    for (uint8_t w = 0; w < 2; w++) {
      drawnFlash[w] = encs[w].flashUntil > now;
      if (!drawnFlash[w]) encs[w].boost = 0;
      encs[w].drawnBoost = encs[w].boost;
      const char *lbl = encs[w].present ? (w ? L.right.label : L.left.label) : "(no enc)";
      drawWheelLabel(lbl, encs[w].boost, w == 1, drawnFlash[w]);
    }
    for (uint8_t i = 0; i < BTN_COUNT; i++) {
      const Action &a = L.buttons[i].action;
      bool latched = a.type == A_MOUSE_LATCH && (latchedMouse & a.code);
      drawCell(i, L.buttons[i].label, btnState[i] || latched);
    }
  }

  // Toast message box over the middle of the screen
  drawnToast = toastUntil > now;
  if (drawnToast) {
    int w = min(126, (int)toastMsg.length() * 6 + 8);
    display.fillRect(64 - w / 2, 24, w, 16, SSD1306_BLACK);
    display.drawRect(64 - w / 2, 24, w, 16, SSD1306_WHITE);
    printCentered(toastMsg.c_str(), 64, 28);
  }
  display.display();
}

void updateScreen() {
  if (!displayOk) return;
  uint32_t now = millis(), idle = now - lastActivity;
  bool wantDim = settings.dimAfterS && idle >= settings.dimAfterS * 1000;
  bool wantOff = settings.sleepAfterS && idle >= settings.sleepAfterS * 1000;

  if (!wantDim && !wantOff) {
    if (screenState != SCREEN_ON) {
      if (screenState == SCREEN_OFF) display.ssd1306_command(SSD1306_DISPLAYON);
      setContrast(fullContrast());
      screenState = SCREEN_ON;
      dirty = true;
    }
  } else if (!wantOff) {
    if (screenState == SCREEN_ON) { setContrast(1); screenState = SCREEN_DIM; }
  } else if (screenState != SCREEN_OFF) {
    display.ssd1306_command(SSD1306_DISPLAYOFF);
    screenState = SCREEN_OFF;
  }
  if (screenState == SCREEN_OFF) return;

  for (uint8_t w = 0; w < 2; w++)
    if ((encs[w].flashUntil > now) != drawnFlash[w] || encs[w].boost != encs[w].drawnBoost) dirty = true;
  if ((toastUntil > now) != drawnToast) dirty = true;

  static uint32_t lastDraw = 0;
  if (dirty && now - lastDraw >= 40) {  // max 25 fps; each frame blocks I2C0 ~10 ms
    dirty = false;
    lastDraw = now;
    drawScreen();
  }
}

// Called by host.cpp after (re)loading layers.json
void onConfigLoaded() {
  releaseAllOutputs();
  resetWheels();
  if (currentLayer >= layers.size()) currentLayer = 0;
  if (displayOk && screenState == SCREEN_ON) setContrast(fullContrast());
  dirty = true;
  sendLayerEvent();
}

// ===========================================================================
// Bootloader escape hatch
// ===========================================================================
void enterBootloader() {
  releaseAllOutputs();
  if (displayOk) {
    display.clearDisplay();
    display.ssd1306_command(SSD1306_DISPLAYON);
    setContrast(255);
    display.setTextColor(SSD1306_WHITE);
    printCentered("USB bootloader", 64, 28);
    display.display();
  }
  delay(100);
  rp2040.rebootToBootloader();
}

void checkBootCombo() {
  static uint32_t since = 0;
  if (btnState[FN_BTN] && btnState[BOOT_BTN]) {
    if (!since) since = millis();
    else if (millis() - since >= BOOTLOADER_HOLD_MS) enterBootloader();
  } else {
    since = 0;
  }
}

// ===========================================================================
// Setup / loop
// ===========================================================================
void setup() {
  for (uint8_t i = 0; i < BTN_COUNT; i++) pinMode(BTN_PINS[i], INPUT_PULLUP);
  delay(2);
  // Hold top-left + bottom-right while plugging in -> straight to bootloader
  if (digitalRead(BTN_PINS[FN_BTN]) == LOW && digitalRead(BTN_PINS[BOOT_BTN]) == LOW) rp2040.rebootToBootloader();

  Serial.begin(115200);
  Keyboard.begin();
  Mouse.begin();

  Wire.setSDA(I2C0_SDA);
  Wire.setSCL(I2C0_SCL);
  Wire.begin();
  Wire.setClock(I2C_HZ);
  Wire1.setSDA(I2C1_SDA);
  Wire1.setSCL(I2C1_SCL);
  Wire1.begin();
  Wire1.setClock(I2C_HZ);

#ifdef BOOT_DEBUG
  while (!Serial && millis() < 20000) delay(10);
  delay(500);
#endif
  // Hardware watchdog: if anything hangs (even with interrupts off), reboot.
  // The safe-mode flag below then skips the step that hung.
  rp2040.wdt_begin(8000);
  BOOTLOG("boot: display\n");
  displayOk = display.begin(SSD1306_SWITCHCAPVCC, 0x3C, false, false);
  for (uint8_t w = 0; w < 2; w++) { encs[w].lastRetry = millis() - 1000; readDelta(w); }

  BOOTLOG("boot: eeprom\n");
  loadSavedLayer();
  // Safe mode: if the previous boot never finished (crash/hang while loading the
  // config), skip the flash filesystem this time and run on built-in defaults.
  safeMode = EEPROM.read(2) == 0x5A;
  EEPROM.write(2, 0x5A);
  EEPROM.commit();
  BOOTLOG("boot: safeMode=%d\n", safeMode);
  if (!safeMode) {
    BOOTLOG("boot: storageBegin\n");
    storageBegin();
  }
  BOOTLOG("boot: reloadConfig\n");
  reloadConfig();
  BOOTLOG("boot: done\n");
  if (safeMode) showToast("SAFE MODE", 5000);
  bootedAt = millis();
  lastActivity = millis();
  dirty = true;
}

void loop() {
  uint32_t now = millis();
  rp2040.wdt_reset();

  hostPoll();
  updateButtons();
  checkBootCombo();

  static uint32_t lastEnc = 0, lastWheelEvent = 0;
  if (now - lastEnc >= 2) {  // ~500 Hz wheel polling
    lastEnc = now;
    uint32_t nowUs = micros();
    float tau = settings.smoothingMs / 1000.0f;
    for (uint8_t w = 0; w < 2; w++) {
      Encoder &e = encs[w];
      bool was = e.present;
      int32_t d = readDelta(w);
      // Smoothed speed in rev/s
      float dt = (nowUs - e.lastReadUs) * 1e-6f;
      e.lastReadUs = nowUs;
      if (dt > 0 && dt < 0.1f) {
        float inst = abs(d) / 4096.0f / dt;
        e.speed += (inst - e.speed) * min(1.0f, dt / tau);
      }
      if (was != e.present) dirty = true;
      if (d) processWheel(w, d);
    }
  }

  // Live wheel telemetry for the configurator's tuning graph (20 Hz while moving)
  if (monitorMode && now - lastWheelEvent >= 50) {
    lastWheelEvent = now;
    static float lastSent[2] = {-1, -1};
    for (uint8_t w = 0; w < 2; w++) {
      Encoder &e = encs[w];
      if (e.speed > 0.01f || lastSent[w] != 0) {
        sendWheelEvent(w, e.speed, e.flashUntil > now ? e.boost : 0, e.last);
        lastSent[w] = e.speed > 0.01f ? e.speed : 0;
      }
    }
  }

  // Let go of held modifiers (e.g. Alt for Alt+Tab) once the wheel stops
  for (auto &e : encs)
    if (e.holding && now - e.lastStepAt > settings.holdReleaseMs) releaseWheelHold(e);

  // Boot is considered successful after 3 s of running: clear the safe-mode flag
  if (bootedAt && now - bootedAt > 3000) {
    bootedAt = 0;
    EEPROM.write(2, 0);
    EEPROM.commit();
  }

  saveLayerIfNeeded();
  updateScreen();
}
