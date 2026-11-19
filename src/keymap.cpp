#include "keymap.h"
#include <ArduinoJson.h>
#include <Keyboard.h>
#include <Mouse.h>

std::vector<Layer> layers;
Settings settings;

// ===========================================================================
// Action strings
// ===========================================================================
struct Token {
  const char *name;
  uint16_t code;
};

static const Token MOD_TOKENS[] = {
  {"ctrl", M_CTRL}, {"control", M_CTRL}, {"shift", M_SHIFT}, {"alt", M_ALT}, {"option", M_ALT},
  {"gui", M_GUI}, {"super", M_GUI}, {"win", M_GUI}, {"meta", M_GUI}, {"cmd", M_GUI},
};

static const Token KEY_TOKENS[] = {
  {"space", ' '}, {"enter", KEY_RETURN}, {"return", KEY_RETURN}, {"esc", KEY_ESC}, {"escape", KEY_ESC},
  {"tab", KEY_TAB}, {"backspace", KEY_BACKSPACE}, {"delete", KEY_DELETE}, {"del", KEY_DELETE},
  {"insert", KEY_INSERT}, {"ins", KEY_INSERT}, {"home", KEY_HOME}, {"end", KEY_END},
  {"pageup", KEY_PAGE_UP}, {"pgup", KEY_PAGE_UP}, {"pagedown", KEY_PAGE_DOWN}, {"pgdn", KEY_PAGE_DOWN},
  {"up", KEY_UP_ARROW}, {"down", KEY_DOWN_ARROW}, {"left", KEY_LEFT_ARROW}, {"right", KEY_RIGHT_ARROW},
  {"printscreen", KEY_PRINT_SCREEN}, {"prtsc", KEY_PRINT_SCREEN}, {"scrolllock", KEY_SCROLL_LOCK},
  {"pause", KEY_PAUSE}, {"capslock", KEY_CAPS_LOCK}, {"numlock", KEY_NUM_LOCK}, {"menu", KEY_MENU},
  {"minus", '-'}, {"equal", '='}, {"lbracket", '['}, {"rbracket", ']'}, {"backslash", '\\'},
  {"semicolon", ';'}, {"quote", '\''}, {"backtick", '`'}, {"comma", ','}, {"period", '.'}, {"slash", '/'},
  {"kp_0", KEY_KP_0}, {"kp_1", KEY_KP_1}, {"kp_2", KEY_KP_2}, {"kp_3", KEY_KP_3}, {"kp_4", KEY_KP_4},
  {"kp_5", KEY_KP_5}, {"kp_6", KEY_KP_6}, {"kp_7", KEY_KP_7}, {"kp_8", KEY_KP_8}, {"kp_9", KEY_KP_9},
  {"kp_plus", KEY_KP_PLUS}, {"kp_minus", KEY_KP_MINUS}, {"kp_multiply", KEY_KP_ASTERISK},
  {"kp_divide", KEY_KP_SLASH}, {"kp_enter", KEY_KP_ENTER}, {"kp_dot", KEY_KP_DOT},
};

static const Token MEDIA_TOKENS[] = {
  {"vol_up", KEY_VOLUME_INCREMENT}, {"vol_down", KEY_VOLUME_DECREMENT}, {"mute", KEY_MUTE},
  {"play_pause", KEY_PLAY_PAUSE}, {"next_track", KEY_SCAN_NEXT}, {"prev_track", KEY_SCAN_PREVIOUS},
  {"stop", KEY_STOP}, {"bright_up", KEY_BRIGHTNESS_INCREMENT}, {"bright_down", KEY_BRIGHTNESS_DECREMENT},
  {"calculator", KEY_AL_CALCULATOR}, {"browser_back", KEY_AC_BACK}, {"browser_forward", KEY_AC_FORWARD},
  {"browser_refresh", KEY_AC_REFRESH}, {"browser_home", KEY_AC_HOME}, {"search", KEY_AC_SEARCH},
};

static const Token MOUSE_TOKENS[] = {
  {"mouse_left", MOUSE_LEFT}, {"mouse_right", MOUSE_RIGHT}, {"mouse_middle", MOUSE_MIDDLE},
};

