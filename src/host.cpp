// Storage (FatFS on flash), USB drive mode and the serial protocol used by the
// web configurator and the auto-switch script.
//
// Serial protocol: one command per line, replies are single-line JSON objects.
//   HELLO                 -> {"type":"hello", ...}
//   GET                   -> {"type":"config","text":"<layers.json>","source":"file|default"}
//   GETDEFAULT            -> {"type":"default","text":"<built-in default>"}
//   PUT <bytes>\n<data>   -> {"type":"saved","errors":[...]} or {"type":"error","msg":...}
//   RESET                 -> restore built-in default layers.json, reply like PUT
//   LAYER <index>         -> switch layer (0-based)
//   APP <class>\t<title>  -> auto-switch to the layer whose "apps" match
//   MONITOR <0|1|2>       -> 1 = stream button/wheel events, 2 = also mute HID output
//   DRIVE <0|1>           -> show/hide the USB drive
//   ERRORS                -> {"type":"errors","errors":[...]}
// Unsolicited: {"type":"layer"...} always; {"type":"btn"...}, {"type":"wheel"...} when monitoring.
#include <FatFS.h>
#include <FatFSUSB.h>
#include <ArduinoJson.h>
#include <vector>
#include "app.h"
#include "embedded_files.h"

// Step-by-step storage logging ("DEBUG 1" command, or the debug build)
#ifdef BOOT_DEBUG
bool debugLog = true;
#else
bool debugLog = false;
#endif
#define BOOTLOG(...) do { if (debugLog && Serial) { Serial.printf(__VA_ARGS__); Serial.flush(); delay(5); } } while (0)


static const char *CONFIG_PATH = "/layers.json";
static const char *ERRORS_PATH = "/errors.txt";
static const char *REFERENCE_PATH = "/REFERENCE.txt";

static bool fsOk = false;
static bool driveOn = false;
static volatile bool driveEjected = false;
static bool usingDefaults = true;
static std::vector<String> lastErrors;
uint8_t monitorMode = 0;

// ===========================================================================
// Files
// ===========================================================================
static bool readFile(const char *path, std::vector<char> &out) {
  File f = FatFS.open(path, "r");
  if (!f) return false;
  size_t n = f.size();
  out.resize(n + 1);
  size_t got = f.read((uint8_t *)out.data(), n);
  f.close();
  out.resize(got + 1);
  out[got] = 0;
  return true;
}

static bool writeFile(const char *path, const char *data, size_t len) {
  BOOTLOG("fs: write %s open\n", path);
  File f = FatFS.open(path, "w");
  BOOTLOG("fs: write opened=%d\n", (bool)f);
  if (!f) return false;
  // Always write through a RAM buffer: FatFS hands full sectors straight to the
  // flash driver, and the source can't be read from flash (e.g. the embedded
  // default files) while flash is being programmed - that hangs the chip.
  static uint8_t chunk[512];
  size_t w = 0;
  while (w < len) {
    size_t n = min(sizeof(chunk), len - w);
    memcpy(chunk, data + w, n);
    size_t got = f.write(chunk, n);
    if (got != n) break;
    w += n;
    rp2040.wdt_reset();
  }
  BOOTLOG("fs: wrote %u\n", (unsigned)w);
  f.close();
  BOOTLOG("fs: closed\n");
  return w == len;
}

static bool fsUsable() { return fsOk && !driveOn; }

// ===========================================================================
// Serial output
// ===========================================================================
static void sendDoc(JsonDocument &d, bool force) {
  if (!Serial) return;
  size_t n = measureJson(d) + 1;
  if (!force && Serial.availableForWrite() < (int)n) return;  // never block the pad for events
  serializeJson(d, Serial);
  Serial.write('\n');
}

static void sendError(const String &msg) {
  JsonDocument d;
  d["type"] = "error";
  d["msg"] = msg;
  sendDoc(d, true);
}

static void addErrors(JsonDocument &d) {
  JsonArray a = d["errors"].to<JsonArray>();
  for (auto &e : lastErrors) a.add(e);
}

void sendLayerEvent() {
  JsonDocument d;
  d["type"] = "layer";
  d["index"] = currentLayer;
  d["name"] = currentLayer < layers.size() ? layers[currentLayer].name : "";
  sendDoc(d, false);
}

void sendButtonEvent(uint8_t i, bool down) {
  if (!monitorMode) return;
  JsonDocument d;
  d["type"] = "btn";
  d["i"] = i;
  d["down"] = down;
  sendDoc(d, false);
}

