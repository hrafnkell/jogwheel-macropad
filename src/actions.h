// Building blocks for the key map in layers.h
#pragma once
#include <Arduino.h>
#include <Keyboard.h>
#include <Mouse.h>

// Modifier bits
enum : uint8_t { M_CTRL = 1, M_SHIFT = 2, M_ALT = 4, M_GUI = 8 };

enum ActType : uint8_t {
  A_NONE,
  A_KEY,          // keyboard key (+ modifiers), held while button is held
  A_MEDIA,        // consumer key (volume, play/pause...), held while button is held
  A_MOUSE,        // mouse button, held while button is held
  A_MOUSE_LATCH,  // mouse button, toggles on each press (drag lock)
  A_SCROLL,       // mouse wheel notches (+ modifiers), fired once on press
};

struct Action {
  ActType type;
  uint8_t mods;
  uint16_t code;
};

// ---- Action helpers -------------------------------------------------------
// Use lowercase letters: key('c'), never key('C') (that would add Shift).
constexpr Action none() { return {A_NONE, 0, 0}; }
constexpr Action key(uint16_t k, uint8_t mods = 0) { return {A_KEY, mods, k}; }
constexpr Action mod(uint8_t mods) { return {A_KEY, mods, 0}; }  // modifier(s) only
constexpr Action ctrl(uint16_t k) { return key(k, M_CTRL); }
constexpr Action shift(uint16_t k) { return key(k, M_SHIFT); }
constexpr Action alt(uint16_t k) { return key(k, M_ALT); }
constexpr Action gui(uint16_t k) { return key(k, M_GUI); }
constexpr Action ctrlShift(uint16_t k) { return key(k, M_CTRL | M_SHIFT); }
constexpr Action media(uint16_t c) { return {A_MEDIA, 0, c}; }
constexpr Action mouseBtn(uint8_t b) { return {A_MOUSE, 0, b}; }
constexpr Action mouseLatch(uint8_t b) { return {A_MOUSE_LATCH, 0, b}; }
// n > 0 scrolls up, n < 0 scrolls down
constexpr Action scroll(int8_t n, uint8_t mods = 0) { return {A_SCROLL, mods, (uint16_t)(uint8_t)n}; }

// ---- Buttons --------------------------------------------------------------
struct Button {
  const char *label;  // max 4 chars (shown on the OLED)
  Action action;
};

// ---- Wheels ---------------------------------------------------------------
enum WheelMode : uint8_t { W_NONE, W_STEP, W_MOUSE_X, W_MOUSE_Y };

struct Wheel {
  const char *label;  // max ~8 chars
  WheelMode mode;
  int16_t perRev;     // W_STEP: steps per revolution. W_MOUSE_*: pixels per revolution (negative = reversed)
  Action ccw, cw;     // W_STEP: fired once per step
  uint8_t holdMods;   // W_STEP: modifiers held down while turning (released shortly after you stop) - e.g. Alt for Alt+Tab
  uint8_t accelMax;   // W_STEP: turning faster repeats each step up to this many times (1 = off)
  Action fastCcw, fastCw;  // W_STEP: optional - used instead of ccw/cw when spinning really fast

  // Chainable tweaks:  wheelSteps(...).accel(4).fast(a, b)
  constexpr Wheel accel(uint8_t maxRepeat) const {
    Wheel w = *this; w.accelMax = maxRepeat; return w;
  }
  constexpr Wheel fast(Action fccw, Action fcw) const {
    Wheel w = *this; w.fastCcw = fccw; w.fastCw = fcw; return w;
  }
};

// Fire `ccw` / `cw` every 1/stepsPerRev of a turn
constexpr Wheel wheelSteps(const char *label, int16_t stepsPerRev, Action ccw, Action cw, uint8_t holdMods = 0) {
  return {label, W_STEP, stepsPerRev, ccw, cw, holdMods, 1, none(), none()};
}
constexpr Wheel wheelMouseX(const char *label, int16_t pxPerRev) { return {label, W_MOUSE_X, pxPerRev, none(), none(), 0, 1, none(), none()}; }
constexpr Wheel wheelMouseY(const char *label, int16_t pxPerRev) { return {label, W_MOUSE_Y, pxPerRev, none(), none(), 0, 1, none(), none()}; }
constexpr Wheel wheelNone() { return {"", W_NONE, 0, none(), none(), 0, 1, none(), none()}; }

// ---- Layers ---------------------------------------------------------------
struct Layer {
  const char *name;       // max 8 chars (header)
  const char *shortName;  // max 4 chars (layer select grid)
  Wheel left, right;
  Button buttons[10];     // top row left->right, then bottom row
};
