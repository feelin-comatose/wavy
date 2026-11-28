// Wavy ESP32 display: shows what Wavy plays on a 128x64 OLED and controls it with four buttons and a volume knob.
// Needs the "ESP32 display" addon running in Wavy on your PC (it relays Wavy over your home Wi-Fi).
//
// Libraries (Arduino IDE → Tools → Manage Libraries): U8g2 by oliver.
// Boards: ESP32 ("ESP32 Dev Module"), ESP32-C3 ("ESP32C3 Dev Module", USB CDC On Boot: Enabled)
//         or ESP8266 ("NodeMCU 1.0"). Pins are picked automatically for each; see README.md.
//
// Wiring: OLED VCC → 3V3, GND → GND; each button between its pin and GND (no resistors needed).
//              SDA  SCL   PREV  PLAY  NEXT  SCREEN   KNOB CLK  DT  SW   BUZZER
//   ESP32      21   22    32    33    25    26       27        14  13   4
//   ESP32-C3   5    6     3     4     10    7        0         1   20   21
//   ESP8266    D2   D1    D5    D6    D7    (none)   (none)             (none)
// The volume knob is a rotary encoder module such as the KY-040 (its + pin to 3V3).
// The buzzer must be a passive one (it plays any note); its other pin goes to GND.
//
// Buttons: short press = previous / play-pause / next / switch screen.
//          hold PLAY = like, hold PREV / NEXT = volume down / up (repeats), hold SCREEN = back to "Now playing".
// Knob: turn = volume, press = mute / unmute (on "Up next": turn = pick a track, press = play it).
// Screens: Now playing, Up next (PREV/NEXT pick a track, PLAY plays it), Clock (PLAY sets the alarm:
// hour → minutes → on/off, PREV/NEXT or the knob change it) and, with a buzzer, Tunes (PLAY plays or stops;
// the last tune played is the alarm's). When the alarm rings, any button stops it and starts the music.
#ifdef ESP8266
#include <ESP8266WiFi.h>
#else
#include <WiFi.h>
#endif
#include <WiFiUdp.h>
#include <U8g2lib.h>
#include <Wire.h>

// ---------- your settings ----------
const char* WIFI_SSID = "your-wifi-name";      // 2.4 GHz network: the ESP32 can't join 5 GHz
const char* WIFI_PASS = "your-wifi-password";
const char* KEY       = "paste-key-here";      // from config.json in the addon folder (or addon.log)
const char* PC_HOST   = "";                    // leave empty to find the PC automatically, or e.g. "192.168.1.20"
const uint16_t PORT           = 3739;          // "port" in config.json
const uint16_t DISCOVERY_PORT = 3740;          // "discovery_port" in config.json

#ifdef ESP8266
const int PIN_PREV = 14, PIN_PLAY = 12, PIN_NEXT = 13;   // D5, D6, D7
const int PIN_VIEW = -1;                                  // no spare safe pin: no screen button
const int PIN_ENC_A = -1, PIN_ENC_B = -1, PIN_ENC_SW = -1;   // ...and no volume knob
const int PIN_BUZZER = -1;                                    // ...and no buzzer
#elif defined(CONFIG_IDF_TARGET_ESP32C3)
// ESP32-C3: OLED SDA → GPIO 5, SCL → GPIO 6. Avoids the boot pins 2, 8 and 9.
const int PIN_SDA = 5, PIN_SCL = 6;
const int PIN_PREV = 3, PIN_PLAY = 4, PIN_NEXT = 10, PIN_VIEW = 7;
const int PIN_ENC_A = 0, PIN_ENC_B = 1, PIN_ENC_SW = 20;    // volume knob: CLK, DT, SW
const int PIN_BUZZER = 21;                                    // passive buzzer
#else
const int PIN_PREV = 32, PIN_PLAY = 33, PIN_NEXT = 25, PIN_VIEW = 26;
const int PIN_ENC_A = 27, PIN_ENC_B = 14, PIN_ENC_SW = 13;  // volume knob: CLK, DT, SW
const int PIN_BUZZER = 4;                                     // passive buzzer
#endif

// Volume knob (a rotary encoder such as the KY-040). If one click moves the volume by two steps,
// set ENC_STEPS to 2; if turning right lowers the volume, set ENC_REVERSE to true.
const int ENC_STEPS = 4;          // encoder signal changes per click
const bool ENC_REVERSE = false;
const int VOLUME_STEP = 2;        // % per click

// For the clock screen. UK time; others e.g. "CET-1CEST,M3.5.0,M10.5.0/3" (Central Europe), "MSK-3" (Moscow).
const char* TIMEZONE = "GMT0BST,M3.5.0/1,M10.5.0";