void sendWheelEvent(uint8_t w, float speed, uint8_t boost, uint16_t raw) {
  if (!monitorMode) return;
  JsonDocument d;
  d["type"] = "wheel";
  d["w"] = w;
  d["speed"] = serialized(String(speed, 2));
  d["boost"] = boost;
  d["raw"] = raw;
  sendDoc(d, false);
}

// ===========================================================================
// Config loading
// ===========================================================================
void reloadConfig() {
  std::vector<String> errors;
  std::vector<char> text;
  bool ok = false;
  usingDefaults = false;

  if (fsUsable()) {
    BOOTLOG("cfg: read\n");
    if (readFile(CONFIG_PATH, text)) {
      BOOTLOG("cfg: parse %u bytes\n", (unsigned)text.size());
      ok = loadConfig(text.data(), text.size() - 1, errors);
      if (!ok) errors.push_back("layers.json NOT loaded - using built-in defaults until it is fixed");
    } else {
      BOOTLOG("cfg: write default\n");
      // Give the user a starting point; it's now their file
      if (writeFile(CONFIG_PATH, DEFAULT_LAYERS_JSON, DEFAULT_LAYERS_JSON_LEN))
        ok = loadConfig(DEFAULT_LAYERS_JSON, DEFAULT_LAYERS_JSON_LEN, errors);
    }
  }
  if (!ok) {
    BOOTLOG("cfg: parse defaults\n");
    std::vector<String> ignore;
    loadConfig(DEFAULT_LAYERS_JSON, DEFAULT_LAYERS_JSON_LEN, ignore);
    usingDefaults = true;
  }
  if (errors.size() > 30) errors.resize(30);
  lastErrors = errors;

  BOOTLOG("cfg: %u layers, %u errors\n", (unsigned)layers.size(), (unsigned)errors.size());
  if (fsUsable()) {
    if (errors.empty()) {
      FatFS.remove(ERRORS_PATH);
    } else {
      String s = "Problems found in layers.json:\n\n";
      for (auto &e : errors) s += "- " + e + "\n";
      writeFile(ERRORS_PATH, s.c_str(), s.length());
    }
  }

  BOOTLOG("cfg: apply\n");
  onConfigLoaded();
  for (auto &e : errors) Serial.printf("config: %s\n", e.c_str());
  if (!errors.empty()) showToast(String(errors.size()) + (errors.size() == 1 ? " config error" : " config errors"), 4000);
  else showToast(usingDefaults ? "Default layers" : "Layers loaded", 1500);
}

void storageBegin() {
  BOOTLOG("fs: begin\n");
  fsOk = FatFS.begin();  // formats the flash area on first use
  BOOTLOG("fs: ok=%d\n", fsOk);
  FatFSUSB.onUnplug([](uint32_t) { driveEjected = true; });
  FatFSUSB.driveReady([](uint32_t) { return driveOn; });
  if (!fsOk) return;
  // Keep the reference doc on the drive up to date with the firmware
  BOOTLOG("fs: open ref\n");
  File f = FatFS.open(REFERENCE_PATH, "r");
  BOOTLOG("fs: opened=%d\n", (bool)f);
  bool same = f && f.size() == REFERENCE_TXT_LEN;
  if (f) f.close();
  BOOTLOG("fs: same=%d\n", same);
  if (!same) writeFile(REFERENCE_PATH, REFERENCE_TXT, REFERENCE_TXT_LEN);
  BOOTLOG("fs: ref written\n");
}

// ===========================================================================
// USB drive
// ===========================================================================
bool driveActive() { return driveOn; }

void setDrive(bool on) {
  if (on == driveOn || !fsOk) return;
  releaseAllOutputs();
  if (on) {
    FatFS.end();
    driveOn = true;
    driveEjected = false;
    FatFSUSB.begin();  // USB re-enumerates with the drive added
    showToast("Drive open", 1500);
  } else {
    FatFSUSB.unplug();  // flush cached sector + sync
    FatFSUSB.end();     // USB re-enumerates without the drive
    driveOn = false;
    driveEjected = false;
    FatFS.begin();
    reloadConfig();
  }
}

// ===========================================================================
// Serial commands
// ===========================================================================
static void replySaved() {
  JsonDocument d;
  d["type"] = "saved";
  d["defaults"] = usingDefaults;
  addErrors(d);
  sendDoc(d, true);
}

