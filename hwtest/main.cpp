// ---------------------------------------------------------------------------
// Jogwheel macropad - peripheral test sketch
//
// Hardware (RP2040 Pico):
//   I2C0  SDA=GP12 SCL=GP13 : AS5600 (right wheel, 0x36) + SSD1306 (0x3C)
//   I2C1  SDA=GP14 SCL=GP15 : AS5600 (left wheel, 0x36)
//   Buttons GP1..GP10, active LOW (internal pull-ups)
//
// Physical layout:
//      O  [OLED]  O
//      B  B  B  B  B
//      B  B  B  B  B
//
// What this does:
//   - Scans both I2C buses at boot and prints what it finds (Serial + OLED)
//   - Shows angle, magnet status and accumulated position for each wheel
//   - Shows a 2x5 button grid that lights up when pressed
//   - Logs button edges and wheel movement over USB serial (115200)
// ---------------------------------------------------------------------------
#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>

// ---- Pins -----------------------------------------------------------------
constexpr uint8_t I2C0_SDA = 12, I2C0_SCL = 13;
constexpr uint8_t I2C1_SDA = 14, I2C1_SCL = 15;

// Button GPIOs in physical order: top row left->right, then bottom row.
// If the test shows a different order, just reorder this array.
constexpr uint8_t BTN_PINS[10] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10};
constexpr uint8_t BTN_COUNT = sizeof(BTN_PINS);

// ---- Devices --------------------------------------------------------------
constexpr uint8_t AS5600_ADDR  = 0x36;
constexpr uint8_t SSD1306_ADDR = 0x3C;

Adafruit_SSD1306 display(128, 64, &Wire, -1);
bool displayOk = false;

// ---- AS5600 ---------------------------------------------------------------
struct Encoder {
  TwoWire *bus;
  const char *name;
  bool present = false;
  uint16_t raw = 0;       // 0..4095
  uint16_t lastRaw = 0;
  int32_t position = 0;   // accumulated ticks (4096 per revolution)
  uint8_t status = 0;     // MD/ML/MH bits
  uint8_t agc = 0;
  uint16_t magnitude = 0;
};

Encoder encL{&Wire1, "L"};  // left wheel is on I2C1
Encoder encR{&Wire,  "R"};  // right wheel is on I2C0 (shared with OLED)

bool i2cRead(TwoWire *bus, uint8_t addr, uint8_t reg, uint8_t *buf, uint8_t len) {
  bus->beginTransmission(addr);
  bus->write(reg);
  if (bus->endTransmission(false) != 0) return false;
  if (bus->requestFrom(addr, len) != len) return false;
  for (uint8_t i = 0; i < len; i++) buf[i] = bus->read();
  return true;
}

bool i2cProbe(TwoWire *bus, uint8_t addr) {
  bus->beginTransmission(addr);
  return bus->endTransmission() == 0;
}

// AS5600 registers
constexpr uint8_t REG_STATUS = 0x0B;  // bit5 MD (magnet detected), bit4 ML (too weak), bit3 MH (too strong)
constexpr uint8_t REG_RAW_ANGLE = 0x0C;
constexpr uint8_t REG_AGC = 0x1A;
constexpr uint8_t REG_MAGNITUDE = 0x1B;

bool readEncoder(Encoder &e) {
  uint8_t b[2];
  if (!i2cRead(e.bus, AS5600_ADDR, REG_RAW_ANGLE, b, 2)) { e.present = false; return false; }
  e.present = true;
  e.raw = ((b[0] & 0x0F) << 8) | b[1];

  // Unwrap across the 4095->0 boundary to get continuous position
  int16_t delta = (int16_t)e.raw - (int16_t)e.lastRaw;
  if (delta > 2048) delta -= 4096;
  else if (delta < -2048) delta += 4096;
  e.position += delta;
  e.lastRaw = e.raw;

  if (i2cRead(e.bus, AS5600_ADDR, REG_STATUS, b, 1)) e.status = b[0];
  if (i2cRead(e.bus, AS5600_ADDR, REG_AGC, b, 1)) e.agc = b[0];
  if (i2cRead(e.bus, AS5600_ADDR, REG_MAGNITUDE, b, 2)) e.magnitude = ((b[0] & 0x0F) << 8) | b[1];
  return true;
}

