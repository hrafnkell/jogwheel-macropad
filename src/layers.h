// ===========================================================================
//  KEY MAP - edit this file to change what everything does.
//
//  Physical layout:          Button index:
//      L  [OLED]  R
//      B B B B B              0 1 2 3 4
//      B B B B B              5 6 7 8 9
//
//  Layer switching:
//    Hold top-left (Fn) + turn LEFT wheel  -> cycle layers
//    Hold top-left (Fn) + button 2..8      -> jump to layer 1..7
//    Tap top-left                          -> its normal action (fires on release)
//    Hold top-left + bottom-right 3 s      -> USB bootloader
//
//  Note: keys are sent as US-layout key positions. Letters, digits, arrows,
//  F-keys etc. are the same on most layouts; symbols like + - = are not,
//  which is why zoom uses Ctrl/Alt + scroll wheel instead of Ctrl + '+'.
// ===========================================================================
#pragma once
#include "actions.h"

const Layer LAYERS[] = {
  // ---- 1: General ----------------------------------------------------------
  {"General", "Gen",
   wheelSteps("Scroll", 24, scroll(1), scroll(-1)),
   wheelSteps("Volume", 24, media(KEY_VOLUME_DECREMENT), media(KEY_VOLUME_INCREMENT)),
   {
     {"Copy", ctrl('c')}, {"Pste", ctrl('v')}, {"Cut", ctrl('x')}, {"Undo", ctrl('z')}, {"Redo", ctrlShift('z')},
     {"Prev", media(KEY_SCAN_PREVIOUS)}, {"Play", media(KEY_PLAY_PAUSE)}, {"Next", media(KEY_SCAN_NEXT)}, {"Mute", media(KEY_MUTE)}, {"Lock", gui('l')},
   }},

  // ---- 2: Kdenlive ---------------------------------------------------------
  // Left = frame step, right = timeline zoom (Ctrl+wheel, mouse must be over timeline)
  {"Kdenlive", "Kden",
   wheelSteps("Frame", 48, key(KEY_LEFT_ARROW), key(KEY_RIGHT_ARROW)),
   wheelSteps("Zoom", 16, scroll(-1, M_CTRL), scroll(1, M_CTRL)),
   {
     {"Play", key(' ')}, {"J", key('j')}, {"K", key('k')}, {"L", key('l')}, {"Razr", shift('r')},
     {"In", key('i')}, {"Out", key('o')}, {"Del", key(KEY_DELETE)}, {"Undo", ctrl('z')}, {"Redo", ctrlShift('z')},
   }},

  // ---- 3: DaVinci Resolve --------------------------------------------------
  // Left = frame step, right = timeline zoom (Alt+wheel; try M_CTRL if that does nothing)
  {"Resolve", "Dvnc",
   wheelSteps("Frame", 48, key(KEY_LEFT_ARROW), key(KEY_RIGHT_ARROW)),
   wheelSteps("Zoom", 16, scroll(-1, M_ALT), scroll(1, M_ALT)),
   {
     {"Play", key(' ')}, {"J", key('j')}, {"K", key('k')}, {"L", key('l')}, {"Splt", ctrl('b')},
     {"In", key('i')}, {"Out", key('o')}, {"Mark", key('m')}, {"Del", key(KEY_DELETE)}, {"Undo", ctrl('z')},
   }},

  // ---- 4: Desktop / browser ------------------------------------------------
  // Left = Alt+Tab window switcher (Alt held while turning), right = browser tabs
  {"Desktop", "Desk",
   wheelSteps("Windows", 10, shift(KEY_TAB), key(KEY_TAB), M_ALT),
   wheelSteps("Tabs", 12, ctrlShift(KEY_TAB), ctrl(KEY_TAB)),
   {
     {"Back", alt(KEY_LEFT_ARROW)}, {"Fwd", alt(KEY_RIGHT_ARROW)}, {"Rfsh", key(KEY_F5)}, {"NewT", ctrl('t')}, {"Clse", ctrl('w')},
     {"ReOp", ctrlShift('t')}, {"Find", ctrl('f')}, {"Addr", ctrl('l')}, {"Full", key(KEY_F11)}, {"Shot", key(KEY_PRINT_SCREEN)},
   }},

  // ---- 5: Gaming -----------------------------------------------------------
  // Left = mouse look / steering, right = weapon wheel (scroll)
  {"Gaming", "Game",
   wheelMouseX("Look", 2000),
   wheelSteps("Weapon", 12, scroll(1), scroll(-1)),
   {
     {"Esc", key(KEY_ESC)}, {"1", key('1')}, {"2", key('2')}, {"3", key('3')}, {"4", key('4')},
     {"Tab", key(KEY_TAB)}, {"Q", key('q')}, {"E", key('e')}, {"R", key('r')}, {"F", key('f')},
   }},

  // ---- 6: Draw (etch-a-sketch) ---------------------------------------------
  // Open a paint program, press "Draw" to lock the left mouse button, and turn.
  {"Draw", "Draw",
   wheelMouseX("< X >", 1000),
   wheelMouseY("^ Y v", -1000),
   {
     {"Esc", key(KEY_ESC)}, {"LClk", mouseBtn(MOUSE_LEFT)}, {"MClk", mouseBtn(MOUSE_MIDDLE)}, {"RClk", mouseBtn(MOUSE_RIGHT)}, {"Undo", ctrl('z')},
     {"Draw", mouseLatch(MOUSE_LEFT)}, {"Ctrl", mod(M_CTRL)}, {"Shft", mod(M_SHIFT)}, {"Alt", mod(M_ALT)}, {"Del", key(KEY_DELETE)},
   }},

  // ---- 7: Numbers ----------------------------------------------------------
  {"Numbers", "Num",
   wheelSteps("Up/Down", 24, key(KEY_UP_ARROW), key(KEY_DOWN_ARROW)),
   wheelSteps("Lft/Rgt", 24, key(KEY_LEFT_ARROW), key(KEY_RIGHT_ARROW)),
   {
     {"1", key('1')}, {"2", key('2')}, {"3", key('3')}, {"4", key('4')}, {"5", key('5')},
     {"6", key('6')}, {"7", key('7')}, {"8", key('8')}, {"9", key('9')}, {"0", key('0')},
   }},
};

constexpr uint8_t NUM_LAYERS = sizeof(LAYERS) / sizeof(LAYERS[0]);