// Pick your screen. Most 0.96" boards are SSD1306; most 1.3" boards are SH1106.
U8G2_SSD1306_128X64_NONAME_F_HW_I2C oled(U8G2_R0, U8X8_PIN_NONE);
// U8G2_SH1106_128X64_NONAME_F_HW_I2C oled(U8G2_R0, U8X8_PIN_NONE);
// -----------------------------------

WiFiClient client;
WiFiUDP udp;
IPAddress pcIp;
bool havePc = false;

// What Wavy is doing, as last reported by the PC.
bool loaded = false, playing = false, liked = false;
int volume = 0;
float position = 0, duration = 0;
String title, artist, status = "Starting...";
unsigned long receivedAt = 0;      // millis() when position was received
unsigned long lastLine = 0;        // for the link timeout
unsigned long volumeShownUntil = 0;
unsigned long titleSince = 0;      // when the title changed, for scrolling
unsigned long nextConnectTry = 0;
String rx;

// The next tracks in Wavy's queue (sent by the addon as a "Q" line).
struct QueueItem { int index; String title, artist; };
const int UP_NEXT_MAX = 20;
QueueItem upNext[UP_NEXT_MAX];
int upNextCount = 0;
int listTop = 0;                   // first row shown on the "Up next" screen (it shows three)

// Screens, switched with the fourth button. The tunes screen is there only with a buzzer.
enum View { NOW_PLAYING, UP_NEXT, CLOCK, TUNE_LIST };
const int VIEW_COUNT = PIN_BUZZER >= 0 ? 4 : 3;
int view = NOW_PLAYING;
int tuneSel = 0, tuneTop = 0;      // highlighted and first shown row on the tunes screen
int selected = 0;                  // highlighted row on the "Up next" screen
unsigned long lastPress = 0;       // "Up next" returns to "Now playing" after a while
unsigned long selectedSince = 0;   // for scrolling the highlighted row

// ---------- buttons ----------
enum Btn { B_PREV, B_PLAY, B_NEXT, B_VIEW, B_KNOB, BUTTON_COUNT };
struct Button {
  int pin;
  bool repeat;          // repeat the hold action while held
  bool down = false, held = false;
  unsigned long since = 0, lastRepeat = 0;
};
Button buttons[] = {{PIN_PREV, true}, {PIN_PLAY, false}, {PIN_NEXT, true}, {PIN_VIEW, false}, {PIN_ENC_SW, false}};
const unsigned long HOLD_MS = 600, REPEAT_MS = 250, DEBOUNCE_MS = 30;

void send(const String& cmd) {
  if (client.connected()) {
    client.print(cmd);
    client.print('\n');
  }
}

// ---------- tunes (ringtone format, RTTTL) ----------
// Any RTTTL ringtone string can be added here: name, then "d=" default note length, "o=" octave, "b=" tempo.
struct Tune { const char* name; const char* rtttl; };
const Tune TUNES[] = {
  {"Gran Vals (Nokia)", "Nokia:d=4,o=5,b=225:8e6,8d6,f#,g#,8c#6,8b,d,e,8b,8a,c#,e,2a"},
  {"Korobeiniki (Tetris)", "Korobeiniki:d=4,o=5,b=160:e6,8b,8c6,d6,8c6,8b,a,8a,8c6,e6,8d6,8c6,b,8b,8c6,d6,e6,c6,a,2a,8p,d6,8f6,a6,8g6,8f6,e6,8e6,8c6,e6,8d6,8c6,b,8b,8c6,d6,e6,c6,a,2a"},
  {"Fur Elise", "FurElise:d=8,o=5,b=125:e6,d#6,e6,d#6,e6,b,d6,c6,4a,p,c,e,a,4b,p,e,g#,b,4c6,p,e,e6,d#6,e6,d#6,e6,b,d6,c6,4a,p,c,e,a,4b,p,e,c6,b,2a"},
  {"Ode to Joy", "OdeToJoy:d=4,o=5,b=140:e,e,f,g,g,f,e,d,c,c,d,e,e.,8d,2d,e,e,f,g,g,f,e,d,c,c,d,e,d.,8c,2c"},
  {"Mountain King", "MountainKing:d=8,o=5,b=180:a4,b4,c,d,e,c,4e,d#,b4,4d#,d,a#4,4d,a4,b4,c,d,e,c,e,a,g,e,c,e,2g"},
  {"Happy Birthday", "Birthday:d=4,o=5,b=125:8g.,16g,a,g,c6,2b,8g.,16g,a,g,d6,2c6,8g.,16g,g6,e6,c6,b,a,8f6.,16f6,e6,c6,d6,2c6"},
};
const int TUNE_COUNT = sizeof(TUNES) / sizeof(TUNES[0]);