const char *magnetStr(uint8_t s) {
  if (!(s & 0x20)) return "NO MAG";
  if (s & 0x10) return "WEAK";
  if (s & 0x08) return "STRONG";
  return "OK";
}

void initEncoder(Encoder &e) {
  e.present = i2cProbe(e.bus, AS5600_ADDR);
  if (e.present) {
    readEncoder(e);
    e.lastRaw = e.raw;
    e.position = 0;
  }
}

// ---- Buttons --------------------------------------------------------------
constexpr uint32_t DEBOUNCE_MS = 5;
bool btnState[BTN_COUNT] = {};
bool btnRaw[BTN_COUNT] = {};
uint32_t btnChanged[BTN_COUNT] = {};
uint32_t btnPressCount[BTN_COUNT] = {};

void updateButtons() {
  uint32_t now = millis();
  for (uint8_t i = 0; i < BTN_COUNT; i++) {
    bool pressed = digitalRead(BTN_PINS[i]) == LOW;
    if (pressed != btnRaw[i]) { btnRaw[i] = pressed; btnChanged[i] = now; }
    if (btnState[i] != btnRaw[i] && now - btnChanged[i] >= DEBOUNCE_MS) {
      btnState[i] = btnRaw[i];
      if (btnState[i]) btnPressCount[i]++;
      Serial.printf("BTN %2u (GP%-2u) %s\n", i + 1, BTN_PINS[i], btnState[i] ? "DOWN" : "up");
    }
  }
}

// ---- I2C scan -------------------------------------------------------------
void scanBus(TwoWire *bus, const char *name) {
  Serial.printf("Scanning %s:", name);
  int found = 0;
  for (uint8_t a = 1; a < 127; a++) {
    if (i2cProbe(bus, a)) { Serial.printf(" 0x%02X", a); found++; }
  }
  Serial.println(found ? "" : " (nothing)");
}

// ---- Display --------------------------------------------------------------
void drawWheel(int cx, int cy, int r, const Encoder &e) {
  display.drawCircle(cx, cy, r, SSD1306_WHITE);
  if (!e.present) {
    display.drawLine(cx - r + 3, cy - r + 3, cx + r - 3, cy + r - 3, SSD1306_WHITE);
    display.drawLine(cx - r + 3, cy + r - 3, cx + r - 3, cy - r + 3, SSD1306_WHITE);
    return;
  }
  float a = e.raw * (TWO_PI / 4096.0f) - HALF_PI;  // 0 = up
  display.drawLine(cx, cy, cx + cosf(a) * (r - 1), cy + sinf(a) * (r - 1), SSD1306_WHITE);
  display.fillCircle(cx, cy, 1, SSD1306_WHITE);
}

void drawEncoderText(int x, int y, const Encoder &e) {
  display.setCursor(x, y);
  if (!e.present) { display.print("missing"); return; }
  display.printf("%5.1f", e.raw * 360.0f / 4096.0f);
  display.setCursor(x, y + 8);
  display.print(magnetStr(e.status));
  display.setCursor(x, y + 16);
  display.printf("%ld", (long)e.position);
}

void drawScreen() {
  display.clearDisplay();
  display.setTextSize(1);
  display.setTextColor(SSD1306_WHITE);

  // Wheels in the top corners, mirroring the physical layout
  drawWheel(10, 10, 9, encL);
  drawWheel(117, 10, 9, encR);
  drawEncoderText(22, 0, encL);

  // Right encoder text (kept clear of the right wheel)
  drawEncoderText(66, 0, encR);

  // Button grid: 2 rows x 5
  const int bw = 22, bh = 16, gap = 3;
  const int x0 = (128 - (5 * bw + 4 * gap)) / 2;
  const int y0 = 26;
  for (uint8_t i = 0; i < BTN_COUNT; i++) {
    int col = i % 5, row = i / 5;
    int x = x0 + col * (bw + gap);
    int y = y0 + row * (bh + gap);
    if (btnState[i]) {
      display.fillRoundRect(x, y, bw, bh, 3, SSD1306_WHITE);
      display.setTextColor(SSD1306_BLACK);
    } else {
      display.drawRoundRect(x, y, bw, bh, 3, SSD1306_WHITE);
      display.setTextColor(SSD1306_WHITE);
    }
    display.setCursor(x + (BTN_PINS[i] < 10 ? 8 : 5), y + 4);
    display.print(BTN_PINS[i]);  // label with GPIO number
  }
  display.setTextColor(SSD1306_WHITE);
  display.display();
}

