// Runtime key map: types + loading from JSON (layers.json / built-in defaults)
#pragma once
#include <Arduino.h>
#include <vector>

constexpr uint8_t NUM_BUTTONS = 10;
constexpr uint8_t MAX_LAYERS = 16;
constexpr uint8_t MAX_APPS = 8;

// Modifier bits
enum : uint8_t { M_CTRL = 1, M_SHIFT = 2, M_ALT = 4, M_GUI = 8 };

enum ActType : uint8_t {
  A_NONE,
  A_KEY,          // keyboard key (+ modifiers), held while button is held. code 0 = modifiers only
  A_MEDIA,        // consumer key (volume, play/pause...), held while button is held
  A_MOUSE,        // mouse button (+ modifiers), held while button is held
  A_MOUSE_LATCH,  // mouse button, toggles on each press (drag lock)
  A_SCROLL,       // mouse wheel notches (+ modifiers); code = int8 notches, + = up
};

struct Action {
  ActType type = A_NONE;
  uint8_t mods = 0;
  uint16_t code = 0;
};

enum WheelMode : uint8_t { W_NONE, W_STEP, W_MOUSE_X, W_MOUSE_Y };

struct Wheel {
  char label[9] = "";
  WheelMode mode = W_NONE;
  int16_t perRev = 0;          // steps per rev, or pixels per rev for mouse modes
  Action ccw, cw, fastCcw, fastCw;
  uint8_t holdMods = 0;
  float accel = 1;             // max repeat / mouse multiplier
  float accelStart = -1, accelFull = -1, accelCurve = -1, fastSpeed = -1;  // -1 = use global setting
};

struct Button {
  char label[5] = "";
  Action action;
};

struct Layer {
  char name[11] = "";
  char shortName[5] = "";
  char apps[MAX_APPS][24] = {};
  uint8_t appCount = 0;
  Wheel left, right;
  Button buttons[NUM_BUTTONS];
};

struct Settings {
  bool invertLeft = false, invertRight = false;
  float accelStart = 0.4f, accelFull = 1.5f, accelCurve = 1.0f, fastSpeed = 1.8f;
  uint16_t smoothingMs = 50;
  uint16_t holdReleaseMs = 700;
  uint8_t layerSteps = 10;
  uint8_t brightness = 100;    // 1..100
  uint32_t dimAfterS = 30, sleepAfterS = 300;
  char autoFallback[11] = "General";
};

extern std::vector<Layer> layers;
extern Settings settings;

// Parse an action string like "ctrl+shift+z". Returns false (and sets err) if invalid.
bool parseAction(const char *s, Action &out, String &err);

// Load a full config from JSON text. On a fatal error (bad JSON / no layers) the
// current config is left untouched and false is returned. Non-fatal problems
// (unknown key names etc.) are appended to `errors` and the item is skipped.
bool loadConfig(const char *json, size_t len, std::vector<String> &errors);

// Find the layer index whose "apps" match a window class/title. -1 = none.
int matchApp(const char *wmClass, const char *title);
int findLayerByName(const char *name);