int playingTune = -1;              // index of the tune playing, or -1
bool tuneRepeat = false;
const char* tuneStart = nullptr;   // first note
const char* tunePos = nullptr;     // next note
int tuneDur = 4, tuneOct = 6, tuneBpm = 63;
unsigned long noteEnd = 0, noteOffAt = 0;

void stopTune() {
  if (PIN_BUZZER >= 0) noTone(PIN_BUZZER);
  playingTune = -1;
  tunePos = nullptr;
  noteOffAt = 0;
}

void playTune(int i, bool repeat) {
  if (PIN_BUZZER < 0) return;
  stopTune();
  const char* p = strchr(TUNES[i].rtttl, ':');
  if (!p) return;
  p++;
  tuneDur = 4; tuneOct = 6; tuneBpm = 63;
  while (*p && *p != ':') {                    // defaults: d=4,o=5,b=140
    char key = *p;
    while (*p && *p != '=' && *p != ':') p++;
    if (*p == '=') p++;
    int v = atoi(p);
    if (key == 'd' && v > 0) tuneDur = v;
    if (key == 'o' && v > 0) tuneOct = v;
    if (key == 'b' && v > 0) tuneBpm = v;
    while (*p && *p != ',' && *p != ':') p++;
    if (*p == ',') p++;
  }
  if (*p != ':') return;
  tuneStart = tunePos = p + 1;
  playingTune = i;
  tuneRepeat = repeat;
  noteEnd = millis();
}

// Plays the tune note by note without blocking: the screen and buttons keep working.
void pollTune() {
  if (!tunePos) return;
  unsigned long now = millis();
  if (noteOffAt && now >= noteOffAt) { noTone(PIN_BUZZER); noteOffAt = 0; }   // short gap between notes
  if (now < noteEnd) return;
  if (!*tunePos) {
    if (!tuneRepeat) { stopTune(); return; }
    tunePos = tuneStart;
    noteEnd = now + 1000;                      // a pause before playing it again
    return;
  }
  int dur = 0;
  while (isdigit(*tunePos)) dur = dur * 10 + (*tunePos++ - '0');
  if (dur <= 0) dur = tuneDur;
  int semitone = -1;                           // from C; -1 = rest
  switch (tolower(*tunePos)) {
    case 'c': semitone = 0; break;
    case 'd': semitone = 2; break;
    case 'e': semitone = 4; break;
    case 'f': semitone = 5; break;
    case 'g': semitone = 7; break;
    case 'a': semitone = 9; break;
    case 'b': case 'h': semitone = 11; break;
  }
  if (*tunePos) tunePos++;
  if (*tunePos == '#') { semitone++; tunePos++; }
  bool dotted = false;
  if (*tunePos == '.') { dotted = true; tunePos++; }
  int octave = tuneOct;
  if (isdigit(*tunePos)) octave = *tunePos++ - '0';
  if (*tunePos == '.') { dotted = true; tunePos++; }
  while (*tunePos && *tunePos != ',') tunePos++;   // skip anything unexpected
  if (*tunePos == ',') tunePos++;

  unsigned long ms = 240000UL / tuneBpm / dur;     // a whole note is four beats
  if (dotted) ms += ms / 2;
  if (semitone >= 0) {
    float freq = 440.0f * powf(2.0f, (octave - 4) + (semitone - 9) / 12.0f);   // a4 = 440 Hz
    tone(PIN_BUZZER, (unsigned int)freq);
    noteOffAt = now + ms * 9 / 10;
  } else {
    noTone(PIN_BUZZER);
    noteOffAt = 0;
  }
  noteEnd = now + ms;
}

// ---------- alarm clock ----------
// Set on the clock screen: button 2 (or the knob) steps through hour → minutes → on/off,
// buttons 1/3 (or turning the knob) change the value. When it rings, any button stops it and starts the music.
enum AlarmEdit { EDIT_NONE, EDIT_HOUR, EDIT_MIN, EDIT_ON };
int alarmEdit = EDIT_NONE;
bool alarmOn = false;
int alarmHour = 7, alarmMin = 0, alarmTune = 0;   // alarmTune: the last tune played on the tunes screen
bool ringing = false;
unsigned long ringingSince = 0;
int alarmDoneDay = -1;             // day of the year it last rang, so it rings once a day

#ifndef ESP8266
#include <Preferences.h>
Preferences prefs;
#endif

void loadAlarm() {
#ifndef ESP8266
  prefs.begin("wavy", true);
  alarmOn = prefs.getBool("alarmOn", false);
  alarmHour = prefs.getInt("alarmH", 7);
  alarmMin = prefs.getInt("alarmM", 0);
  alarmTune = constrain(prefs.getInt("alarmTune", 0), 0, TUNE_COUNT - 1);
  prefs.end();
#endif
}