// ---- Setup / loop ---------------------------------------------------------
void setup() {
  Serial.begin(115200);
  uint32_t t0 = millis();
  while (!Serial && millis() - t0 < 2000) delay(10);  // give USB serial a moment

  for (uint8_t i = 0; i < BTN_COUNT; i++) pinMode(BTN_PINS[i], INPUT_PULLUP);
  delay(2);
  // Escape hatch: hold top-left + bottom-right while plugging in -> USB bootloader
  if (digitalRead(BTN_PINS[0]) == LOW && digitalRead(BTN_PINS[BTN_COUNT - 1]) == LOW) rp2040.rebootToBootloader();

  Wire.setSDA(I2C0_SDA);
  Wire.setSCL(I2C0_SCL);
  Wire.begin();
  Wire.setClock(400000);

  Wire1.setSDA(I2C1_SDA);
  Wire1.setSCL(I2C1_SCL);
  Wire1.begin();
  Wire1.setClock(400000);

  Serial.println("\n=== Jogwheel macropad peripheral test ===");
  scanBus(&Wire, "I2C0 (GP12/13)");
  scanBus(&Wire1, "I2C1 (GP14/15)");

  displayOk = display.begin(SSD1306_SWITCHCAPVCC, SSD1306_ADDR, false, false);
  Serial.printf("SSD1306 @0x3C : %s\n", displayOk ? "OK" : "NOT FOUND");
  Wire.setClock(400000);  // Adafruit lib may change the clock

  initEncoder(encL);
  initEncoder(encR);
  Serial.printf("AS5600 L (I2C1): %s\n", encL.present ? "OK" : "NOT FOUND");
  Serial.printf("AS5600 R (I2C0): %s\n", encR.present ? "OK" : "NOT FOUND");

  if (displayOk) {
    display.clearDisplay();
    display.setTextColor(SSD1306_WHITE);
    display.setCursor(0, 0);
    display.println("Jogwheel test");
    display.printf("OLED     OK\n");
    display.printf("AS5600 L %s\n", encL.present ? "OK" : "MISSING");
    display.printf("AS5600 R %s\n", encR.present ? "OK" : "MISSING");
    display.display();
    delay(1500);
  }
}

void loop() {
  static uint32_t lastEnc = 0, lastDraw = 0, lastLog = 0;
  static int32_t loggedL = 0, loggedR = 0;
  uint32_t now = millis();

  updateButtons();

  // Hold top-left + bottom-right for 3 s -> reboot into USB bootloader (no BOOTSEL needed)
  static uint32_t comboSince = 0;
  if (btnState[0] && btnState[BTN_COUNT - 1]) {
    if (!comboSince) comboSince = now;
    else if (now - comboSince >= 3000) {
      if (displayOk) { display.clearDisplay(); display.setCursor(0, 0); display.print("Bootloader..."); display.display(); }
      rp2040.rebootToBootloader();
    }
  } else comboSince = 0;

  if (now - lastEnc >= 5) {  // 200 Hz encoder polling
    lastEnc = now;
    if (!readEncoder(encL)) initEncoder(encL);  // retry if it drops off / gets plugged in
    if (!readEncoder(encR)) initEncoder(encR);
  }

  // Log wheel movement (only when it changed noticeably)
  if (now - lastLog >= 100) {
    lastLog = now;
    if (abs(encL.position - loggedL) > 4 || abs(encR.position - loggedR) > 4) {
      Serial.printf("L: raw=%4u pos=%7ld %-6s agc=%3u mag=%4u | R: raw=%4u pos=%7ld %-6s agc=%3u mag=%4u\n",
                    encL.raw, (long)encL.position, magnetStr(encL.status), encL.agc, encL.magnitude,
                    encR.raw, (long)encR.position, magnetStr(encR.status), encR.agc, encR.magnitude);
      loggedL = encL.position;
      loggedR = encR.position;
    }
  }

  if (displayOk && now - lastDraw >= 33) {  // ~30 fps
    lastDraw = now;
    drawScreen();
  }
}
