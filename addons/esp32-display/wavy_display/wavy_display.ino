// Wavy ESP32 display: shows what Wavy plays on a 128x64 OLED and controls it with three buttons.
// Needs the "ESP32 display" addon running in Wavy on your PC (it relays Wavy over your home Wi-Fi).
//
// Libraries (Arduino IDE → Tools → Manage Libraries): U8g2 by oliver.
// Board: any ESP32 dev board (Tools → Board → esp32 → "ESP32 Dev Module").
//
// Wiring (see README.md):
//   OLED  VCC → 3V3, GND → GND, SDA → GPIO 21, SCL → GPIO 22
//   Buttons between the pin and GND (no resistors needed): PREV → GPIO 32, PLAY → GPIO 33, NEXT → GPIO 25
//
// Buttons: short press = previous / play-pause / next.
//          hold PLAY = like, hold PREV / NEXT = volume down / up (repeats while held).

#include <WiFi.h>
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

const int PIN_PREV = 32, PIN_PLAY = 33, PIN_NEXT = 25;

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

// ---------- buttons ----------
struct Button {
  int pin;
  const char* tap;      // command on a short press
  const char* hold;     // command when held
  bool repeat;          // repeat the hold command while held
  bool down = false, held = false;
  unsigned long since = 0, lastRepeat = 0;
};
Button buttons[] = {
  {PIN_PREV, "prev", "vol-", true},
  {PIN_PLAY, "toggle", "like", false},
  {PIN_NEXT, "next", "vol+", true},
};
const unsigned long HOLD_MS = 600, REPEAT_MS = 250, DEBOUNCE_MS = 30;

void send(const char* cmd) {
  if (client.connected()) {
    client.print(cmd);
    client.print('\n');
  }
}

void pollButtons() {
  unsigned long now = millis();
  for (auto& b : buttons) {
    bool pressed = digitalRead(b.pin) == LOW;
    if (pressed && !b.down) {
      if (now - b.since < DEBOUNCE_MS) continue;
      b.down = true; b.held = false; b.since = now;
    } else if (!pressed && b.down) {
      if (now - b.since < DEBOUNCE_MS) continue;
      b.down = false;
      if (!b.held) send(b.tap);
      b.since = now;
    } else if (pressed && b.down) {
      if (!b.held && now - b.since >= HOLD_MS) {
        b.held = true; b.lastRepeat = now;
        send(b.hold);
      } else if (b.held && b.repeat && now - b.lastRepeat >= REPEAT_MS) {
        b.lastRepeat = now;
        send(b.hold);
      }
    }
  }
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
  if (line == "N") { loaded = false; return; }
  if (line.startsWith("E")) { status = "Wrong key!"; client.stop(); return; }
  String p[8];
  if (line[0] != 'S' || splitTabs(line, p, 8) < 8) return;
  int newVolume = p[3].toInt();
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
  if (client.connect(pcIp, PORT, 2000)) {
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
    else if (c != '\r' && rx.length() < 400) rx += c;
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

void draw() {
  oled.clearBuffer();
  oled.setFont(u8g2_font_6x13_t_cyrillic);
  if (!loaded) {
    String s = client.connected() ? "Nothing playing" : status;
    oled.drawUTF8((128 - oled.getUTF8Width(s.c_str())) / 2, 30, s.c_str());
    oled.drawUTF8((128 - oled.getUTF8Width("Wavy")) / 2, 50, "Wavy");
    oled.sendBuffer();
    return;
  }

  // Top bar: play/pause, like, volume.
  if (playing) oled.drawTriangle(0, 0, 0, 9, 7, 5);
  else { oled.drawBox(0, 1, 3, 8); oled.drawBox(5, 1, 3, 8); }
  if (liked) drawHeart(12, 1);
  if (millis() < volumeShownUntil) {
    String v = "Vol " + String(volume) + "%";
    oled.drawUTF8(127 - oled.getUTF8Width(v.c_str()), 10, v.c_str());
  }

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
  for (auto& b : buttons) pinMode(b.pin, INPUT_PULLUP);
  oled.begin();
  oled.enableUTF8Print();
  WiFi.mode(WIFI_STA);
  WiFi.setAutoReconnect(true);
  WiFi.begin(WIFI_SSID, WIFI_PASS);
  udp.begin(DISCOVERY_PORT);
}

unsigned long lastDraw = 0;
String lastStatus;

void loop() {
  pollButtons();
  keepConnected();
  readLines();
  if (status != lastStatus) { Serial.println(status); lastStatus = status; }
  if (millis() - lastDraw >= 40) {   // 25 frames per second, smooth enough for scrolling
    lastDraw = millis();
    draw();
  }
}