void saveAlarm() {
#ifndef ESP8266
  prefs.begin("wavy", false);
  prefs.putBool("alarmOn", alarmOn);
  prefs.putInt("alarmH", alarmHour);
  prefs.putInt("alarmM", alarmMin);
  prefs.putInt("alarmTune", alarmTune);
  prefs.end();
#endif
}

void adjustAlarm(int by) {
  if (alarmEdit == EDIT_HOUR) alarmHour = ((alarmHour + by) % 24 + 24) % 24;
  if (alarmEdit == EDIT_MIN) alarmMin = ((alarmMin + by) % 60 + 60) % 60;
  if (alarmEdit == EDIT_ON && by % 2) alarmOn = !alarmOn;
}

void finishAlarmEdit() {
  if (alarmEdit == EDIT_NONE) return;
  alarmEdit = EDIT_NONE;
  saveAlarm();
}

void stopRinging() {
  ringing = false;
  stopTune();
  send("play");                    // wake up to your music
}

void checkAlarm() {
  if (ringing) {
    if (millis() - ringingSince > 120000) { ringing = false; stopTune(); }   // gives up after 2 minutes
    return;
  }
  if (!alarmOn || alarmEdit != EDIT_NONE) return;
  time_t now = time(nullptr);
  if (now < 1700000000) return;    // the clock isn't set yet
  struct tm t;
  localtime_r(&now, &t);
  if (t.tm_hour == alarmHour && t.tm_min == alarmMin && t.tm_yday != alarmDoneDay) {
    alarmDoneDay = t.tm_yday;
    ringing = true;
    ringingSince = millis();
    view = CLOCK;
    if (PIN_BUZZER >= 0) playTune(alarmTune, true);
    else send("play");             // no buzzer: just start the music
  }
}

void onTap(int b) {
  if (ringing) { stopRinging(); return; }
  if (b == B_VIEW) {
    finishAlarmEdit();
    view = (view + 1) % VIEW_COUNT;
    selected = 0;
    selectedSince = millis();
    return;
  }
  if (view == CLOCK && (b == B_PLAY || b == B_KNOB)) {   // next step of setting the alarm
    alarmEdit = (alarmEdit + 1) % 4;
    if (alarmEdit == EDIT_NONE) saveAlarm();
    return;
  }
  if (view == CLOCK && alarmEdit != EDIT_NONE) {
    adjustAlarm(b == B_NEXT ? 1 : -1);
    return;
  }
  if (view == TUNE_LIST) {                       // pick a tune, play or stop it
    if (b == B_PREV) tuneSel = (tuneSel + TUNE_COUNT - 1) % TUNE_COUNT;
    if (b == B_NEXT) tuneSel = (tuneSel + 1) % TUNE_COUNT;
    if (b == B_PLAY || b == B_KNOB) {
      if (playingTune == tuneSel) stopTune();
      else {
        playTune(tuneSel, false);
        if (alarmTune != tuneSel) { alarmTune = tuneSel; saveAlarm(); }
      }
    }
    selectedSince = millis();
    return;
  }
  if (view == UP_NEXT && upNextCount > 0) {   // on "Up next": move the highlight, play the highlighted track
    if (b == B_PREV) selected = (selected + upNextCount - 1) % upNextCount;
    if (b == B_NEXT) selected = (selected + 1) % upNextCount;
    if (b == B_PLAY || b == B_KNOB) { send("jump " + String(upNext[selected].index)); view = NOW_PLAYING; }
    selectedSince = millis();
    return;
  }
  if (b == B_KNOB) { send("mute"); return; }
  send(b == B_PREV ? "prev" : b == B_PLAY ? "toggle" : "next");
}

// ---------- volume knob ----------
volatile int encRaw = 0;          // signal changes counted by the interrupt
volatile uint8_t encState = 0;
int encLeft = 0;                  // changes not yet making up a whole click
int volumePending = 0;            // % not yet sent to the PC
unsigned long volumeSentAt = 0;
unsigned long volumeLocalUntil = 0;   // ignore the PC's (older) volume while the knob is turning

void IRAM_ATTR encoderIsr() {
  // Quadrature decoding: previous and current A/B state → -1, 0 or +1.
  static const int8_t table[16] = {0, -1, 1, 0, 1, 0, 0, -1, -1, 0, 0, 1, 0, 1, -1, 0};
  encState = ((encState << 2) | (digitalRead(PIN_ENC_A) << 1) | digitalRead(PIN_ENC_B)) & 0x0F;
  encRaw += table[encState];
}