static void cmdPut(size_t len) {
  if (len == 0 || len > 65536) { sendError("bad length"); return; }
  std::vector<char> buf(len + 1);
  size_t got = 0;
  uint32_t t = millis();
  while (got < len && millis() - t < 5000) {
    rp2040.wdt_reset();
    int a = Serial.available();
    if (a > 0) {
      got += Serial.readBytes(buf.data() + got, min((size_t)a, len - got));
      t = millis();
    } else {
      delay(1);
    }
  }
  buf[got] = 0;
  if (got < len) { sendError("timeout receiving config"); return; }
  if (!fsUsable()) { sendError(driveOn ? "USB drive is open - close it first (Fn + button 9)" : "flash filesystem unavailable"); return; }

  JsonDocument test;  // validate before overwriting the user's file
  DeserializationError de = deserializeJson(test, buf.data(), len);
  if (de) { sendError(String("invalid JSON: ") + de.c_str()); return; }
  if (!test["layers"].is<JsonArray>() || test["layers"].size() == 0) { sendError("no layers in config"); return; }
  test.clear();

  if (!writeFile(CONFIG_PATH, buf.data(), len)) { sendError("write failed"); return; }
  reloadConfig();
  replySaved();
}

static void sendText(const char *type, const char *text, const char *source) {
  JsonDocument d;
  d["type"] = type;
  d["text"] = text;
  if (source) d["source"] = source;
  sendDoc(d, true);
}

static void handleLine(char *line) {
  char *arg = strchr(line, ' ');
  if (arg) *arg++ = 0;
  else arg = line + strlen(line);

  if (!strcmp(line, "HELLO")) {
    JsonDocument d;
    d["type"] = "hello";
    d["fw"] = FW_VERSION;
    d["layer"] = currentLayer;
    d["drive"] = driveOn;
    d["fs"] = fsOk;
    d["defaults"] = usingDefaults;
    JsonArray a = d["layers"].to<JsonArray>();
    for (auto &l : layers) a.add(l.name);
    addErrors(d);
    sendDoc(d, true);
  } else if (!strcmp(line, "GET")) {
    std::vector<char> text;
    if (fsUsable() && readFile(CONFIG_PATH, text)) sendText("config", text.data(), "file");
    else sendText("config", DEFAULT_LAYERS_JSON, "default");
  } else if (!strcmp(line, "GETDEFAULT")) {
    sendText("default", DEFAULT_LAYERS_JSON, nullptr);
  } else if (!strcmp(line, "PUT")) {
    cmdPut(strtoul(arg, nullptr, 10));
  } else if (!strcmp(line, "RESET")) {
    if (!fsUsable()) { sendError("filesystem busy"); return; }
    writeFile(CONFIG_PATH, DEFAULT_LAYERS_JSON, DEFAULT_LAYERS_JSON_LEN);
    reloadConfig();
    replySaved();
  } else if (!strcmp(line, "LAYER")) {
    setLayer(atoi(arg));
  } else if (!strcmp(line, "APP")) {
    char *title = strchr(arg, '\t');
    if (title) *title++ = 0;
    int idx = matchApp(arg, title ? title : "");
    if (idx < 0 && settings.autoFallback[0]) idx = findLayerByName(settings.autoFallback);
    if (idx >= 0) setLayer(idx);
  } else if (!strcmp(line, "MONITOR")) {
    uint8_t m = constrain(atoi(arg), 0, 2);
    if (m == 2 && monitorMode != 2) releaseAllOutputs();
    monitorMode = m;
    sendLayerEvent();
  } else if (!strcmp(line, "DRIVE")) {
    setDrive(atoi(arg) != 0);
  } else if (!strcmp(line, "DEBUG")) {
    debugLog = atoi(arg) != 0;
  } else if (!strcmp(line, "FSINIT")) {  // retry storage at runtime (e.g. from safe mode)
    storageBegin();
    reloadConfig();
    replySaved();
  } else if (!strcmp(line, "FORMAT")) {  // wipe the config drive
    if (driveOn) { sendError("close the drive first"); return; }
    FatFS.end();
    fsOk = FatFS.format() && FatFS.begin();
    storageBegin();
    reloadConfig();
    replySaved();
  } else if (!strcmp(line, "ERRORS")) {
    JsonDocument d;
    d["type"] = "errors";
    addErrors(d);
    sendDoc(d, true);
  } else if (line[0]) {
    sendError(String("unknown command: ") + line);
  }
}

void hostPoll() {
  static char buf[512];
  static size_t n = 0;
  while (Serial.available()) {
    char c = Serial.read();
    if (c == '\r') continue;
    if (c == '\n') {
      buf[n] = 0;
      n = 0;
      handleLine(buf);
      continue;
    }
    if (n < sizeof(buf) - 1) buf[n++] = c;
  }

  // Host disconnected (configurator closed): stop streaming and un-mute
  if (!Serial && monitorMode) monitorMode = 0;

  // Host ejected the drive: hide it and load the edited config
  if (driveEjected && driveOn) setDrive(false);
}