template <size_t N>
static bool lookup(const Token (&table)[N], const String &name, uint16_t &code) {
  for (auto &t : table)
    if (name == t.name) { code = t.code; return true; }
  return false;
}

bool parseAction(const char *s, Action &out, String &err) {
  out = Action();
  String str(s ? s : "");
  str.trim();
  str.toLowerCase();
  if (str.length() == 0 || str == "none") return true;
  if (str.indexOf(',') >= 0) { err = "key sequences (a, b) only work on buttons - use \"comma\" for the , key"; return false; }
  if (str.startsWith("wait:")) {
    int ms = str.substring(5).toInt();
    if (ms < 1 || ms > MAX_WAIT_MS) { err = "wait must be 1-5000 ms (\"" + String(s) + "\")"; return false; }
    out = {A_WAIT, 0, (uint16_t)ms};
    return true;
  }

  uint8_t mods = 0;
  String last;
  int start = 0;
  // Split on '+' (a trailing "+" alone isn't a key name we support; use kp_plus)
  while (true) {
    int p = str.indexOf('+', start);
    String tok = str.substring(start, p < 0 ? str.length() : p);
    tok.trim();
    if (tok.length() == 0) { err = "empty key name in \"" + String(s) + "\""; return false; }
    if (p < 0) { last = tok; break; }
    uint16_t m;
    if (!lookup(MOD_TOKENS, tok, m)) { err = "\"" + tok + "\" is not a modifier (in \"" + String(s) + "\")"; return false; }
    mods |= m;
    start = p + 1;
  }

  uint16_t code;
  if (lookup(MOD_TOKENS, last, code)) {  // modifiers only
    out = {A_KEY, (uint8_t)(mods | code), 0};
    return true;
  }
  if (last.length() == 1) {
    char c = last[0];
    if (c > ' ' && c < 127) { out = {A_KEY, mods, (uint16_t)c}; return true; }
  }
  if (last.length() >= 2 && last.length() <= 3 && last[0] == 'f') {
    int n = last.substring(1).toInt();
    if (n >= 1 && n <= 24) {
      out = {A_KEY, mods, (uint16_t)(n <= 12 ? KEY_F1 + n - 1 : KEY_F13 + n - 13)};
      return true;
    }
  }
  if (lookup(KEY_TOKENS, last, code)) { out = {A_KEY, mods, code}; return true; }
  if (lookup(MEDIA_TOKENS, last, code)) {
    if (mods) { err = "media keys can't have modifiers (\"" + String(s) + "\")"; return false; }
    out = {A_MEDIA, 0, code};
    return true;
  }
  if (lookup(MOUSE_TOKENS, last, code)) { out = {A_MOUSE, mods, code}; return true; }
  if (last.startsWith("latch_")) {
    if (lookup(MOUSE_TOKENS, "mouse_" + last.substring(6), code)) { out = {A_MOUSE_LATCH, 0, code}; return true; }
  }
  if (last.startsWith("scroll_up") || last.startsWith("scroll_down")) {
    bool up = last.startsWith("scroll_up");
    String rest = last.substring(up ? 9 : 11);
    int n = 1;
    if (rest.length()) {
      if (rest[0] != ':' || (n = rest.substring(1).toInt()) < 1 || n > 127) {
        err = "bad scroll amount in \"" + String(s) + "\"";
        return false;
      }
    }
    int8_t v = up ? n : -n;
    out = {A_SCROLL, mods, (uint16_t)(uint8_t)v};
    return true;
  }
  err = "unknown key \"" + last + "\"";
  return false;
}

// ===========================================================================
// JSON -> structs
// ===========================================================================
static void copyStr(char *dst, size_t size, const char *src) {
  strncpy(dst, src ? src : "", size - 1);
  dst[size - 1] = 0;
}