void pollEncoder() {
  if (PIN_ENC_A < 0) return;
  noInterrupts();
  int raw = encRaw;
  encRaw = 0;
  interrupts();
  encLeft += raw;
  int clicks = encLeft / ENC_STEPS;
  encLeft -= clicks * ENC_STEPS;
  if (ENC_REVERSE) clicks = -clicks;
  unsigned long now = millis();
  if (clicks) {
    lastPress = now;
    if (ringing) {
      stopRinging();
    } else if (view == CLOCK && alarmEdit != EDIT_NONE) {   // setting the alarm
      adjustAlarm(clicks);
    } else if (view == TUNE_LIST) {
      tuneSel = constrain(tuneSel + clicks, 0, TUNE_COUNT - 1);
      selectedSince = now;
    } else if (view == UP_NEXT && upNextCount > 0) {   // on "Up next" the knob scrolls the list
      selected = constrain(selected + clicks, 0, upNextCount - 1);
      selectedSince = now;
    } else {
      int target = constrain(volume + clicks * VOLUME_STEP, 0, 100);
      volumePending += target - volume;
      volume = target;                          // show the new volume right away
      volumeShownUntil = now + 1500;
      volumeLocalUntil = now + 800;
    }
  }
  if (volumePending && now - volumeSentAt >= 100) {   // at most ten updates a second
    send("vol " + String(volumePending));
    volumePending = 0;
    volumeSentAt = now;
  }
}

void onHold(int b) {
  if (ringing) { stopRinging(); return; }
  if (view == CLOCK && alarmEdit != EDIT_NONE && (b == B_PREV || b == B_NEXT)) {   // hold to change it quickly
    adjustAlarm(b == B_NEXT ? 1 : -1);
    return;
  }
  if (b == B_PREV) send("vol-");
  if (b == B_NEXT) send("vol+");
  if (b == B_PLAY) send("like");
  if (b == B_VIEW) { finishAlarmEdit(); view = NOW_PLAYING; }   // hold the screen button: back to "Now playing"
}

void pollButtons() {
  unsigned long now = millis();
  for (int i = 0; i < BUTTON_COUNT; i++) {
    Button& b = buttons[i];
    if (b.pin < 0) continue;
    bool pressed = digitalRead(b.pin) == LOW;
    if (pressed && !b.down) {
      if (now - b.since < DEBOUNCE_MS) continue;
      b.down = true; b.held = false; b.since = now;
      lastPress = now;
    } else if (!pressed && b.down) {
      if (now - b.since < DEBOUNCE_MS) continue;
      b.down = false;
      if (!b.held) onTap(i);
      b.since = now;
    } else if (pressed && b.down) {
      if (!b.held && now - b.since >= HOLD_MS) {
        b.held = true; b.lastRepeat = now;
        onHold(i);
      } else if (b.held && b.repeat && now - b.lastRepeat >= REPEAT_MS) {
        b.lastRepeat = now;
        onHold(i);
      }
    }
  }
  if (view == UP_NEXT && now - lastPress > 20000) view = NOW_PLAYING;
  if (alarmEdit != EDIT_NONE && now - lastPress > 20000) finishAlarmEdit();   // forgot to finish: keep what was set
}

// ---------- link to the PC ----------
// Splits s at tabs into parts; returns how many were found.
int splitTabs(const String& s, String* parts, int max) {
  int n = 0, start = 0;
  while (n < max) {
    int tab = s.indexOf('\t', start);
    if (tab < 0) { parts[n++] = s.substring(start); break; }
    parts[n++] = s.substring(start, tab);
    start = tab + 1;
  }
  return n;
}

void handleLine(const String& line) {
  lastLine = millis();
  if (line == "N") { loaded = false; upNextCount = 0; return; }
  if (line.startsWith("E")) { status = "Wrong key!"; client.stop(); return; }
  if (line[0] == 'Q') {
    static String q[1 + 3 * UP_NEXT_MAX];
    int n = (splitTabs(line, q, 1 + 3 * UP_NEXT_MAX) - 1) / 3;
    for (int i = 0; i < n; i++) {
      QueueItem item = {q[1 + i * 3].toInt(), q[2 + i * 3], q[3 + i * 3]};
      upNext[i] = item;
    }
    upNextCount = n;
    if (selected >= n) selected = n > 0 ? n - 1 : 0;
    return;
  }
  String p[8];
  if (line[0] != 'S' || splitTabs(line, p, 8) < 8) return;
  int newVolume = millis() < volumeLocalUntil ? volume : p[3].toInt();
  if (loaded && newVolume != volume) volumeShownUntil = millis() + 1500;
  if (p[6] != title || p[7] != artist) titleSince = millis();
  loaded = true;
  playing = p[1] == "1";
  liked = p[2] == "1";
  volume = newVolume;
  position = p[4].toFloat();
  duration = p[5].toFloat();
  title = p[6];
  artist = p[7];
  receivedAt = millis();
}

