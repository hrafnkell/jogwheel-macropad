// Shared interface between the engine (main.cpp) and the host side (host.cpp)
#pragma once
#include <Arduino.h>
#include "keymap.h"

#define FW_VERSION "2.0"

// ---- main.cpp -------------------------------------------------------------
extern uint8_t currentLayer;
void setLayer(uint8_t n);
void onConfigLoaded();                 // re-apply settings, reset wheel/button state
void showToast(const String &msg, uint32_t ms = 2500);
void releaseAllOutputs();

// ---- host.cpp: storage, USB drive, serial protocol --------------------------
void storageBegin();
void reloadConfig();
void hostPoll();
bool driveActive();
void setDrive(bool on);

// 0 = off, 1 = send button/wheel events, 2 = events + mute HID output (configurator "learn" mode)
extern uint8_t monitorMode;
inline bool hidMuted() { return monitorMode == 2; }
void sendButtonEvent(uint8_t i, bool down);
void sendLayerEvent();
void sendWheelEvent(uint8_t w, float speed, uint8_t boost, uint16_t raw);
