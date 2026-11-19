// ---------------------------------------------------------------------------
// Jogwheel macropad firmware
//   Key map:  layers.h      Hardware/tuning:  config.h
// ---------------------------------------------------------------------------
#include <Arduino.h>
#include <Wire.h>
#include <EEPROM.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>
#include "actions.h"
#include "config.h"
#include "layers.h"

// Poll HID every 1 ms instead of the core's default 10 ms (snappier wheels)
int usb_hid_poll_interval = 1;

constexpr uint8_t BTN_COUNT = sizeof(BTN_PINS);
static_assert(BTN_COUNT == 10, "layers.h assumes 10 buttons");

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
  switch (a.type) {
    case A_KEY:
      modsDown(a.mods);
      if (a.code) Keyboard.press((uint8_t)a.code);
      break;
    case A_MEDIA: Keyboard.consumerPress(a.code); break;
    case A_MOUSE: Mouse.press((uint8_t)a.code); break;
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
  switch (a.type) {
    case A_KEY:
      if (a.code) Keyboard.release((uint8_t)a.code);
      modsUp(a.mods);
      break;
    case A_MEDIA: Keyboard.consumerRelease(); break;
    case A_MOUSE: Mouse.release((uint8_t)a.code); break;
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

// ===========================================================================
// Encoders
// ===========================================================================
constexpr uint8_t AS5600_ADDR = 0x36;

struct Encoder {
  TwoWire *bus;
  bool invert;
  bool present = false;
  uint16_t last = 0;
  int32_t acc = 0;        // step accumulator (raw counts)
  float mouseAcc = 0;     // fractional mouse pixels
  uint32_t activityAcc = 0;
  bool holding = false;   // holdMods currently pressed
  uint8_t heldMods = 0;
  uint32_t lastStepAt = 0;
  uint32_t flashUntil = 0;
  float speed = 0;        // smoothed speed, revolutions per second
  uint32_t lastReadUs = 0;
  uint8_t boost = 0;      // what the OLED should show: 0 = normal, N = xN, 255 = fast
  uint8_t drawnBoost = 0;
  uint32_t lastRetry = 0;
};

Encoder encs[2] = {{&Wire1, INVERT_LEFT}, {&Wire, INVERT_RIGHT}};  // [0]=left, [1]=right

bool readRaw(Encoder &e, uint16_t &raw) {
  e.bus->beginTransmission(AS5600_ADDR);
  e.bus->write(0x0C);  // RAW ANGLE
  if (e.bus->endTransmission(false) != 0) return false;
  if (e.bus->requestFrom(AS5600_ADDR, (uint8_t)2) != 2) return false;
  raw = ((e.bus->read() & 0x0F) << 8);
  raw |= e.bus->read();
  return true;
}

// Returns movement since last call in raw counts (4096/rev), + = clockwise
int32_t readDelta(Encoder &e) {
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
  return e.invert ? -d : d;
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
// Layers
// ===========================================================================
void setLayer(uint8_t n) {
  if (n >= NUM_LAYERS || n == currentLayer) return;
  resetWheels();
  if (latchedMouse) { Mouse.release(latchedMouse); latchedMouse = 0; }
  currentLayer = n;
  layerSavePending = true;
  layerChangedAt = millis();
  dirty = true;
  Serial.printf("Layer %u: %s\n", n + 1, LAYERS[n].name);
}

void loadLayer() {
  EEPROM.begin(256);
  if (EEPROM.read(0) == 0xA5 && EEPROM.read(1) < NUM_LAYERS) currentLayer = EEPROM.read(1);
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
void processWheel(uint8_t idx, int32_t delta) {
  Encoder &e = encs[idx];
  uint32_t now = millis();

  e.activityAcc += abs(delta);
  if (e.activityAcc > 40) { e.activityAcc = 0; lastActivity = now; }

  if (fnActive) {  // layer select mode: left wheel scrolls layers
    if (idx != 0) return;
    layerAcc += delta;
    const int32_t cps = 4096 / LAYER_STEPS_PER_REV;
    while (layerAcc >= cps) { layerAcc -= cps; setLayer((currentLayer + 1) % NUM_LAYERS); fnUsed = true; }
    while (layerAcc <= -cps) { layerAcc += cps; setLayer((currentLayer + NUM_LAYERS - 1) % NUM_LAYERS); fnUsed = true; }
    return;
  }

  const Wheel &w = idx == 0 ? LAYERS[currentLayer].left : LAYERS[currentLayer].right;
  switch (w.mode) {
    case W_STEP: {
      if (w.perRev <= 0) return;
      const int32_t cps = 4096 / w.perRev;
      e.acc += delta;
      while (e.acc >= cps || e.acc <= -cps) {
        bool cw = e.acc > 0;
        e.acc += cw ? -cps : cps;
        if (w.holdMods && !e.holding) {
          modsDown(w.holdMods);
          e.holding = true;
          e.heldMods = w.holdMods;
        }
        // Speed-dependent output
        bool hasFast = w.fastCw.type != A_NONE;
        if (hasFast && e.speed >= FAST_RPS) {
          tap(cw ? w.fastCw : w.fastCcw);
          e.boost = 255;
        } else {
          float f = (e.speed - ACCEL_START_RPS) / (ACCEL_FULL_RPS - ACCEL_START_RPS);
          f = constrain(f, 0.0f, 1.0f);
          uint8_t repeat = 1 + (uint8_t)(f * (w.accelMax - 1) + 0.5f);
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
      e.mouseAcc += delta * (float)w.perRev / 4096.0f;
      int px = (int)e.mouseAcc;
      if (px) {
        e.mouseAcc -= px;
        px = constrain(px, -127, 127);
        if (w.mode == W_MOUSE_X) Mouse.move(px, 0, 0);
        else Mouse.move(0, px, 0);
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
bool btnState[BTN_COUNT] = {}, btnRaw[BTN_COUNT] = {};
uint32_t btnChangedAt[BTN_COUNT] = {};
bool btnFired[BTN_COUNT] = {};
Action btnHeld[BTN_COUNT];  // action captured at press time (layer may change while held)

void onPress(uint8_t i) {
  if (i == FN_BTN) {
    fnActive = true;
    fnUsed = false;
    fnPressedAt = millis();
    resetWheels();
    return;
  }
  if (fnActive) {  // Fn + button 2..8 -> jump to layer
    fnUsed = true;
    if (i != BOOT_BTN && i >= 1 && i - 1 < NUM_LAYERS) setLayer(i - 1);
    return;
  }
  btnHeld[i] = LAYERS[currentLayer].buttons[i].action;
  btnFired[i] = true;
  actionDown(btnHeld[i]);
}

void onRelease(uint8_t i) {
  if (i == FN_BTN) {
    fnActive = false;
    layerAcc = 0;
    if (!fnUsed && millis() - fnPressedAt < FN_TAP_MAX_MS) tap(LAYERS[currentLayer].buttons[FN_BTN].action);
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
  // Show acceleration next to the label, on the side facing the screen centre
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

void drawScreen() {
  const Layer &L = LAYERS[currentLayer];
  uint32_t now = millis();
  display.clearDisplay();
  display.setTextSize(1);

  // Header bar
  char hdr[24];
  if (fnActive) snprintf(hdr, sizeof(hdr), "LAYER: %s", L.name);
  else snprintf(hdr, sizeof(hdr), "%u/%u %s", currentLayer + 1, NUM_LAYERS, L.name);
  display.fillRect(0, 0, 128, 10, SSD1306_WHITE);
  display.setTextColor(SSD1306_BLACK);
  printCentered(hdr, 64, 1);
  display.setTextColor(SSD1306_WHITE);

  if (fnActive) {
    printCentered("<turn L / pick>", 64, 12);
    for (uint8_t i = 0; i < BTN_COUNT; i++) {
      if (i == FN_BTN) drawCell(i, "Fn", true);
      else if (i == BOOT_BTN) drawCell(i, "Boot", btnState[i]);
      else if (i - 1 < NUM_LAYERS) drawCell(i, LAYERS[i - 1].shortName, i - 1 == currentLayer);
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
  display.display();
}

void updateScreen() {
  if (!displayOk) return;
  uint32_t now = millis(), idle = now - lastActivity;

  // Screensaver
  if (idle < DIM_AFTER_MS) {
    if (screenState != SCREEN_ON) {
      if (screenState == SCREEN_OFF) display.ssd1306_command(SSD1306_DISPLAYON);
      display.dim(false);
      screenState = SCREEN_ON;
      dirty = true;
    }
  } else if (idle < SLEEP_AFTER_MS) {
    if (screenState == SCREEN_ON) { display.dim(true); screenState = SCREEN_DIM; }
  } else if (screenState != SCREEN_OFF) {
    display.ssd1306_command(SSD1306_DISPLAYOFF);
    screenState = SCREEN_OFF;
  }
  if (screenState == SCREEN_OFF) return;

  for (uint8_t w = 0; w < 2; w++)
    if ((encs[w].flashUntil > now) != drawnFlash[w] || encs[w].boost != encs[w].drawnBoost) dirty = true;

  static uint32_t lastDraw = 0;
  if (dirty && now - lastDraw >= 40) {  // max 25 fps; each frame blocks I2C0 ~10 ms
    dirty = false;
    lastDraw = now;
    drawScreen();
  }
}

// ===========================================================================
// Bootloader escape hatch
// ===========================================================================
void enterBootloader() {
  Keyboard.releaseAll();
  Mouse.release(MOUSE_ALL);
  if (displayOk) {
    display.clearDisplay();
    display.ssd1306_command(SSD1306_DISPLAYON);
    display.dim(false);
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

  displayOk = display.begin(SSD1306_SWITCHCAPVCC, 0x3C, false, false);
  for (auto &e : encs) { e.lastRetry = millis() - 1000; readDelta(e); }

  loadLayer();
  lastActivity = millis();
  dirty = true;
}

void loop() {
  uint32_t now = millis();

  updateButtons();
  checkBootCombo();

  static uint32_t lastEnc = 0;
  if (now - lastEnc >= 2) {  // ~500 Hz wheel polling
    lastEnc = now;
    uint32_t nowUs = micros();
    for (uint8_t w = 0; w < 2; w++) {
      Encoder &e = encs[w];
      bool was = e.present;
      int32_t d = readDelta(e);
      // Smoothed speed (rev/s), ~50 ms time constant
      float dt = (nowUs - e.lastReadUs) * 1e-6f;
      e.lastReadUs = nowUs;
      if (dt > 0 && dt < 0.1f) {
        float inst = abs(d) / 4096.0f / dt;
        float a = min(1.0f, dt / 0.05f);
        e.speed += (inst - e.speed) * a;
      }
      if (was != e.present) dirty = true;
      if (d) processWheel(w, d);
    }
  }

  // Let go of held modifiers (e.g. Alt for Alt+Tab) once the wheel stops
  for (auto &e : encs)
    if (e.holding && now - e.lastStepAt > HOLD_MODS_RELEASE_MS) releaseWheelHold(e);

  saveLayerIfNeeded();
  updateScreen();
}