void findPc() {
  if (PC_HOST[0]) {
    havePc = pcIp.fromString(PC_HOST) || WiFi.hostByName(PC_HOST, pcIp);
    return;
  }
  // The addon announces "WAVY-ESP32 <port>" every 2 s.
  int size = udp.parsePacket();
  if (size <= 0) return;
  char buf[32] = {0};
  udp.read(buf, sizeof(buf) - 1);
  if (strncmp(buf, "WAVY-ESP32", 10) == 0) {
    pcIp = udp.remoteIP();
    havePc = true;
  }
}

void keepConnected() {
  if (WiFi.status() != WL_CONNECTED) {
    status = "Connecting to Wi-Fi...";
    loaded = false;
    return;
  }
  if (client.connected()) {
    if (millis() - lastLine > 15000) {   // the PC sends at least every 5 s
      client.stop();
      havePc = PC_HOST[0];               // look for the PC again (its address may have changed)
    }
    return;
  }
  loaded = false;
  if (millis() < nextConnectTry) return;
  if (!havePc) {
    status = "Looking for Wavy...";
    findPc();
    if (!havePc && PC_HOST[0]) nextConnectTry = millis() + 3000;   // a name lookup blocks: don't retry every loop
    if (!havePc) return;
  }
  nextConnectTry = millis() + 3000;
  status = "Connecting to PC...";
#ifdef ESP8266
  client.setTimeout(2000);
  if (client.connect(pcIp, PORT)) {
#else
  if (client.connect(pcIp, PORT, 2000)) {
#endif
    client.setNoDelay(true);
    client.print("key ");
    client.print(KEY);
    client.print('\n');
    lastLine = millis();
    rx = "";
    status = "Waiting for Wavy...";
  } else if (!PC_HOST[0]) {
    havePc = false;                      // wait for the next announcement
  }
}

void readLines() {
  while (client.available()) {
    char c = client.read();
    if (c == '\n') { handleLine(rx); rx = ""; }
    else if (c != '\r' && rx.length() < 2000) rx += c;   // the queue line can be long
  }
}

// ---------- drawing ----------
String mmss(float s) {
  if (s < 0) s = 0;
  int t = (int)s;
  char buf[12];
  snprintf(buf, sizeof(buf), "%d:%02d", t / 60, t % 60);
  return String(buf);
}

// Draws text in the band top..bottom; scrolls it if it's wider than the screen.
void drawScrolling(const String& text, int baseline, int top, int bottom) {
  int w = oled.getUTF8Width(text.c_str());
  if (w <= 128) {
    oled.drawUTF8((128 - w) / 2, baseline, text.c_str());
    return;
  }
  const int gap = 30;
  unsigned long t = millis() - titleSince;
  int offset = t < 1500 ? 0 : ((t - 1500) / 30) % (w + gap);   // pause at the start, then ~33 px/s
  oled.setClipWindow(0, top, 127, bottom);
  oled.drawUTF8(-offset, baseline, text.c_str());
  oled.drawUTF8(-offset + w + gap, baseline, text.c_str());
  oled.setMaxClipWindow();
}

void drawHeart(int x, int y) {   // 9x8, top-left at x,y
  oled.drawDisc(x + 2, y + 2, 2);
  oled.drawDisc(x + 6, y + 2, 2);
  oled.drawTriangle(x, y + 3, x + 9, y + 3, x + 4, y + 8);
}

// The eye-catcher in the top-right corner of every screen: a tiny equalizer that dances
// while music plays and slowly breathes otherwise.
float eqLevel[4] = {1, 1, 1, 1}, eqTarget[4] = {1, 1, 1, 1};
unsigned long eqNext = 0;

void drawEq() {
  unsigned long now = millis();
  bool dancing = (loaded && playing) || playingTune >= 0;
  if (dancing && now >= eqNext) {
    eqNext = now + 110;
    for (int i = 0; i < 4; i++) eqTarget[i] = 2 + random(9);                 // 2..10 px
  }
  for (int i = 0; i < 4; i++) {
    float target = dancing ? eqTarget[i] : 2.5f + 1.5f * sinf(now / 500.0f + i * 0.9f);   // 1..4 px
    eqLevel[i] += (target - eqLevel[i]) * 0.35f;
    int h = constrain((int)(eqLevel[i] + 0.5f), 1, 10);
    oled.drawBox(116 + i * 3, 10 - h, 2, h);
  }
}

// Shared top bar: which screen you're on (dots), the volume after a change, and the equalizer.
void drawTopBar() {
  oled.setFont(u8g2_font_6x13_t_cyrillic);
  if (millis() < volumeShownUntil) {
    String v = "Vol " + String(volume) + "%";
    oled.drawUTF8(113 - oled.getUTF8Width(v.c_str()), 10, v.c_str());
  } else if (PIN_VIEW >= 0) {
    for (int i = 0; i < VIEW_COUNT; i++) {
      if (i == view) oled.drawBox(86 + i * 7, 3, 4, 4);
      else oled.drawFrame(86 + i * 7, 3, 4, 4);
    }
  }
  drawEq();
}

// One row of the "Up next" list; the highlighted row is inverted and scrolls if too long.
void drawRow(const String& text, int top, bool highlighted, int width) {
  if (highlighted) { oled.drawBox(0, top, width, 15); oled.setDrawColor(0); }
  oled.setClipWindow(0, top, width - 1, top + 14);
  int w = oled.getUTF8Width(text.c_str());
  int offset = 0;
  if (highlighted && w > width - 4) {
    unsigned long t = millis() - selectedSince;
    offset = t < 1000 ? 0 : ((t - 1000) / 30) % (w + 30);
    oled.drawUTF8(2 - offset + w + 30, top + 12, text.c_str());
  }
  oled.drawUTF8(2 - offset, top + 12, text.c_str());
  oled.setMaxClipWindow();
  oled.setDrawColor(1);
}

// A list with a heading ("Name 2/15"), three rows at a time, and a scrollbar when it's longer.
void drawList(const char* name, int count, int sel, int& top, String (*rowText)(int)) {
  String head = String(name) + " " + String(sel + 1) + "/" + String(count);
  oled.drawUTF8(0, 10, head.c_str());

  // The window follows the highlight, keeping one row of context where it can.
  const int ROWS = 3;
  if (sel < top + 1) top = sel - 1;
  if (sel > top + ROWS - 2) top = sel - ROWS + 2;
  top = constrain(top, 0, max(0, count - ROWS));

  bool scrollbar = count > ROWS;
  int width = scrollbar ? 124 : 128;
  for (int r = 0; r < ROWS && top + r < count; r++) drawRow(rowText(top + r), 15 + r * 16, top + r == sel, width);
  if (scrollbar) {   // thin track on the right, with a thumb showing where the three rows are
    const int y0 = 15, height = 47;
    oled.drawVLine(126, y0, height);
    int thumb = max(5, height * ROWS / count);
    oled.drawBox(125, y0 + (height - thumb) * top / (count - ROWS), 3, thumb);
  }
}

String upNextRow(int i) {
  return upNext[i].artist.length() ? upNext[i].title + " - " + upNext[i].artist : upNext[i].title;
}

String tuneRow(int i) {
  String mark = i == playingTune ? "> " : "";   // the one playing now
  return mark + TUNES[i].name;
}

void drawUpNext() {
  if (upNextCount == 0) {
    oled.drawUTF8(0, 10, "Up next");
    oled.drawUTF8((128 - oled.getUTF8Width("Queue is empty")) / 2, 40, "Queue is empty");
    return;
  }
  drawList("Up next", upNextCount, selected, listTop, upNextRow);
}

void drawBell(int x, int y) {   // 9x9, top-left at x,y: shown on the clock while the alarm is on
  oled.drawDisc(x + 4, y + 4, 3, U8G2_DRAW_UPPER_LEFT | U8G2_DRAW_UPPER_RIGHT);
  oled.drawBox(x + 1, y + 4, 7, 3);
  oled.drawHLine(x, y + 7, 9);
  oled.drawPixel(x + 4, y + 8);
}

void drawClock() {
  time_t now = time(nullptr);
  struct tm t;
  localtime_r(&now, &t);
  char hh[4] = "--", mm[4] = "--", date[20] = "";
  bool synced = now > 1700000000;              // the clock has been set from the internet
  bool editing = alarmEdit != EDIT_NONE;
  bool blinkOff = (millis() / 300) % 2;        // the value being set blinks
  if (editing) {
    snprintf(hh, sizeof(hh), "%02d", alarmHour);
    snprintf(mm, sizeof(mm), "%02d", alarmMin);
  } else if (synced) {
    strftime(hh, sizeof(hh), "%H", &t);
    strftime(mm, sizeof(mm), "%M", &t);
    strftime(date, sizeof(date), "%a %d %b", &t);
  }
  oled.drawUTF8(0, 10, editing ? "Set alarm" : date);
  if (alarmOn && !editing) drawBell(64, 1);
  // Hours, colon and minutes at fixed positions, so the blinking colon never moves the digits.
  oled.setFont(u8g2_font_logisoso28_tn);
  int x = (128 - oled.getStrWidth("00:00")) / 2;
  int colonX = x + oled.getStrWidth("00");
  if (!(alarmEdit == EDIT_HOUR && blinkOff)) oled.drawStr(x, 46, hh);
  if (editing || !synced || now % 2) oled.drawStr(colonX, 46, ":");
  if (!(alarmEdit == EDIT_MIN && blinkOff)) oled.drawStr(colonX + oled.getStrWidth(":"), 46, mm);
  oled.setFont(u8g2_font_6x13_t_cyrillic);
  const char* line = nullptr;
  if (ringing) line = "Press any button";
  else if (alarmEdit == EDIT_HOUR) line = "Set the hour";
  else if (alarmEdit == EDIT_MIN) line = "Set the minutes";
  else if (alarmEdit == EDIT_ON) line = blinkOff ? "" : (alarmOn ? "Alarm ON" : "Alarm OFF");
  if (line) oled.drawUTF8((128 - oled.getUTF8Width(line)) / 2, 62, line);
  else if (loaded) drawScrolling(artist.length() ? title + " - " + artist : title, 62, 51, 63);
  else {
    String s = client.connected() ? "Nothing playing" : status;
    oled.drawUTF8((128 - oled.getUTF8Width(s.c_str())) / 2, 62, s.c_str());
  }
}

void draw() {
  oled.clearBuffer();
  oled.setFont(u8g2_font_6x13_t_cyrillic);
  if (view == CLOCK) {
    drawClock();
    drawTopBar();
    if (ringing && (millis() / 500) % 2) {   // ringing: the whole screen flashes
      oled.setDrawColor(2);
      oled.drawBox(0, 0, 128, 64);
      oled.setDrawColor(1);
    }
    oled.sendBuffer();
    return;
  }
  if (view == TUNE_LIST) {
    drawList("Tunes", TUNE_COUNT, tuneSel, tuneTop, tuneRow);
    drawTopBar();
    oled.sendBuffer();
    return;
  }
  if (!loaded) {
    String s = client.connected() ? "Nothing playing" : status;
    oled.drawUTF8((128 - oled.getUTF8Width(s.c_str())) / 2, 30, s.c_str());
    oled.drawUTF8((128 - oled.getUTF8Width("Wavy")) / 2, 50, "Wavy");
    drawTopBar();
    oled.sendBuffer();
    return;
  }
  if (view == UP_NEXT) {
    drawUpNext();
    drawTopBar();
    oled.sendBuffer();
    return;
  }

  // Now playing. Top left: play/pause and like.
  if (playing) oled.drawTriangle(0, 0, 0, 9, 7, 5);
  else { oled.drawBox(0, 1, 3, 8); oled.drawBox(5, 1, 3, 8); }
  if (liked) drawHeart(12, 1);
  drawTopBar();

  float pos = position + (playing ? (millis() - receivedAt) / 1000.0 : 0);
  if (duration > 0 && pos > duration) pos = duration;

  oled.setFont(u8g2_font_9x15_t_cyrillic);
  drawScrolling(title, 28, 15, 31);
  oled.setFont(u8g2_font_6x13_t_cyrillic);
  drawScrolling(artist, 43, 33, 45);

  // Progress bar and times (radio has no duration: only the elapsed time).
  if (duration > 0) {
    oled.drawFrame(0, 48, 128, 4);
    oled.drawBox(0, 48, (int)(128 * pos / duration), 4);
    String d = mmss(duration);
    oled.drawUTF8(127 - oled.getUTF8Width(d.c_str()), 63, d.c_str());
  }
  oled.drawUTF8(0, 63, mmss(pos).c_str());
  oled.sendBuffer();
}

// ---------- main ----------
void setup() {
  Serial.begin(115200);
  for (auto& b : buttons) if (b.pin >= 0) pinMode(b.pin, INPUT_PULLUP);
  loadAlarm();
  if (PIN_ENC_A >= 0) {
    pinMode(PIN_ENC_A, INPUT_PULLUP);
    pinMode(PIN_ENC_B, INPUT_PULLUP);
    attachInterrupt(digitalPinToInterrupt(PIN_ENC_A), encoderIsr, CHANGE);
    attachInterrupt(digitalPinToInterrupt(PIN_ENC_B), encoderIsr, CHANGE);
  }
#if defined(CONFIG_IDF_TARGET_ESP32C3)
  Wire.begin(PIN_SDA, PIN_SCL);
#endif
  oled.begin();
  oled.enableUTF8Print();
  oled.setFontMode(1);   // transparent text, so it can be drawn inverted on the highlighted row
  WiFi.mode(WIFI_STA);
  WiFi.setAutoReconnect(true);
  WiFi.begin(WIFI_SSID, WIFI_PASS);
#ifdef ESP8266
  configTime(TIMEZONE, "pool.ntp.org");
#else
  configTzTime(TIMEZONE, "pool.ntp.org");
#endif
  udp.begin(DISCOVERY_PORT);
}

unsigned long lastDraw = 0;
String lastStatus;

void loop() {
  pollButtons();
  pollEncoder();
  pollTune();
  checkAlarm();
  keepConnected();
  readLines();
  if (status != lastStatus) { Serial.println(status); lastStatus = status; }
  if (millis() - lastDraw >= 40) {   // 25 frames per second, smooth enough for scrolling
    lastDraw = millis();
    draw();
  }
}