bool parseSequence(const char *s, Button &out, String &err) {
  out.len = 0;
  String str(s ? s : "");
  int start = 0;
  while (true) {
    int p = str.indexOf(',', start);
    String step = str.substring(start, p < 0 ? str.length() : p);
    step.trim();
    if (step.length() == 0) {
      if (p < 0 && out.len == 0) return true;  // empty = no action
      err = "empty step in \"" + str + "\"";
      out.len = 0;
      return false;
    }
    if (out.len >= MAX_SEQ) { err = "more than " + String(MAX_SEQ) + " steps"; out.len = 0; return false; }
    if (!parseAction(step.c_str(), out.seq[out.len], err)) { out.len = 0; return false; }
    out.len++;
    if (p < 0) return true;
    start = p + 1;
  }
}

static void readAction(JsonVariantConst v, Action &out, const String &where, std::vector<String> &errors) {
  if (v.isNull()) { out = Action(); return; }
  String err;
  if (!parseAction(v.as<const char *>(), out, err)) errors.push_back(where + ": " + err);
  else if (out.type == A_WAIT) { errors.push_back(where + ": wait only works in button sequences"); out = Action(); }
}

// Button action: a string ("a, b") or an array of strings (["a", "b"])
static void readButtonAction(JsonVariantConst v, Button &b, const String &where, std::vector<String> &errors) {
  String s;
  if (v.is<JsonArrayConst>()) {
    for (JsonVariantConst x : v.as<JsonArrayConst>()) {
      if (s.length()) s += ",";
      s += x.as<const char *>() ? x.as<const char *>() : "";
    }
  } else if (!v.isNull()) {
    s = v.as<const char *>() ? v.as<const char *>() : "";
  }
  String err;
  if (!parseSequence(s.c_str(), b, err)) errors.push_back(where + ": " + err);
}

static float optFloat(JsonVariantConst v, float dflt) { return v.is<float>() || v.is<int>() ? v.as<float>() : dflt; }

static void readWheel(JsonObjectConst o, Wheel &w, const String &where, std::vector<String> &errors) {
  w = Wheel();
  if (o.isNull()) return;
  copyStr(w.label, sizeof(w.label), o["label"] | "");
  String mode = o["mode"] | "none";
  if (mode == "steps") {
    w.mode = W_STEP;
    w.perRev = constrain((int)(o["steps"] | 24), 1, 512);
    readAction(o["ccw"], w.ccw, where + ".ccw", errors);
    readAction(o["cw"], w.cw, where + ".cw", errors);
    readAction(o["fast_ccw"], w.fastCcw, where + ".fast_ccw", errors);
    readAction(o["fast_cw"], w.fastCw, where + ".fast_cw", errors);
    Action hold;
    readAction(o["hold"], hold, where + ".hold", errors);
    if (hold.type == A_KEY && hold.code == 0) w.holdMods = hold.mods;
    else if (hold.type != A_NONE) errors.push_back(where + ".hold: must be modifiers only (e.g. \"alt\")");
  } else if (mode == "mouse_x" || mode == "mouse_y") {
    w.mode = mode == "mouse_x" ? W_MOUSE_X : W_MOUSE_Y;
    w.perRev = constrain((int)(o["px"] | 1000), -20000, 20000);
  } else if (mode != "none") {
    errors.push_back(where + ".mode: unknown mode \"" + mode + "\" (steps, mouse_x, mouse_y, none)");
  }
  w.accel = constrain(optFloat(o["accel"], 1), 1.0f, 20.0f);
  w.accelStart = optFloat(o["accel_start"], -1);
  w.accelFull = optFloat(o["accel_full"], -1);
  w.accelCurve = optFloat(o["accel_curve"], -1);
  w.fastSpeed = optFloat(o["fast_speed"], -1);
}

bool loadConfig(const char *json, size_t len, std::vector<String> &errors) {
  JsonDocument doc;
  DeserializationError de = deserializeJson(doc, json, len);
  if (de) {
    errors.push_back(String("JSON error: ") + de.c_str() + " - check commas/brackets");
    return false;
  }
  JsonArrayConst arr = doc["layers"];
  if (arr.isNull() || arr.size() == 0) {
    errors.push_back("no \"layers\" array found");
    return false;
  }

  Settings s;
  JsonObjectConst so = doc["settings"];
  if (!so.isNull()) {
    s.invertLeft = so["invert_left"] | s.invertLeft;
    s.invertRight = so["invert_right"] | s.invertRight;
    s.accelStart = optFloat(so["accel_start"], s.accelStart);
    s.accelFull = optFloat(so["accel_full"], s.accelFull);
    s.accelCurve = constrain(optFloat(so["accel_curve"], s.accelCurve), 0.1f, 5.0f);
    s.fastSpeed = optFloat(so["fast_speed"], s.fastSpeed);
    s.smoothingMs = constrain((int)(so["smoothing_ms"] | (int)s.smoothingMs), 1, 1000);
    s.holdReleaseMs = constrain((int)(so["hold_release_ms"] | (int)s.holdReleaseMs), 50, 10000);
    s.layerSteps = constrain((int)(so["layer_steps"] | (int)s.layerSteps), 2, 64);
    s.brightness = constrain((int)(so["brightness"] | (int)s.brightness), 1, 100);
    s.dimAfterS = so["dim_after_s"] | s.dimAfterS;
    s.sleepAfterS = so["sleep_after_s"] | s.sleepAfterS;
    copyStr(s.autoFallback, sizeof(s.autoFallback), so["auto_fallback"] | s.autoFallback);
    if (s.accelFull <= s.accelStart) {
      errors.push_back("settings: accel_full must be larger than accel_start");
      s.accelFull = s.accelStart + 0.1f;
    }
  }

  std::vector<Layer> ls;
  for (JsonObjectConst lo : arr) {
    if (ls.size() >= MAX_LAYERS) {
      errors.push_back("more than 16 layers - the rest are ignored");
      break;
    }
    Layer l;
    String where = "layer " + String(ls.size() + 1);
    copyStr(l.name, sizeof(l.name), lo["name"] | ("Layer " + String(ls.size() + 1)).c_str());
    where = where + " (" + l.name + ")";
    copyStr(l.shortName, sizeof(l.shortName), lo["short"] | l.name);
    for (JsonVariantConst a : lo["apps"].as<JsonArrayConst>()) {
      if (l.appCount >= MAX_APPS) break;
      copyStr(l.apps[l.appCount], sizeof(l.apps[0]), a.as<const char *>());
      for (char *c = l.apps[l.appCount]; *c; c++) *c = tolower(*c);
      if (l.apps[l.appCount][0]) l.appCount++;
    }
    readWheel(lo["left"], l.left, where + " left", errors);
    readWheel(lo["right"], l.right, where + " right", errors);
    JsonArrayConst btns = lo["buttons"];
    if (btns.size() > NUM_BUTTONS) errors.push_back(where + ": more than 10 buttons");
    uint8_t i = 0;
    for (JsonVariantConst b : btns) {
      if (i >= NUM_BUTTONS) break;
      copyStr(l.buttons[i].label, sizeof(l.buttons[i].label), b["label"] | "");
      readButtonAction(b["action"], l.buttons[i], where + " button " + String(i + 1), errors);
      i++;
    }
    ls.push_back(l);
  }

  layers = ls;
  settings = s;
  if (settings.autoFallback[0] && findLayerByName(settings.autoFallback) < 0)
    errors.push_back(String("settings.auto_fallback: no layer named \"") + settings.autoFallback + "\"");
  return true;
}

// ===========================================================================
// Lookup
// ===========================================================================
int findLayerByName(const char *name) {
  for (size_t i = 0; i < layers.size(); i++)
    if (strcasecmp(layers[i].name, name) == 0) return i;
  return -1;
}

int matchApp(const char *wmClass, const char *title) {
  String cls(wmClass), ttl(title);
  cls.toLowerCase();
  ttl.toLowerCase();
  for (size_t i = 0; i < layers.size(); i++) {
    for (uint8_t a = 0; a < layers[i].appCount; a++) {
      const char *pat = layers[i].apps[a];
      if (strncmp(pat, "title:", 6) == 0) {
        if (pat[6] && ttl.indexOf(pat + 6) >= 0) return i;
      } else if (cls.indexOf(pat) >= 0) {
        return i;
      }
    }
  }
  return -1;
}
